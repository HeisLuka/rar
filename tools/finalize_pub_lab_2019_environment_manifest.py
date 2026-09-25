#!/usr/bin/env python3
from __future__ import annotations

import argparse
import copy
import json
import pathlib

from jsonschema import Draft202012Validator, FormatChecker

from tools.validate_pub_lab_2019_restore_pair import compute_environment_fingerprint, validate_restore_pair

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1"
RAW_SCHEMA = BASE / "environment-capture-raw.schema.json"


def _load_json(path: pathlib.Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def _validate_raw(value: dict) -> None:
    schema = _load_json(RAW_SCHEMA)
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema, format_checker=FormatChecker())
    errors = sorted(validator.iter_errors(value), key=lambda error: list(error.path))
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError(detail)


def finalize_manifest(challenge: dict, raw: dict) -> dict:
    _validate_raw(raw)
    if raw["restore_nonce"] != challenge["restore_nonce"]:
        raise AssertionError("raw environment capture restore nonce mismatch")
    if raw["vm_name"] != challenge["vm_name"]:
        raise AssertionError("raw environment capture VM identity mismatch")

    manifest = copy.deepcopy(raw)
    manifest["schema_version"] = "chaptera.publisher2019-environment-manifest.v1"
    manifest["environment_fingerprint"] = compute_environment_fingerprint(manifest)

    # This performs schema, timestamp, nonce, Publisher build/process and expected-fingerprint checks.
    validate_restore_pair(challenge, manifest)
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--challenge", type=pathlib.Path, required=True)
    parser.add_argument("--raw", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()

    challenge = _load_json(args.challenge)
    raw = _load_json(args.raw)
    manifest = finalize_manifest(challenge, raw)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(manifest, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
