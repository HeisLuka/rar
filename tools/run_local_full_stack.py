#!/usr/bin/env python3
"""One-command local Chaptera full-stack launcher.

This is a developer/product runtime convenience layer, not a production deploy.
It starts a configured Chaptera serve + worker on loopback, keeps a local OIDC
fixture alive, writes persistent logs under .chaptera-local/, waits for
/ready=200 and opens the browser diagnostics surface.
"""

from __future__ import annotations

import argparse
import html
import os
import pathlib
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
import webbrowser

from run_cloud_config_receipt import OidcFixture

DEV_ROOT = pathlib.Path(__file__).resolve().parents[1]
ROOT = pathlib.Path(os.environ.get("CHAPTERA_LOCAL_RUNTIME_ROOT", DEV_ROOT)).resolve()
PACKAGED = os.environ.get("CHAPTERA_LOCAL_PACKAGED") == "1" or (
    (ROOT / "BUILD.json").is_file() and (ROOT / "bin" / "chaptera.exe").is_file()
)
if os.environ.get("CHAPTERA_LOCAL_STATE_ROOT"):
    STATE = pathlib.Path(os.environ["CHAPTERA_LOCAL_STATE_ROOT"]).resolve()
elif PACKAGED and os.name == "nt" and os.environ.get("LOCALAPPDATA"):
    STATE = pathlib.Path(os.environ["LOCALAPPDATA"]) / "Chaptera" / "Local"
else:
    STATE = ROOT / ".chaptera-local"
LOGS = STATE / "logs"
CONFIG = STATE / "chaptera.local.toml"
DATABASE = STATE / "chaptera.sqlite"
CREDENTIALS = STATE / "credentials"
SERVER_LOG = LOGS / "server.log"
WORKER_LOG = LOGS / "worker.log"
EDITOR_LOG = LOGS / "editor-service.log"
URL = "http://127.0.0.1:18082"
DASHBOARD = URL + "/local"


def run_checked(args: list[str], env: dict[str, str]) -> None:
    proc = subprocess.run(args, cwd=ROOT, env=env, text=True)
    if proc.returncode != 0:
        raise RuntimeError(f"command failed ({proc.returncode}): {' '.join(args)}")


def http_code(url: str) -> int | None:
    try:
        with urllib.request.urlopen(url, timeout=1) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code
    except OSError:
        return None


def require_loopback_ports_available(ports: tuple[int, ...]) -> None:
    for port in ports:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
            try:
                probe.bind(("127.0.0.1", port))
            except OSError as error:
                raise RuntimeError(
                    f"required local port 127.0.0.1:{port} is unavailable: {error}"
                ) from error


def tail(path: pathlib.Path, count: int = 80) -> str:
    if not path.exists():
        return "(log file does not exist)"
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    return "\n".join(lines[-count:])


def open_failure_page(message: str) -> None:
    STATE.mkdir(parents=True, exist_ok=True)
    page = STATE / "startup-error.html"
    body = f"""<!doctype html><meta charset="utf-8"><title>Chaptera Local — startup error">
<style>body{{font:15px system-ui;background:#111317;color:#eef1f5;max-width:1100px;margin:30px auto;padding:0 20px}}pre{{white-space:pre-wrap;background:#1a1e24;padding:14px;border-radius:8px;color:#ffb4b4}}code{{color:#c7d2fe}}</style>
<h1>Chaptera Local не запустился</h1>
<p>{html.escape(message)}</p>
<h2>server.log</h2><pre>{html.escape(tail(SERVER_LOG))}</pre>
<h2>worker.log</h2><pre>{html.escape(tail(WORKER_LOG))}</pre>
<h2>editor-service.log</h2><pre>{html.escape(tail(EDITOR_LOG))}</pre>
<p>Файлы логов: <code>{html.escape(str(LOGS))}</code></p>"""
    page.write_text(body, encoding="utf-8")
    webbrowser.open(page.resolve().as_uri())


