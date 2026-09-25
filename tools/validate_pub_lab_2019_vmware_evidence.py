#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import json
import pathlib
import re

from jsonschema import Draft202012Validator, FormatChecker

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1" / "vmware-restore-evidence.schema.json"
WINDOWS_PATH = re.compile(r"^[A-Za-z]:[\\/]")
UNC_PATH = re.compile(r"^\\\\")
SECRET_KEYS = {
    "password",
    "product_key",
    "activation_token",
    "credential",
    "secret",
    "media_path",
    "vmx_path",
    "restore_nonce",
}


def _walk(value, path=()):
    if isinstance(value, dict):
        for key, child in value.items():
            yield path + (key,), child
            yield from _walk(child, path + (key,))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from _walk(child, path + (str(index),))


def _instant(value: str) -> dt.datetime:
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))


def validate_vmware_evidence(value: dict) -> dict:
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema, format_checker=FormatChecker())
    errors = sorted(validator.iter_errors(value), key=lambda error: list(error.path))
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError(detail)

    for path, child in _walk(value):
        key = path[-1].lower() if path else ""
        if key in SECRET_KEYS:
            raise AssertionError(f"forbidden backend evidence field: {'.'.join(path)}")
        if isinstance(child, str) and (WINDOWS_PATH.match(child) or UNC_PATH.match(child)):
            raise AssertionError(f"local path leaked at {'.'.join(path)}")

    started = _instant(value["restore"]["started_at_utc"])
    completed = _instant(value["restore"]["completed_at_utc"])
    if completed < started:
        raise AssertionError("restore completion predates restore start")
    if not value["restore"]["restore_verified"]:
        raise AssertionError("restore is not verified")

    return {
        "schema_version": value["schema_version"],
        "baseline_id": value["baseline_id"],
        "snapshot_id": value["snapshot_identity"]["name"],
        "environment_fingerprint": value["environment_fingerprint"],
        "restore_verified": True,
        "source_free": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=pathlib.Path)
    args = parser.parse_args()
    value = json.loads(args.evidence.read_text(encoding="utf-8"))
    print(json.dumps(validate_vmware_evidence(value), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
