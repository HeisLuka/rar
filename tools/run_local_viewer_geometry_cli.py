#!/usr/bin/env python3
"""Strict Rar-owned launcher for a local canonical Viewer geometry engine.

The outer builder (build_viewer_geometry_receipt.py) owns the public receipt
allowlist. This launcher owns the local execution boundary:
- execution starts from the active Rar checkout;
- the caller supplies a local/private pinned PUB fixture;
- the fixture SHA-256 and byte length must match the builder request;
- an authorized local Viewer engine command is invoked with {fixture}
  substituted by the verified fixture path;
- stdout must be exactly one ViewerGeometryDocument JSON value;
- the engine-reported source identity must match the verified local bytes;
- stdout bytes are forwarded unchanged.

No PUB parsing, Viewer semantics, field stripping, receipt normalization, or
historical-repository checkout lives here.
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


class LocalViewerProducerError(RuntimeError):
    pass


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_request(raw: str) -> tuple[str, int]:
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        raise LocalViewerProducerError("builder request is not valid JSON") from error
    if not isinstance(value, dict):
        raise LocalViewerProducerError("builder request must be a JSON object")
    if set(value) != {"action", "source_hash", "source_byte_len"}:
        raise LocalViewerProducerError("builder request fields mismatch")
    if value["action"] != "viewer_geometry":
        raise LocalViewerProducerError("unsupported producer action")
    source_hash = value["source_hash"]
    source_byte_len = value["source_byte_len"]
    if (
        not isinstance(source_hash, str)
        or len(source_hash) != 64
        or any(ch not in "0123456789abcdef" for ch in source_hash)
    ):
        raise LocalViewerProducerError("source_hash must be lowercase SHA-256")
    if (
        not isinstance(source_byte_len, int)
        or isinstance(source_byte_len, bool)
        or source_byte_len < 0
    ):
        raise LocalViewerProducerError("source_byte_len must be a non-negative integer")
    return source_hash, source_byte_len


def bind_fixture(
    fixture: pathlib.Path,
    *,
    expected_hash: str,
    expected_len: int,
) -> pathlib.Path:
    path = fixture.expanduser().resolve(strict=True)
    if not path.is_file():
        raise LocalViewerProducerError("fixture path is not a regular file")
    actual_len = path.stat().st_size
    if actual_len != expected_len:
        raise LocalViewerProducerError(
            f"fixture byte length mismatch: expected={expected_len} actual={actual_len}"
        )
    actual_hash = sha256_file(path)
    if actual_hash != expected_hash:
        raise LocalViewerProducerError(
            f"fixture SHA-256 mismatch: expected={expected_hash} actual={actual_hash}"
        )
    return path


def render_command(template: list[str], fixture: pathlib.Path) -> list[str]:
    if not template:
        raise LocalViewerProducerError("viewer engine command is empty")
    placeholder_count = sum(part.count("{fixture}") for part in template)
    if placeholder_count != 1:
        raise LocalViewerProducerError(
            "viewer engine command must contain {fixture} exactly once"
        )
    return [part.replace("{fixture}", str(fixture)) for part in template]


def validate_engine_identity(
    receipt: Any,
    *,
    expected_hash: str,
    expected_len: int,
) -> None:
    if not isinstance(receipt, dict):
        raise LocalViewerProducerError("Viewer engine output must be a JSON object")
    try:
        source = receipt["document"]["source"]
    except (KeyError, TypeError) as error:
        raise LocalViewerProducerError(
            "Viewer engine output is missing document.source"
        ) from error
    if not isinstance(source, dict):
        raise LocalViewerProducerError("Viewer engine document.source must be an object")
    if source.get("source_hash") != expected_hash:
        raise LocalViewerProducerError(
            "Viewer engine source_hash differs from verified fixture"
        )
    if source.get("byte_len") != expected_len:
        raise LocalViewerProducerError(
            "Viewer engine byte_len differs from verified fixture"
        )


def run_local_viewer(
    *,
    fixture: pathlib.Path,
    command_template: list[str],
    request_raw: str,
) -> bytes:
    expected_hash, expected_len = parse_request(request_raw)
    fixture = bind_fixture(
        fixture,
        expected_hash=expected_hash,
        expected_len=expected_len,
    )
    command = render_command(command_template, fixture)
    completed = subprocess.run(
        command,
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace").strip()
        raise LocalViewerProducerError(
            "local Viewer engine failed"
            + (f": {detail}" if detail else "")
        )

    raw = completed.stdout
    try:
        decoded = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise LocalViewerProducerError("Viewer engine stdout is not UTF-8 JSON") from error
    try:
        receipt = json.loads(decoded)
    except json.JSONDecodeError as error:
        raise LocalViewerProducerError(
            "Viewer engine stdout is not exactly one JSON value"
        ) from error

    validate_engine_identity(
        receipt,
        expected_hash=expected_hash,
        expected_len=expected_len,
    )
    return raw


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Bind a verified local PUB fixture to an authorized Viewer engine command"
    )
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("viewer_command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = list(args.viewer_command)
    if command and command[0] == "--":
        command = command[1:]

    try:
        raw = run_local_viewer(
            fixture=args.fixture,
            command_template=command,
            request_raw=sys.stdin.read(),
        )
    except (LocalViewerProducerError, OSError) as error:
        print(str(error), file=sys.stderr)
        return 2

    sys.stdout.buffer.write(raw)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
