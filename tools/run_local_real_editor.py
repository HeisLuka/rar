#!/usr/bin/env python3
"""Prepare and run the proven real-PUB local browser editor surface."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import venv

ROOT = pathlib.Path(__file__).resolve().parents[1]
STATE = ROOT / ".chaptera-local" / "editor"
VENV = STATE / "venv"
FIXTURE = STATE / "SampleNewsletter.pub"
GRAPH = STATE / "resolved-graph.json"
WORK = STATE / "work"
API_PORT = 18765
WEB_PORT = 18083
API = f"http://127.0.0.1:{API_PORT}"
EDITOR_URL = (
    f"http://127.0.0.1:{WEB_PORT}/apps/web/local-editor.html"
    f"?api={urllib.parse.quote(API, safe='')}&emu_per_css_px=12700&pan_y_css_px=-2600"
)
FIXTURE_URL = (
    "https://raw.githubusercontent.com/apache/poi/"
    "942d95d85b15d0dfdb3bc9ba1b4f273f277757c8/"
    "test-data/publisher/SampleNewsletter.pub"
)
FIXTURE_SHA = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
GRAPH_SHA = "7c327cd729fc2f9760e59cce1dd7c104c55162f4c3032c132068cfee513b1c4d"


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def python_in_venv() -> pathlib.Path:
    return VENV / ("Scripts/python.exe" if os.name == "nt" else "bin/python")


def run_checked(args: list[str], *, stdout=None) -> None:
    proc = subprocess.run(args, cwd=ROOT, stdout=stdout, text=stdout is None)
    if proc.returncode != 0:
        raise RuntimeError(f"command failed ({proc.returncode}): {' '.join(args)}")


def ensure_venv() -> pathlib.Path:
    python = python_in_venv()
    if not python.exists():
        venv.EnvBuilder(with_pip=True).create(VENV)
    check = subprocess.run(
        [str(python), "-c", "import jsonschema; assert jsonschema.__version__"],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if check.returncode != 0:
        run_checked(
            [
                str(python),
                "-m",
                "pip",
                "install",
                "--disable-pip-version-check",
                "--no-input",
                "jsonschema==4.23.0",
            ]
        )
    return python


def ensure_fixture() -> None:
    if FIXTURE.exists() and FIXTURE.stat().st_size == 291840 and sha256(FIXTURE) == FIXTURE_SHA:
        return
    FIXTURE.unlink(missing_ok=True)
    with urllib.request.urlopen(FIXTURE_URL, timeout=60) as response:
        data = response.read(292000)
    if len(data) != 291840 or hashlib.sha256(data).hexdigest() != FIXTURE_SHA:
        raise RuntimeError("downloaded SampleNewsletter.pub identity mismatch")
    FIXTURE.write_bytes(data)


def producer_path() -> pathlib.Path:
    name = "chaptera-producer-a.exe" if os.name == "nt" else "chaptera-producer-a"
    return ROOT / "vendor/producer-a/target/debug" / name


def build_inputs() -> pathlib.Path:
    ensure_fixture()
    run_checked(
        [
            "cargo",
            "build",
            "--manifest-path",
            "vendor/producer-a/Cargo.toml",
            "-p",
            "chaptera-producer-a",
        ]
    )
    producer = producer_path()
    with GRAPH.open("wb") as output:
        proc = subprocess.run(
            [str(producer), "resolved-graph", str(FIXTURE)],
            cwd=ROOT,
            stdout=output,
        )
    if proc.returncode != 0:
        raise RuntimeError("resolved-graph producer failed")
    if sha256(GRAPH) != GRAPH_SHA:
        raise RuntimeError("resolved graph identity mismatch")
    return producer


def wait_http(url: str, process: subprocess.Popen, timeout: float = 20) -> None:
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"process exited before {url} became ready: {process.returncode}")
        try:
            with urllib.request.urlopen(url, timeout=1) as response:
                if response.status == 200:
                    return
                last = response.status
        except (OSError, urllib.error.HTTPError) as error:
            last = error
        time.sleep(0.2)
    raise RuntimeError(f"{url} did not become ready: {last}")


def json_request(path: str, *, method: str = "GET", body=None):
    headers = {"x-chaptera-principal-id": "synthetic-editor"}
    data = None
    if body is not None:
        data = json.dumps(body, separators=(",", ":")).encode("utf-8")
        headers["content-type"] = "application/json"
    request = urllib.request.Request(
        API + path,
        data=data,
        method=method,
        headers=headers,
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.load(response)


def interactive_smoke() -> None:
    receipt = json.loads(
        (
            ROOT
            / "packages/protocol/revision/v1/producer-receipts/sample-newsletter.real.json"
        ).read_text(encoding="utf-8")
    )
    request = copy.deepcopy(receipt["request"])
    request["client_operation_id"] = "local-interactive-smoke-move"
    request["command"]["x_emu"] = (
        receipt["accepted"]["canonical_operation"]["before"]["x"] + 38100
    )
    request["command"]["y_emu"] = (
        receipt["accepted"]["canonical_operation"]["before"]["y"] + 50800
    )
    accepted = json_request("/v1/commit", method="POST", body=request)
    if accepted.get("protocol_version") != "chaptera.commit-accepted.v1":
        raise RuntimeError(f"interactive MoveNode was not accepted: {accepted}")

    scene = json_request("/v1/scenes/current")
    target = request["command"]["node_id"]
    node = next((node for node in scene["nodes"] if node["node_id"] == target), None)
    if node is None:
        raise RuntimeError("interactive target missing from accepted Scene")
    if (
        node["bounds"]["x"] != request["command"]["x_emu"]
        or node["bounds"]["y"] != request["command"]["y_emu"]
    ):
        raise RuntimeError("interactive Scene does not reflect arbitrary MoveNode")

    undo = {
        "protocol_version": "chaptera.history-transition-intent.v1",
        "document_id": scene["document_id"],
        "source_hash": scene["source_hash"],
        "base_revision_id": scene["revision_id"],
        "client_operation_id": "local-interactive-smoke-undo",
        "command": {"kind": "undo"},
    }
    undone = json_request("/v1/commit", method="POST", body=undo)
    if undone.get("protocol_version") != "chaptera.history-transition-accepted.v1":
        raise RuntimeError(f"interactive Undo was not accepted: {undone}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--smoke", action="store_true")
    args = parser.parse_args()

    STATE.mkdir(parents=True, exist_ok=True)
    WORK.mkdir(parents=True, exist_ok=True)
    python = ensure_venv()
    producer = build_inputs()

    service = subprocess.Popen(
        [
            str(python),
            "services/editor-api/web_real_acceptance_service.py",
            "--interactive",
            "--port",
            str(API_PORT),
            "--fixture",
            str(FIXTURE),
            "--resolved-graph",
            str(GRAPH),
            "--viewer-receipt",
            "apps/web/acceptance/receipts/viewer-geometry.real.json",
            "--revision-receipt",
            "packages/protocol/revision/v1/producer-receipts/sample-newsletter.real.json",
            "--exporter",
            str(producer),
            "--work-dir",
            str(WORK),
        ],
        cwd=ROOT,
    )
    static = subprocess.Popen(
        [
            str(python),
            "-m",
            "http.server",
            str(WEB_PORT),
            "--bind",
            "127.0.0.1",
            "--directory",
            str(ROOT),
        ],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    try:
        wait_http(API + "/health", service)
        wait_http(
            f"http://127.0.0.1:{WEB_PORT}/apps/web/local-editor.html",
            static,
        )
        print(f"LOCAL_EDITOR_READY {EDITOR_URL}", flush=True)
        if args.smoke:
            interactive_smoke()
            return 0
        while True:
            if service.poll() is not None:
                raise RuntimeError(f"real editor service exited: {service.returncode}")
            if static.poll() is not None:
                raise RuntimeError(f"local editor web server exited: {static.returncode}")
            time.sleep(1)
    except KeyboardInterrupt:
        return 0
    finally:
        for proc in (static, service):
            if proc.poll() is None:
                proc.terminate()
        for proc in (static, service):
            if proc.poll() is None:
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()


if __name__ == "__main__":
    raise SystemExit(main())
