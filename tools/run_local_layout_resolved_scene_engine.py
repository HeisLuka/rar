#!/usr/bin/env python3
"""Strict Rar-owned launcher for the authoritative resolved-graph Scene engine.

The public receipt builder deliberately invokes its producer once per lifecycle
action. A real local engine must not turn those process boundaries into hidden
post-edit PUB reparses.

This launcher therefore enforces a two-phase local contract:
- baseline is the only action that receives the verified source PUB path;
- the engine materializes reusable source-free/canonical session state in a
  caller-supplied local state directory;
- commit/history/replay receive only that state directory and the builder JSON;
  the source PUB path is never present in their argv or stdin;
- all engine stdout is forwarded unchanged after basic JSON/privacy checks.

The engine/runtime and its local state may remain private. Rar owns the
execution contract and the public receipt validator, not the private engine.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINDING_FILE = ".chaptera-layout-session-binding.json"
ALLOWED_ACTIONS = {"baseline", "commit", "history", "replay"}


class LocalResolvedSceneError(RuntimeError):
    pass


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_request(raw: str) -> dict[str, Any]:
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        raise LocalResolvedSceneError("builder request is not valid JSON") from error
    if not isinstance(value, dict):
        raise LocalResolvedSceneError("builder request must be a JSON object")

    action = value.get("action")
    if action not in ALLOWED_ACTIONS:
        raise LocalResolvedSceneError("unsupported resolved-scene action")

    source_hash = value.get("source_hash")
    if (
        not isinstance(source_hash, str)
        or len(source_hash) != 64
        or any(ch not in "0123456789abcdef" for ch in source_hash)
    ):
        raise LocalResolvedSceneError("source_hash must be lowercase SHA-256")
    return value


def binding_path(state_dir: pathlib.Path) -> pathlib.Path:
    return state_dir / BINDING_FILE


def verify_fixture(
    fixture: pathlib.Path,
    *,
    source_hash: str,
    source_byte_len: int,
) -> pathlib.Path:
    path = fixture.expanduser().resolve(strict=True)
    if not path.is_file():
        raise LocalResolvedSceneError("fixture path is not a regular file")
    actual_len = path.stat().st_size
    if actual_len != source_byte_len:
        raise LocalResolvedSceneError(
            f"fixture byte length mismatch: expected={source_byte_len} actual={actual_len}"
        )
    actual_hash = sha256_file(path)
    if actual_hash != source_hash:
        raise LocalResolvedSceneError(
            f"fixture SHA-256 mismatch: expected={source_hash} actual={actual_hash}"
        )
    return path


def read_binding(state_dir: pathlib.Path, source_hash: str, source_byte_len: int) -> None:
    marker = binding_path(state_dir)
    if not marker.is_file():
        raise LocalResolvedSceneError(
            "local resolved-scene state is not bootstrapped; run baseline first"
        )
    try:
        value = json.loads(marker.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise LocalResolvedSceneError("invalid local state binding marker") from error
    expected = {
        "binding_version": "chaptera.local-resolved-scene-binding.v1",
        "source_hash": source_hash,
        "source_byte_len": source_byte_len,
    }
    if value != expected:
        raise LocalResolvedSceneError("local state binding does not match pinned source")


def write_binding(state_dir: pathlib.Path, source_hash: str, source_byte_len: int) -> None:
    state_dir.mkdir(parents=True, exist_ok=True)
    marker = binding_path(state_dir)
    value = {
        "binding_version": "chaptera.local-resolved-scene-binding.v1",
        "source_hash": source_hash,
        "source_byte_len": source_byte_len,
    }
    marker.write_text(
        json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )


def engine_command(
    prefix: list[str],
    *,
    action: str,
    state_dir: pathlib.Path,
    fixture: pathlib.Path | None,
) -> list[str]:
    if not prefix:
        raise LocalResolvedSceneError("resolved-scene engine command is empty")
    command = list(prefix) + [action, "--state-dir", str(state_dir)]
    if fixture is not None:
        command += ["--fixture", str(fixture)]
    return command


def reject_local_path_leakage(raw: bytes, paths: list[pathlib.Path]) -> None:
    decoded = raw.decode("utf-8", errors="strict")
    for path in paths:
        candidates = {
            str(path),
            str(path).replace("\\", "/"),
        }
        for candidate in candidates:
            if candidate and candidate in decoded:
                raise LocalResolvedSceneError(
                    "resolved-scene engine stdout leaked a local fixture/state path"
                )


def run_local_resolved_scene(
    *,
    fixture: pathlib.Path,
    source_byte_len: int,
    state_dir: pathlib.Path,
    engine_prefix: list[str],
    request_raw: str,
) -> bytes:
    request = parse_request(request_raw)
    action = request["action"]
    source_hash = request["source_hash"]

    state_dir = state_dir.expanduser().resolve()
    fixture_path: pathlib.Path | None = None

    if action == "baseline":
        fixture_path = verify_fixture(
            fixture,
            source_hash=source_hash,
            source_byte_len=source_byte_len,
        )
        # A stale state directory may silently bind a new baseline to an old
        # graph/session. Reuse is allowed only when the marker already matches.
        if binding_path(state_dir).exists():
            read_binding(state_dir, source_hash, source_byte_len)
        else:
            state_dir.mkdir(parents=True, exist_ok=True)
    else:
        read_binding(state_dir, source_hash, source_byte_len)

    command = engine_command(
        engine_prefix,
        action=action,
        state_dir=state_dir,
        fixture=fixture_path,
    )
    completed = subprocess.run(
        command,
        cwd=ROOT,
        input=request_raw.encode("utf-8"),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace").strip()
        raise LocalResolvedSceneError(
            "local resolved-scene engine failed"
            + (f": {detail}" if detail else "")
        )

    raw = completed.stdout
    try:
        decoded = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise LocalResolvedSceneError(
            "resolved-scene engine stdout is not UTF-8 JSON"
        ) from error
    try:
        output = json.loads(decoded)
    except json.JSONDecodeError as error:
        raise LocalResolvedSceneError(
            "resolved-scene engine stdout is not exactly one JSON value"
        ) from error
    if not isinstance(output, dict):
        raise LocalResolvedSceneError(
            "resolved-scene engine output must be a JSON object"
        )

    reject_local_path_leakage(
        raw,
        [path for path in (fixture_path, state_dir) if path is not None],
    )

    if action == "baseline":
        if output.get("source_hash") != source_hash:
            raise LocalResolvedSceneError(
                "baseline engine source_hash differs from verified fixture"
            )
        write_binding(state_dir, source_hash, source_byte_len)
    else:
        if output.get("source_hash_after") != source_hash:
            raise LocalResolvedSceneError(
                "post-baseline engine source identity changed"
            )
        if output.get("source_reparse_after_edit_count") != 0:
            raise LocalResolvedSceneError(
                "post-baseline engine reported immutable-source reparse"
            )

    return raw


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Bind a pinned PUB once, then run current-state Scene lifecycle "
            "actions without passing source bytes/path after baseline"
        )
    )
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--source-byte-len", required=True, type=int)
    parser.add_argument("--state-dir", required=True, type=pathlib.Path)
    parser.add_argument("engine_command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = list(args.engine_command)
    if command and command[0] == "--":
        command = command[1:]
    if args.source_byte_len < 0:
        parser.error("--source-byte-len must be non-negative")

    try:
        raw = run_local_resolved_scene(
            fixture=args.fixture,
            source_byte_len=args.source_byte_len,
            state_dir=args.state_dir,
            engine_prefix=command,
            request_raw=sys.stdin.read(),
        )
    except (LocalResolvedSceneError, OSError, UnicodeDecodeError) as error:
        print(str(error), file=sys.stderr)
        return 2

    sys.stdout.buffer.write(raw)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
