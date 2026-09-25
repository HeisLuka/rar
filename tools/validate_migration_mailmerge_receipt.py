#!/usr/bin/env python3
"""Validate a controlled Publisher mail-merge producer receipt against local artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA_PATH = ROOT / "packages" / "product" / "migration-mailmerge" / "v1" / "producer-receipt.schema.json"
EXPECTED_VALUES = ["TEXT_A", "TEXT_B", "TEXT_C"]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def safe_child(root: Path, file_name: str) -> Path:
    require(Path(file_name).name == file_name, f"unsafe file_name: {file_name!r}")
    candidate = (root / file_name).resolve()
    candidate.relative_to(root.resolve())
    require(candidate.is_file(), f"missing artifact: {file_name}")
    return candidate


def verify_artifact(root: Path, record: dict[str, Any], label: str) -> None:
    path = safe_child(root, record["file_name"])
    require(path.stat().st_size == record["byte_len"], f"{label}: byte_len mismatch")
    require(sha256_file(path) == record["sha256"], f"{label}: sha256 mismatch")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--receipt", required=True, type=Path)
    parser.add_argument("--root", required=True, type=Path)
    args = parser.parse_args()

    root = args.root.resolve()
    receipt_path = args.receipt.resolve()
    receipt_path.relative_to(root)

    receipt = json.loads(receipt_path.read_text(encoding="utf-8-sig"))
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda error: list(error.path))
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError(f"mail-merge receipt schema validation failed\n{detail}")

    require(receipt["publisher"]["version"].startswith("16.0"), "Publisher version must be 16.0*")
    require(str(receipt["publisher"]["build"]).startswith("12527"), "Publisher build must be 12527*")
    require(receipt["template_data_source_connected"] is True, "template datasource must be connected")
    require("TextValue" in receipt["data_fields"], "TextValue datasource field is required")
    require(receipt["lineage"] == "new_controlled_fixture_2026_09_26", "unexpected fixture lineage")
    require(receipt["byte_identity_with_lost_merge_clone_01"] is False, "lost historical byte identity claim is forbidden")

    fixture = receipt["fixture"]
    require(fixture["wizard_id"] == 161, "wizard_id mismatch")
    require(fixture["design"] == 1, "design mismatch")
    require(fixture["merge_destination"] == 2, "merge_destination mismatch")
    require(fixture["merge_field"] == "TextValue", "merge_field mismatch")
    require(fixture["merge_values"] == EXPECTED_VALUES, "merge_values mismatch")
    require(fixture["data_source_table"] == "Sheet1$", "datasource table mismatch")

    verify_artifact(root, receipt["data_source"], "data_source")
    verify_artifact(root, receipt["template"], "template")

    outputs = receipt["outputs"]
    require([row["record_count"] for row in outputs] == [1, 2, 3], "outputs must be exactly record counts 1/2/3")

    seen_files: set[str] = set()
    for row in outputs:
        count = row["record_count"]
        expected = EXPECTED_VALUES[:count]
        require(row["expected_values"] == expected, f"record_count={count}: expected_values mismatch")
        joined = "\n".join(row["observed_text"])
        for value in expected:
            require(value in joined, f"record_count={count}: missing visible text {value}")
        file_name = row["artifact"]["file_name"]
        require(file_name not in seen_files, f"duplicate output artifact file: {file_name}")
        seen_files.add(file_name)
        verify_artifact(root, row["artifact"], f"output-{count}")

    print(json.dumps({
        "status": "valid",
        "receipt_version": receipt["receipt_version"],
        "publisher": receipt["publisher"],
        "lineage": receipt["lineage"],
        "outputs": [
            {"record_count": row["record_count"], "sha256": row["artifact"]["sha256"]}
            for row in outputs
        ],
    }, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
