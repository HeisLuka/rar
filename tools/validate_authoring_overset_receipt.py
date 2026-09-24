#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "packages" / "protocol" / "authoring-overset" / "v1" / "producer-receipt.schema.json"


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("authoring overset receipt schema validation failed\n" + detail)


def validate_layout_state(state, label):
    if state["state"] in ("fits", "overset"):
        if not state["environment_authoritative"]:
            raise AssertionError(f"{label}: fits/overset requires authoritative environment")
        if state["layout_environment_hash"] is None:
            raise AssertionError(f"{label}: authoritative state requires environment hash")
        if state["reason_code"] is not None:
            raise AssertionError(f"{label}: fits/overset must not carry layout_unknown reason")
    else:
        if state["environment_authoritative"]:
            raise AssertionError(f"{label}: layout_unknown cannot claim authoritative environment")
        if state["reason_code"] is None:
            raise AssertionError(f"{label}: layout_unknown requires stable reason code")


def validate_semantics(receipt):
    edit = receipt["canonical_edit"]
    states = receipt["states"]
    baseline = states["baseline"]
    accepted = states["accepted"]
    undo = states["undo"]
    redo = states["redo"]
    replay = states["replay"]

    for label, state in states.items():
        validate_layout_state(state, label)
    validate_layout_state(receipt["layout_unknown_probe"], "layout_unknown_probe")

    if baseline["story_hash"] != edit["before_story_hash"]:
        raise AssertionError("baseline Story differs from canonical edit before-state")
    if baseline["scalar_count"] != edit["before_scalar_count"]:
        raise AssertionError("baseline scalar count differs from canonical edit before-state")
    if accepted["story_hash"] != edit["after_story_hash"]:
        raise AssertionError("accepted Story differs from canonical edit after-state")
    if accepted["scalar_count"] != edit["after_scalar_count"]:
        raise AssertionError("accepted scalar count differs from canonical edit after-state")

    if undo != baseline:
        raise AssertionError("undo must restore exact Story/layout state")
    if redo != accepted:
        raise AssertionError("redo must restore exact post-edit Story/layout state")
    if replay != accepted:
        raise AssertionError("fresh EditorProject replay must re-derive exact post-edit state")

    if baseline["state"] != "fits":
        raise AssertionError("bounded regression baseline must fit")
    if accepted["state"] != "overset":
        raise AssertionError("bounded regression extended Story must be overset")

    unknown = receipt["layout_unknown_probe"]
    if unknown["state"] != "layout_unknown":
        raise AssertionError("missing/mismatched environment probe must be layout_unknown")

    output = receipt["output_probe"]
    if output["editable_export_story_hash"] != accepted["story_hash"]:
        raise AssertionError("editable export did not preserve full canonical overset Story")
    if not output["overset_state_explicit"]:
        raise AssertionError("fixed output must expose overset/loss state")

    inv = receipt["invariants"]
    if inv["canonical_story_truncated"]:
        raise AssertionError("canonical Story must never be truncated to fit")
    if inv["autofit_mutation_count"] != 0:
        raise AssertionError("AutoFit/copyfit mutation is outside this task")
    if inv["source_write_count"] != 0:
        raise AssertionError("native PUB write is outside this task")
    if inv["linked_frame_flow_used"]:
        raise AssertionError("linked-frame flow is outside one-frame overset slice")
    if inv["host_font_fallback_used"]:
        raise AssertionError("host-font fallback must not turn unknown layout into fits/overset")
    if inv["raw_story_text_emitted"]:
        raise AssertionError("public receipt must not emit raw Story text")

    return {
        "receipt_kind": receipt["receipt_version"],
        "source_hash": receipt["source_hash"],
        "story_id": receipt["story_id"],
        "frame_node_id": receipt["frame_node_id"],
        "baseline_state": baseline["state"],
        "accepted_state": accepted["state"],
        "undo_exact": True,
        "redo_exact": True,
        "replay_exact": True,
        "layout_unknown_probe": True,
        "editable_export_preserves_story": True,
        "fixed_output_overset_explicit": True,
        "canonical_story_truncated": False,
    }


def main():
    if len(sys.argv) != 2:
        print("usage: validate_authoring_overset_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    receipt = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
    validate_schema(receipt)
    print(json.dumps(validate_semantics(receipt), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
