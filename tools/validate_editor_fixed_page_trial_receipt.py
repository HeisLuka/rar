#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "product" / "editor-fixed-page-trial" / "v1"
SCHEMA = BASE / "acceptance-receipt.schema.json"
PUBLIC_SURROGATE_SHA = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("fixed-page trial acceptance schema validation failed\n" + detail)


def validate_semantics(receipt):
    phase = receipt["trial_phase"]
    source = receipt["source_fixture"]

    if phase == "public_surrogate":
        if source["public_lineage"] != "apache_poi_sample_newsletter":
            raise AssertionError("public surrogate must use the pinned SampleNewsletter lineage")
        if source["public_sha256"] != PUBLIC_SURROGATE_SHA:
            raise AssertionError("public surrogate SHA does not match the pinned fixture")
    elif phase == "private_customer":
        if source["public_lineage"] is not None or source["public_sha256"] is not None:
            raise AssertionError("private-customer public receipt must redact source identity")

    for name in ("package", "resize_node", "replace_image_ui", "replace_image_export"):
        if not receipt["evidence_chain"][name]["validated"]:
            raise AssertionError(f"upstream evidence is not validated: {name}")

    replace_ui = receipt["evidence_chain"]["replace_image_ui"]
    replace_export = receipt["evidence_chain"]["replace_image_export"]
    if replace_ui["replacement_binding_id"] != replace_export["replacement_binding_id"]:
        raise AssertionError("final trial mixes ReplaceImage UI/export evidence from different replacement sessions")

    wrap = receipt["evidence_chain"]["auth_wrap"]
    if not wrap["validated"]:
        raise AssertionError("AUTH-WRAP receipt is not validated")
    if not wrap["native_observation"] or not wrap["closure_candidate"]:
        raise AssertionError("fixed-page trial requires a real native AUTH-WRAP closure candidate")
    if wrap["authority_class"] == "inconclusive":
        raise AssertionError("fixed-page trial cannot close while wrap authority is inconclusive")

    if not all(receipt["user_path"].values()):
        missing = [key for key, value in receipt["user_path"].items() if not value]
        raise AssertionError(f"fixed-page trial user path is incomplete: {missing}")

    result = receipt["export_result"]
    required_export = (
        "edited_story_present",
        "moved_geometry_present",
        "resized_geometry_present",
        "replacement_image_exact",
        "bounded_wrap_result_preserved",
        "approximations_explicit",
    )
    if not all(result[key] for key in required_export):
        raise AssertionError("edited export does not preserve the bounded trial state")
    if result["blocking_loss_count"] != 0:
        raise AssertionError("fixed-page supported trial cannot close with blocking export loss")

    safety = receipt["safety"]
    expected_safety = {
        "source_pub_immutable": True,
        "native_save_pub_claimed": False,
        "unsupported_mutation_fails_closed": True,
        "no_silent_source_image_fallback": True,
        "no_hidden_network_upload": True,
    }
    if safety != expected_safety:
        raise AssertionError("trial safety/product boundary changed")

    if any(receipt["privacy"].values()):
        raise AssertionError("public trial receipt admits private/customer content")

    return {
        "receipt_version": receipt["receipt_version"],
        "receipt_kind": receipt["receipt_kind"],
        "trial_phase": phase,
        "export_target": result["target"],
        "upstream_evidence_validated": True,
        "replacement_identity_bound_end_to_end": True,
        "native_wrap_authority_closed": True,
        "full_user_path_complete": True,
        "blocking_loss_count": 0,
        "source_pub_immutable": True,
        "native_save_pub_not_claimed": True,
        "source_free_receipt": True,
    }


def validate_receipt(receipt):
    validate_schema(receipt)
    return validate_semantics(receipt)


def main():
    if len(sys.argv) != 2:
        print("usage: validate_editor_fixed_page_trial_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    receipt = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
    print(json.dumps(validate_receipt(receipt), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