def render_config(issuer: str) -> None:
    example = (ROOT / "deploy/config/chaptera.prod.example.toml").read_text(encoding="utf-8")
    rendered = example
    rendered = rendered.replace('environment = "prod"', 'environment = "test"', 1)
    rendered = rendered.replace('listen = "127.0.0.1:8080"', 'listen = "127.0.0.1:18082"', 1)
    rendered = rendered.replace(
        'public_origin = "https://cloud.example.invalid"',
        'public_origin = "http://127.0.0.1:18082"',
        1,
    )
    rendered = rendered.replace(
        'path = "/var/lib/chaptera/chaptera.sqlite"',
        f'path = "{DATABASE.resolve().as_posix()}"',
        1,
    )
    rendered = rendered.replace(
        'issuer = "https://id.example.invalid"',
        f'issuer = "{issuer}"',
        1,
    )
    CONFIG.write_text(rendered, encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--no-browser", action="store_true")
    parser.add_argument("--smoke", action="store_true")
    args = parser.parse_args()

    STATE.mkdir(parents=True, exist_ok=True)
    LOGS.mkdir(parents=True, exist_ok=True)
    CREDENTIALS.mkdir(parents=True, exist_ok=True)
    (CREDENTIALS / "oidc_client_secret").write_text("chaptera-local-only-secret\n", encoding="utf-8")

    env = os.environ.copy()
    env.update(
        {
            "CREDENTIALS_DIRECTORY": str(CREDENTIALS.resolve()),
            "AWS_ACCESS_KEY_ID": "chaptera-local",
            "AWS_SECRET_ACCESS_KEY": "chaptera-local",
            "AWS_REGION": "us-east-1",
            "AWS_EC2_METADATA_DISABLED": "true",
            "CHAPTERA_LOCAL_RUNTIME_ROOT": str(ROOT),
            "CHAPTERA_LOCAL_STATE_ROOT": str(STATE),
            "CHAPTERA_LOCAL_PACKAGED": "1" if PACKAGED else "0",
        }
    )

    server = None
    worker = None
    editor = None
    server_handle = None
    worker_handle = None
    editor_handle = None

    try:
        if PACKAGED:
            require_loopback_ports_available((18082, 18765, 18083))
        if not args.skip_build and not PACKAGED:
            run_checked(["cargo", "build", "-p", "chaptera-server"], env)
        if PACKAGED:
            binary = ROOT / "bin" / ("chaptera.exe" if os.name == "nt" else "chaptera")
        else:
            binary = ROOT / "target/debug/chaptera"
            if os.name == "nt":
                binary = binary.with_suffix(".exe")
        if not binary.exists():
            raise RuntimeError(f"Chaptera binary not found: {binary}")

        with OidcFixture() as oidc:
            render_config(oidc.issuer)
            run_checked([str(binary), "--config", str(CONFIG), "migrate", "up"], env)

            server_handle = SERVER_LOG.open("w", encoding="utf-8")
            worker_handle = WORKER_LOG.open("w", encoding="utf-8")
            server = subprocess.Popen(
                [str(binary), "--config", str(CONFIG), "serve"],
                cwd=ROOT,
                env=env,
                stdout=server_handle,
                stderr=subprocess.STDOUT,
                text=True,
            )
            worker = subprocess.Popen(
                [str(binary), "--config", str(CONFIG), "worker"],
                cwd=ROOT,
                env=env,
                stdout=worker_handle,
                stderr=subprocess.STDOUT,
                text=True,
            )

            deadline = time.monotonic() + 30
            ready = None
            while time.monotonic() < deadline:
                if server.poll() is not None:
                    raise RuntimeError(f"server exited during startup with code {server.returncode}")
                if worker.poll() is not None:
                    raise RuntimeError(f"worker exited during startup with code {worker.returncode}")
                ready = http_code(URL + "/ready")
                if ready == 200:
                    break
                time.sleep(0.25)
            if ready != 200:
                raise RuntimeError(f"runtime did not reach /ready=200 (last status: {ready})")

            editor_handle = EDITOR_LOG.open("w", encoding="utf-8")
            editor_launcher = (
                ROOT / "launcher/run_local_real_editor.py"
                if PACKAGED
                else ROOT / "tools/run_local_real_editor.py"
            )
            editor = subprocess.Popen(
                [sys.executable, str(editor_launcher)],
                cwd=ROOT,
                env=env,
                stdout=editor_handle,
                stderr=subprocess.STDOUT,
                text=True,
            )

            print(f"Chaptera Local READY: {DASHBOARD}")
            print(f"Server log: {SERVER_LOG}")
            print(f"Worker log: {WORKER_LOG}")
            print(f"Editor log: {EDITOR_LOG}")
            print("Use the browser UI; Ctrl+C here stops the local stack.")
            if not args.no_browser:
                webbrowser.open(DASHBOARD)
            if args.smoke:
                status = urllib.request.urlopen(URL + "/local/api/status", timeout=2)
                if status.status != 200:
                    raise RuntimeError(f"local diagnostics status returned {status.status}")
                dashboard = urllib.request.urlopen(DASHBOARD, timeout=2)
                if dashboard.status != 200:
                    raise RuntimeError(f"local dashboard returned {dashboard.status}")

                editor_deadline = time.monotonic() + 240
                editor_ready = None
                while time.monotonic() < editor_deadline:
                    if editor.poll() is not None:
                        raise RuntimeError(
                            f"local editor exited during startup with code {editor.returncode}; "
                            f"see {EDITOR_LOG}"
                        )
                    editor_ready = http_code("http://127.0.0.1:18765/health")
                    editor_page = http_code(
                        "http://127.0.0.1:18083/apps/web/local-editor.html"
                    )
                    if editor_ready == 200 and editor_page == 200:
                        break
                    time.sleep(0.5)
                if editor_ready != 200 or editor_page != 200:
                    raise RuntimeError(
                        "local editor did not become browser-ready "
                        f"(api={editor_ready}, page={editor_page}); see {EDITOR_LOG}"
                    )
                return 0

            while True:
                if server.poll() is not None:
                    raise RuntimeError(f"server exited with code {server.returncode}")
                if worker.poll() is not None:
                    raise RuntimeError(f"worker exited with code {worker.returncode}")
                if editor is not None and editor.poll() is not None:
                    print(
                        f"Local editor bootstrap exited with code {editor.returncode}; "
                        f"base stack remains available. See {EDITOR_LOG}",
                        file=sys.stderr,
                    )
                    editor = None
                time.sleep(1)
    except KeyboardInterrupt:
        return 0
    except Exception as error:
        print(f"Chaptera Local failed: {error}", file=sys.stderr)
        open_failure_page(str(error))
        return 1
    finally:
        for proc in (editor, worker, server):
            if proc is not None and proc.poll() is None:
                proc.terminate()
        for proc in (editor, worker, server):
            if proc is not None and proc.poll() is None:
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
        if server_handle is not None:
            server_handle.close()
        if worker_handle is not None:
            worker_handle.close()
        if editor_handle is not None:
            editor_handle.close()


if __name__ == "__main__":
    raise SystemExit(main())
