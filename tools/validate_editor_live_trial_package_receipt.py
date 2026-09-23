#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "product" / "editor-live-trial" / "v1"
SCHEMA = BASE / "package-receipt.schema.json"


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("editor live-trial package receipt schema validation failed\n" + detail)


def validate_semantics(receipt):
    if receipt["product"] != "Chaptera Editor":
        raise AssertionError("trial artifact must identify Chaptera Editor")

    build = receipt["build"]
    if build["platform"] != "windows" or build["arch"] != "x86_64":
        raise AssertionError("V0 trial package must be Windows x86_64")
    if build["package_kind"] != "portable_zip":
        raise AssertionError("V0 trial package must be a portable ZIP")
    if build["binary_sha256"] == build["zip_sha256"]:
        raise AssertionError("binary and ZIP identities must be distinct artifact hashes")

    boundary = receipt["editor_boundary"]
    expected_boundary = {
        "reader_only": False,
        "editor_controls_enabled": True,
        "native_save_pub_claimed": False,
        "source_pub_immutable": True,
    }
    if boundary != expected_boundary:
        raise AssertionError("trial package must expose Editor controls without widening native Save PUB claims")

    if not all(receipt["runtime_smoke"].values()):
        raise AssertionError("trial package runtime smoke is incomplete")

    contents = receipt["package_contents"]
    expected_contents = {
        "chaptera_executable_present": True,
        "trial_readme_present": True,
        "trial_readme_contract": "chaptera.editor-live-trial-readme.v1",
        "installer_included": False,
        "code_signing_claimed": False,
        "auto_update_claimed": False,
    }
    if contents != expected_contents:
        raise AssertionError("portable trial package contents/claims are outside the bounded V0 contract")

    if any(receipt["privacy"].values()):
        raise AssertionError("public package receipt admits private document/customer values")

    return {
        "receipt_version": receipt["receipt_version"],
        "product": receipt["product"],
        "platform": build["platform"],
        "arch": build["arch"],
        "package_kind": build["package_kind"],
        "fixture_kind": receipt["fixture_kind"],
        "editor_controls_enabled": True,
        "reader_only": False,
        "source_pub_immutable": True,
        "runtime_loop_complete": True,
        "portable_zip_bounded": True,
        "source_free_receipt": True,
    }


def validate_receipt(receipt):
    validate_schema(receipt)
    return validate_semantics(receipt)


def main():
    if len(sys.argv) != 2:
        print("usage: validate_editor_live_trial_package_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    path = pathlib.Path(sys.argv[1])
    receipt = json.loads(path.read_text(encoding="utf-8"))
    print(json.dumps(validate_receipt(receipt), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
