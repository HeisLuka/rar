#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import pathlib
import re

from tools.validate_pub_lab_2019_vmware_evidence import validate_vmware_evidence

SCHEMA = "pub-research-reset-receipt.v1"
PROVIDER_ID = "vmware-workstation-pub-lab-2019"
PROVIDER_VERSION = "1.0"
SAFE_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{1,127}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


def _safe_id(value: str, field: str) -> str:
    if not SAFE_ID_RE.fullmatch(value):
        raise AssertionError(f"{field} must be a 2-128 character safe identifier")
    return value


def build_provider_receipt(
    evidence: dict,
    *,
    baseline_id: str,
    snapshot_id: str,
    experiment_id: str,
    packet_sha256: str,
) -> dict:
    summary = validate_vmware_evidence(evidence)
    _safe_id(baseline_id, "baseline_id")
    _safe_id(snapshot_id, "snapshot_id")
    _safe_id(experiment_id, "experiment_id")
    if not SHA256_RE.fullmatch(packet_sha256):
        raise AssertionError("packet_sha256 must be lowercase SHA-256")

    if baseline_id != summary["baseline_id"]:
        raise AssertionError("requested baseline does not match VMware evidence")
    if snapshot_id != summary["snapshot_id"]:
        raise AssertionError("requested snapshot does not match VMware evidence")

    restore = evidence["restore"]
    return {
        "schema": SCHEMA,
        "provider_id": PROVIDER_ID,
        "provider_version": PROVIDER_VERSION,
        "baseline_id": baseline_id,
        "snapshot_id": snapshot_id,
        "experiment_id": experiment_id,
        "packet_sha256": packet_sha256,
        "restore_started_at_utc": restore["started_at_utc"],
        "restore_completed_at_utc": restore["completed_at_utc"],
        "pre_restore_state_sha256": restore["pre_restore_state_sha256"],
        "post_restore_state_sha256": restore["post_restore_state_sha256"],
        "environment_fingerprint_sha256": evidence["environment_fingerprint"],
        "restore_verified": True,
        "failure_reason": None,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", required=True, type=pathlib.Path)
    parser.add_argument("--baseline-id", required=True)
    parser.add_argument("--snapshot-id", required=True)
    parser.add_argument("--experiment-id", required=True)
    parser.add_argument("--packet-sha256", required=True)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    args = parser.parse_args()

    evidence = json.loads(args.evidence.read_text(encoding="utf-8"))
    receipt = build_provider_receipt(
        evidence,
        baseline_id=args.baseline_id,
        snapshot_id=args.snapshot_id,
        experiment_id=args.experiment_id,
        packet_sha256=args.packet_sha256.lower(),
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
