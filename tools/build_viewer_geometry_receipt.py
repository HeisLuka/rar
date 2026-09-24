#!/usr/bin/env python3
"""Run the minimum local Viewer producer and admit its exact source-free JSON.

This tool intentionally owns no PUB parsing or Viewer semantics. The external
authorized producer writes exactly one ViewerGeometryDocument JSON value to
stdout. Rar validates that unmodified value against the public allowlist and
pinned source identity before retaining the exact producer bytes.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from validate_viewer_geometry_receipt import validate_schema


def invoke_producer(
    command: list[str],
    *,
    source_hash: str,
    source_byte_len: int,
) -> tuple[dict[str, Any], bytes]:
    request = {
        "action": "viewer_geometry",
        "source_hash": source_hash,
        "source_byte_len": source_byte_len,
    }
    completed = subprocess.run(
        command,
        input=json.dumps(request, separators=(",", ":")),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "Viewer producer failed"
            + (f"\nstderr:\n{completed.stderr}" if completed.stderr else "")
        )
    raw = completed.stdout.encode("utf-8")
    try:
        receipt = json.loads(raw)
    except json.JSONDecodeError as error:
        raise RuntimeError("Viewer producer stdout is not exactly one JSON value") from error
    if not isinstance(receipt, dict):
        raise RuntimeError("Viewer producer output must be a JSON object")
    return receipt, raw


def validate_identity(
    receipt: dict[str, Any],
    *,
    source_hash: str,
    source_byte_len: int,
) -> None:
    validate_schema(receipt)
    source = receipt["document"]["source"]
    if source["source_hash"] != source_hash:
        raise RuntimeError("Viewer producer source_hash differs from pinned source")
    if source["byte_len"] != source_byte_len:
        raise RuntimeError("Viewer producer byte_len differs from pinned source")


def build_viewer_receipt(
    command: list[str],
    *,
    source_hash: str,
    source_byte_len: int,
) -> tuple[dict[str, Any], bytes]:
    receipt, raw = invoke_producer(
        command,
        source_hash=source_hash,
        source_byte_len=source_byte_len,
    )
    validate_identity(
        receipt,
        source_hash=source_hash,
        source_byte_len=source_byte_len,
    )
    return receipt, raw


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run local canonical Viewer producer and retain exact validated JSON"
    )
    parser.add_argument("--source-hash", required=True)
    parser.add_argument("--source-byte-len", required=True, type=int)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("producer_command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    producer_command = list(args.producer_command)
    if producer_command and producer_command[0] == "--":
        producer_command = producer_command[1:]
    if not producer_command:
        parser.error("producer_command is required after --")
    if args.source_byte_len < 0:
        parser.error("--source-byte-len must be non-negative")

    receipt, raw = build_viewer_receipt(
        producer_command,
        source_hash=args.source_hash,
        source_byte_len=args.source_byte_len,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(raw)

    source = receipt["document"]["source"]
    print(json.dumps({
        "status": "valid",
        "output": str(args.output),
        "source_hash": source["source_hash"],
        "byte_len": source["byte_len"],
        "page_count": len(receipt["document"]["pages"]),
        "node_count": len(receipt["scene"]["nodes"]),
    }, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
