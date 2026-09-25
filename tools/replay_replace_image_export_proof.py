#!/usr/bin/env python3
"""Replay one private ReplaceImage export proof into the existing receipt builder.

The private proof is produced by the real Chaptera desktop evidence test on the
same runner. This adapter validates the builder request binding and returns only
the proof object; it does not synthesize export evidence.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--proof", required=True, type=pathlib.Path)
    args = parser.parse_args()

    request = json.load(sys.stdin)
    packet = json.loads(args.proof.read_text(encoding="utf-8"))

    if packet.get("request") != {
        "action": request.get("action"),
        "source_hash": request.get("source_hash"),
        "replacement_binding_id": request.get("replacement_binding_id"),
        "fixture_kind": request.get("fixture_kind"),
    }:
        raise RuntimeError("private ReplaceImage proof does not match builder request")

    proof = packet.get("proof")
    if not isinstance(proof, dict):
        raise RuntimeError("private ReplaceImage proof payload is missing")
    json.dump(proof, sys.stdout, ensure_ascii=False, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
