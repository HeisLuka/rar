#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import pathlib

from jsonschema import Draft202012Validator, FormatChecker

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "protocol" / "pub-lab-2019" / "v1"
CHALLENGE_SCHEMA = BASE / "restore-challenge.schema.json"
ENV_SCHEMA = BASE / "environment-manifest.schema.json"


def _load_schema(path: pathlib.Path) -> dict:
    schema = json.loads(path.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    return schema


def _validate_schema(value: dict, schema: dict, label: str) -> None:
    validator = Draft202012Validator(schema, format_checker=FormatChecker())
    errors = sorted(validator.iter_errors(value), key=lambda error: list(error.path))
    if errors:
        detail = "\n".join(f"{label}{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError(detail)


def _instant(value: str) -> dt.datetime:
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))


def measured_environment(manifest: dict) -> dict:
    return {
        "os_build": manifest["os_build"],
        "system_locale": manifest["system_locale"],
        "user_locale": manifest["user_locale"],
        "code_page": manifest["code_page"],
        "time_zone": manifest["time_zone"],
        "default_printer": manifest["default_printer"],
        "font_set_sha256": manifest["font_set_sha256"],
        "publisher": manifest["publisher"],
    }


def compute_environment_fingerprint(manifest: dict) -> str:
    encoded = json.dumps(
        measured_environment(manifest),
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def validate_restore_pair(challenge: dict, manifest: dict) -> dict:
    _validate_schema(challenge, _load_schema(CHALLENGE_SCHEMA), "challenge")
    _validate_schema(manifest, _load_schema(ENV_SCHEMA), "manifest")

    requested = _instant(challenge["restore_requested_at_utc"])
    started = _instant(challenge["cold_start_succeeded_at_utc"])
    captured = _instant(manifest["captured_at_utc"])

    if started < requested:
        raise AssertionError("cold-start success predates restore request")
    if captured < started:
        raise AssertionError("EnvironmentManifest predates successful cold start")
    if manifest["restore_nonce"] != challenge["restore_nonce"]:
        raise AssertionError("EnvironmentManifest restore nonce mismatch")

    recomputed = compute_environment_fingerprint(manifest)
    if manifest["environment_fingerprint"] != recomputed:
        raise AssertionError("EnvironmentManifest fingerprint is not derived from measured fields")
    if recomputed != challenge["expected_environment_fingerprint"]:
        raise AssertionError("measured environment differs from pre-start challenge")

    return {
        "schema_version": "chaptera.pub-lab-2019-restore-pair-validation.v1",
        "vm_name": challenge["vm_name"],
        "snapshot_name": challenge["snapshot_name"],
        "publisher_build": manifest["publisher"]["build"],
        "publisher_bitness": manifest["publisher"]["bitness"],
        "publisher_process_count": manifest["publisher"]["process_count"],
        "challenge_bound": True,
        "post_boot_capture": True,
        "measured_environment": True,
        "environment_match": True,
        "environment_fingerprint": recomputed,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("challenge", type=pathlib.Path)
    parser.add_argument("manifest", type=pathlib.Path)
    args = parser.parse_args()

    challenge = json.loads(args.challenge.read_text(encoding="utf-8"))
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    print(json.dumps(validate_restore_pair(challenge, manifest), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
