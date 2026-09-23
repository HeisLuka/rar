#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "research" / "auth-wrap" / "v1"
SCHEMA = BASE / "native-receipt.schema.json"

KNOWN_FIXTURES = {
    "apache_poi_sample_newsletter": "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf",
    "aspose_computer_classes_flyer": "f934cdd537d42e0448052ad6617db089b6523db8f7f2cfb392b2679a1a691991",
}


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("AUTH-WRAP native receipt schema validation failed\n" + detail)


def _capture_complete(capture):
    return all(capture.values())


def validate_semantics(receipt):
    fixture = receipt["fixture"]
    expected_sha = KNOWN_FIXTURES[fixture["lineage"]]
    if fixture["sha256"] != expected_sha:
        raise AssertionError("AUTH-WRAP receipt fixture SHA does not match the pinned public lineage")

    family_a = receipt["family_a"]
    if not all([
        family_a["one_0x47_target_ref_changed"],
        family_a["0x46_count_kept_consistent"],
        family_a["geometry_unchanged_before_open"],
        family_a["textwrap_fopt_unchanged_before_open"],
    ]):
        raise AssertionError("family A did not isolate the 0x47 conflict")

    family_b = receipt["family_b"]
    if not all([
        family_b["one_wrap_property_changed_via_com"],
        family_b["geometry_unchanged_before_save"],
        family_b["old_0x47_left_untouched_before_save"],
    ]):
        raise AssertionError("family B did not isolate the TextWrap/FOPT conflict")

    if any(receipt["privacy"].values()):
        raise AssertionError("AUTH-WRAP public receipt admits private runtime/document values")

    native = receipt["receipt_kind"] == "native_observation"
    closure = receipt["scope"]["closure_candidate"]
    env = receipt["environment"]
    conclusion = receipt["conclusion"]

    if native:
        if not env["reset_provider_receipt_verified"]:
            raise AssertionError("native AUTH-WRAP evidence requires a verified reset-provider receipt")
        if not env["cold_restore_pair_verified"]:
            raise AssertionError("native AUTH-WRAP evidence requires the verified cold-restore pair")
        if env["environment_fingerprint_sha256"] is None:
            raise AssertionError("native AUTH-WRAP evidence requires an environment fingerprint")

    if closure:
        if not native:
            raise AssertionError("only native observation can be a closure candidate")
        if not _capture_complete(family_a["capture"]):
            raise AssertionError("closure candidate requires complete family A Open/Save/reopen capture")
        if not _capture_complete(family_b["capture"]):
            raise AssertionError("closure candidate requires complete family B Open/Save/reopen capture")
        if "not_observed" in {
            family_a["post_save_0x47_outcome"],
            family_a["post_save_fopt_outcome"],
            family_a["layout_outcome"],
            family_b["post_save_0x47_outcome"],
            family_b["post_save_fopt_outcome"],
            family_b["layout_outcome"],
        }:
            raise AssertionError("closure candidate cannot contain not_observed outcomes")
        if not conclusion["both_conflict_families_executed"]:
            raise AssertionError("closure candidate requires both conflict families")
        if not conclusion["save_reopen_evidence_complete"]:
            raise AssertionError("closure candidate requires complete save/reopen evidence")
        if conclusion["authority_class"] == "inconclusive":
            raise AssertionError("closure candidate cannot remain inconclusive")
        if conclusion["needs_additional_native_discriminator"]:
            raise AssertionError("closure candidate cannot still require another native discriminator")
    else:
        if conclusion["authority_class"] != "inconclusive" and not native:
            raise AssertionError("synthetic/non-native receipt cannot assert a wrap authority conclusion")

    return {
        "receipt_version": receipt["receipt_version"],
        "task_id": receipt["task_id"],
        "receipt_kind": receipt["receipt_kind"],
        "fixture_lineage": fixture["lineage"],
        "publisher_version": env["publisher_version"],
        "publisher_build": env["publisher_build"],
        "reset_verified": env["reset_provider_receipt_verified"],
        "cold_restore_pair_verified": env["cold_restore_pair_verified"],
        "family_a_complete": _capture_complete(family_a["capture"]),
        "family_b_complete": _capture_complete(family_b["capture"]),
        "authority_class": conclusion["authority_class"],
        "closure_candidate": closure,
        "source_free_receipt": True,
    }


def validate_receipt(receipt):
    validate_schema(receipt)
    return validate_semantics(receipt)


def main():
    if len(sys.argv) != 2:
        print("usage: validate_auth_wrap_native_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    receipt = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
    print(json.dumps(validate_receipt(receipt), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
