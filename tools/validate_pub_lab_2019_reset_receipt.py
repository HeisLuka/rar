#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import pathlib
import re
from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1" / "reset-receipt.schema.json"
WINDOWS_PATH = re.compile(r"^[A-Za-z]:[\\/]")
UNC_PATH = re.compile(r"^\\\\")
SECRET_KEYS = {"password", "product_key", "activation_token", "credential", "secret", "media_path", "vmx_path"}


def _walk(value, path=()):
    if isinstance(value, dict):
        for key, child in value.items():
            yield path + (key,), child
            yield from _walk(child, path + (key,))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from _walk(child, path + (str(index),))


def validate_receipt(value: dict) -> dict:
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(
        Draft202012Validator(schema).iter_errors(value),
        key=lambda error: list(error.path),
    )
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError(detail)

    for path, child in _walk(value):
        key = path[-1].lower() if path else ""
        if key in SECRET_KEYS:
            raise AssertionError(f"forbidden receipt field: {'.'.join(path)}")
        if isinstance(child, str) and (WINDOWS_PATH.match(child) or UNC_PATH.match(child)):
            raise AssertionError(f"local path leaked at {'.'.join(path)}")

    if value["vm_identity"]["name"] != "PUB-LAB-2019":
        raise AssertionError("wrong VM identity")
    if value["snapshot_identity"]["name"] != "MODERN-2019-12527-GOLDEN-v1":
        raise AssertionError("wrong golden snapshot identity")
    if not value["restore"]["restore_verified"]:
        raise AssertionError("restore not verified")

    return {
        "schema_version": value["schema_version"],
        "vm": value["vm_identity"]["name"],
        "snapshot": value["snapshot_identity"]["name"],
        "restore_verified": True,
        "source_free": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("receipt", type=pathlib.Path)
    args = parser.parse_args()
    value = json.loads(args.receipt.read_text(encoding="utf-8"))
    print(json.dumps(validate_receipt(value), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
