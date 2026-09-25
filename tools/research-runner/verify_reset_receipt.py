#!/usr/bin/env python3
"""Fail-closed verifier for native reset/restore receipts."""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import datetime
from pathlib import Path
from typing import Any


SCHEMA = "pub-research-reset-receipt.v1"
SAFE_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{1,127}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"reset receipt validation failed: {message}")


def require_string(doc: dict[str, Any], field: str, *, safe_id: bool = False) -> str:
    value = doc.get(field)
    if not isinstance(value, str) or not value:
        fail(f"{field} must be a non-empty string")
    if safe_id and not SAFE_ID_RE.fullmatch(value):
        fail(f"{field} contains unsafe identifier characters")
    return value


def require_sha256(doc: dict[str, Any], field: str) -> str:
    value = require_string(doc, field)
    if not SHA256_RE.fullmatch(value):
        fail(f"{field} must be 64 lowercase hex SHA-256 characters")
    return value


def parse_utc(value: str, field: str) -> datetime:
    if not value.endswith("Z"):
        fail(f"{field} must be an RFC3339 UTC timestamp ending in Z")
    try:
        return datetime.fromisoformat(value[:-1] + "+00:00")
    except ValueError as exc:
        fail(f"{field} is not a valid RFC3339 timestamp: {exc}")


def load_receipt(path: Path) -> dict[str, Any]:
    try:
        doc = json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot parse receipt {path}: {exc}")
    if not isinstance(doc, dict):
        fail("receipt root must be an object")
    return doc


def validate_receipt(
    doc: dict[str, Any],
    *,
    expected_baseline: str,
    expected_snapshot: str,
    expected_experiment: str,
    expected_packet_sha256: str,
) -> dict[str, str]:
    if doc.get("schema") != SCHEMA:
        fail(f"schema must be {SCHEMA!r}")

    provider_id = require_string(doc, "provider_id", safe_id=True)
    provider_version = require_string(doc, "provider_version")
    baseline_id = require_string(doc, "baseline_id", safe_id=True)
    snapshot_id = require_string(doc, "snapshot_id", safe_id=True)
    experiment_id = require_string(doc, "experiment_id", safe_id=True)
    packet_sha256 = require_sha256(doc, "packet_sha256")
    pre_state = require_sha256(doc, "pre_restore_state_sha256")
    post_state = require_sha256(doc, "post_restore_state_sha256")
    environment = require_sha256(doc, "environment_fingerprint_sha256")

    if baseline_id != expected_baseline:
        fail(f"baseline mismatch: expected {expected_baseline!r}, got {baseline_id!r}")
    if snapshot_id != expected_snapshot:
        fail(f"snapshot mismatch: expected {expected_snapshot!r}, got {snapshot_id!r}")
    if experiment_id != expected_experiment:
        fail(f"experiment mismatch: expected {expected_experiment!r}, got {experiment_id!r}")
    if packet_sha256 != expected_packet_sha256:
        fail("packet_sha256 does not match the currently validated experiment packet")

    started_text = require_string(doc, "restore_started_at_utc")
    completed_text = require_string(doc, "restore_completed_at_utc")
    started = parse_utc(started_text, "restore_started_at_utc")
    completed = parse_utc(completed_text, "restore_completed_at_utc")
    if completed < started:
        fail("restore_completed_at_utc precedes restore_started_at_utc")

    verified = doc.get("restore_verified")
    if verified is not True:
        reason = doc.get("failure_reason")
        fail(f"restore_verified is not true; failure_reason={reason!r}")
    if doc.get("failure_reason") not in (None, ""):
        fail("verified receipt must not contain a failure_reason")

    return {
        "provider_id": provider_id,
        "provider_version": provider_version,
        "baseline_id": baseline_id,
        "snapshot_id": snapshot_id,
        "experiment_id": experiment_id,
        "packet_sha256": packet_sha256,
        "pre_restore_state_sha256": pre_state,
        "post_restore_state_sha256": post_state,
        "environment_fingerprint_sha256": environment,
        "restore_started_at_utc": started_text,
        "restore_completed_at_utc": completed_text,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--receipt", required=True)
    parser.add_argument("--expected-baseline", required=True)
    parser.add_argument("--expected-snapshot", required=True)
    parser.add_argument("--expected-experiment", required=True)
    parser.add_argument("--expected-packet-sha256", required=True)
    parser.add_argument("--compare-receipt")
    args = parser.parse_args()

    if not SHA256_RE.fullmatch(args.expected_packet_sha256):
        fail("--expected-packet-sha256 must be 64 lowercase hex characters")

    primary = validate_receipt(
        load_receipt(Path(args.receipt)),
        expected_baseline=args.expected_baseline,
        expected_snapshot=args.expected_snapshot,
        expected_experiment=args.expected_experiment,
        expected_packet_sha256=args.expected_packet_sha256,
    )

    if args.compare_receipt:
        comparison = validate_receipt(
            load_receipt(Path(args.compare_receipt)),
            expected_baseline=args.expected_baseline,
            expected_snapshot=args.expected_snapshot,
            expected_experiment=args.expected_experiment,
            expected_packet_sha256=args.expected_packet_sha256,
        )
        for field in ("provider_id", "baseline_id", "snapshot_id", "environment_fingerprint_sha256"):
            if primary[field] != comparison[field]:
                fail(
                    f"cold-restore comparison drift for {field}: "
                    f"{primary[field]!r} != {comparison[field]!r}"
                )

    print(json.dumps({"schema": SCHEMA, **primary}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
