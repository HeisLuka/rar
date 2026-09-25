from __future__ import annotations

import re
from datetime import datetime

SAFE_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{1,127}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


def _instant(value: str) -> datetime:
    if not isinstance(value, str) or not value.endswith("Z"):
        raise AssertionError("reset timestamps must be RFC3339 UTC ending in Z")
    return datetime.fromisoformat(value[:-1] + "+00:00")


def verify_compat(
    doc: dict,
    *,
    expected_baseline: str,
    expected_snapshot: str,
    expected_experiment: str,
    expected_packet_sha256: str,
) -> None:
    if doc.get("schema") != "pub-research-reset-receipt.v1":
        raise AssertionError("wrong provider-neutral reset schema")
    for field in ("provider_id", "baseline_id", "snapshot_id", "experiment_id"):
        value = doc.get(field)
        if not isinstance(value, str) or not SAFE_ID_RE.fullmatch(value):
            raise AssertionError(f"{field} is not a safe identifier")
    for field in (
        "packet_sha256",
        "pre_restore_state_sha256",
        "post_restore_state_sha256",
        "environment_fingerprint_sha256",
    ):
        if not SHA256_RE.fullmatch(str(doc.get(field, ""))):
            raise AssertionError(f"{field} is not SHA-256")
    if doc["baseline_id"] != expected_baseline:
        raise AssertionError("baseline mismatch")
    if doc["snapshot_id"] != expected_snapshot:
        raise AssertionError("snapshot mismatch")
    if doc["experiment_id"] != expected_experiment:
        raise AssertionError("experiment mismatch")
    if doc["packet_sha256"] != expected_packet_sha256:
        raise AssertionError("packet mismatch")
    if _instant(doc["restore_completed_at_utc"]) < _instant(doc["restore_started_at_utc"]):
        raise AssertionError("restore timestamp reversal")
    if doc.get("restore_verified") is not True or doc.get("failure_reason") not in (None, ""):
        raise AssertionError("receipt is not verified")
