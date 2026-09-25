#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "packages" / "protocol" / "layout-resolved-scene" / "v1" / "producer-receipt.schema.json"


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("resolved-scene receipt schema validation failed\n" + detail)


def _same_identity(left, right):
    for key in ("node_id", "page_id", "origin_node_id", "surface_page_ids_hash", "origin_mapping_hash"):
        if left[key] != right[key]:
            raise AssertionError(f"state identity changed for {key}")


def validate_semantics(receipt):
    move = receipt["canonical_move"]
    states = receipt["states"]
    baseline = states["baseline"]
    accepted = states["accepted"]
    undo = states["undo"]
    redo = states["redo"]
    replay = states["replay"]

    for state in states.values():
        if state["node_id"] != move["node_id"] or state["page_id"] != move["page_id"]:
            raise AssertionError("state target identity differs from canonical move")
        if state["origin_node_id"] != move["node_id"]:
            raise AssertionError("scene origin must remain the canonical node id")

    if baseline["bounds"] != move["before"]:
        raise AssertionError("baseline Scene bounds differ from canonical MoveNode.before")
    if accepted["bounds"] != move["after"]:
        raise AssertionError("accepted Scene bounds differ from canonical MoveNode.after")
    if undo["bounds"] != move["before"]:
        raise AssertionError("undo Scene bounds do not restore baseline")
    if redo["bounds"] != move["after"]:
        raise AssertionError("redo Scene bounds do not restore accepted state")
    if replay["bounds"] != move["after"]:
        raise AssertionError("fresh replay Scene bounds differ from accepted state")

    for other in (accepted, undo, redo, replay):
        _same_identity(baseline, other)

    if accepted["scene_snapshot_id"] != redo["scene_snapshot_id"]:
        raise AssertionError("redo snapshot must equal accepted snapshot")
    if accepted["scene_snapshot_id"] != replay["scene_snapshot_id"]:
        raise AssertionError("fresh replay snapshot must equal accepted snapshot")
    if baseline["scene_snapshot_id"] != undo["scene_snapshot_id"]:
        raise AssertionError("undo snapshot must equal baseline snapshot")

    eq = receipt["baseline_equivalence"]
    pairs = (
        ("viewer_geometry_hash", "adapter_geometry_hash"),
        ("viewer_surface_hash", "adapter_surface_hash"),
        ("viewer_origin_mapping_hash", "adapter_origin_mapping_hash"),
    )
    for left, right in pairs:
        if eq[left] != eq[right]:
            raise AssertionError(f"baseline Viewer/adapter equivalence failed for {left}")

    if move["before"] == move["after"]:
        raise AssertionError("canonical move must actually change geometry")

    context = receipt["projection_context_state"]
    if not context["carried_outside_editor_project"]:
        raise AssertionError("projection context must remain outside EditorProject")
    if context["cmo_layout_consumed"]:
        raise AssertionError("unsupported Cmo layout must remain deferred")
    if context["master_relation_count"] < 0 or context["cmo_relation_count"] < 0:
        raise AssertionError("projection context relation counts must be non-negative")

    inv = receipt["invariants"]
    if inv["source_reparse_after_edit_count"] != 0:
        raise AssertionError("post-edit Scene must not reparse immutable source")
    if inv["viewer_private_mapping_used"]:
        raise AssertionError("Viewer-private mapping must not remain authoritative")
    if inv["browser_layout_authoritative"]:
        raise AssertionError("browser layout must not become authoritative")
    if inv["second_geometry_model_created"]:
        raise AssertionError("task must not introduce a second geometry model")
    if not inv["context_extension_seam_present"]:
        raise AssertionError("resolved projection context extension seam is required")
    if not inv["graph_only_wrapper_is_empty_context"]:
        raise AssertionError("graph-only convenience path must equal empty context")
    if not inv["projection_context_carried_outside_editor_project"]:
        raise AssertionError("projection context must be carried outside EditorProject")
    if not inv["unsupported_cmo_layout_deferred"]:
        raise AssertionError("unsupported Cmo layout must stay deferred")
    if inv["raw_source_bytes_emitted"]:
        raise AssertionError("public receipt must not emit source bytes")

    return {
        "receipt_kind": receipt["receipt_version"],
        "source_hash": receipt["source_hash"],
        "scene_protocol_version": receipt["scene_protocol_version"],
        "projection_api": receipt["projection_api"],
        "node_id": move["node_id"],
        "page_id": move["page_id"],
        "baseline_restored_by_undo": True,
        "accepted_restored_by_redo": True,
        "accepted_equal_to_replay": True,
        "stable_origin_mapping": True,
        "viewer_adapter_baseline_equivalent": True,
        "source_reparse_after_edit_count": 0,
        "context_extension_seam_present": True,
        "projection_context_hash": context["projection_context_hash"],
        "master_relation_count": context["master_relation_count"],
        "cmo_relation_count": context["cmo_relation_count"],
    }


def main():
    if len(sys.argv) != 2:
        print("usage: validate_layout_resolved_scene_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    receipt = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
    validate_schema(receipt)
    print(json.dumps(validate_semantics(receipt), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
