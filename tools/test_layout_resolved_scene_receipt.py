#!/usr/bin/env python3
import copy
import unittest

from validate_layout_resolved_scene_receipt import validate_schema, validate_semantics

SOURCE_HASH = "a" * 64
NODE = "10000000-0000-4000-8000-000000000001"
PAGE = "20000000-0000-4000-8000-000000000001"
BASE_SNAPSHOT = "sha256:" + "b" * 64
MOVED_SNAPSHOT = "sha256:" + "c" * 64
STABLE_SURFACES = "sha256:" + "d" * 64
STABLE_ORIGINS = "sha256:" + "e" * 64
GEOMETRY_HASH = "sha256:" + "f" * 64

BEFORE = {"x": 1000, "y": 2000, "width": 3000, "height": 4000}
AFTER = {"x": 128000, "y": 256000, "width": 3000, "height": 4000}


def state(snapshot_id, bounds):
    return {
        "scene_snapshot_id": snapshot_id,
        "node_id": NODE,
        "page_id": PAGE,
        "origin_node_id": NODE,
        "bounds": copy.deepcopy(bounds),
        "surface_page_ids_hash": STABLE_SURFACES,
        "origin_mapping_hash": STABLE_ORIGINS,
        "projection_input": "current_resolved_graph",
    }


def valid_receipt():
    return {
        "receipt_version": "chaptera.layout-resolved-scene-receipt.v1",
        "producer": {
            "implementation": "chaptera-private-layout-adapter",
            "commit_or_build": "deadbeef",
            "core_integration": True,
        },
        "source_hash": SOURCE_HASH,
        "scene_protocol_version": "chaptera.scene.v1",
        "projection_api": "resolved_graph_adapter",
        "canonical_move": {
            "node_id": NODE,
            "page_id": PAGE,
            "before": copy.deepcopy(BEFORE),
            "after": copy.deepcopy(AFTER),
        },
        "states": {
            "baseline": state(BASE_SNAPSHOT, BEFORE),
            "accepted": state(MOVED_SNAPSHOT, AFTER),
            "undo": state(BASE_SNAPSHOT, BEFORE),
            "redo": state(MOVED_SNAPSHOT, AFTER),
            "replay": state(MOVED_SNAPSHOT, AFTER),
        },
        "baseline_equivalence": {
            "viewer_geometry_hash": GEOMETRY_HASH,
            "adapter_geometry_hash": GEOMETRY_HASH,
            "viewer_surface_hash": STABLE_SURFACES,
            "adapter_surface_hash": STABLE_SURFACES,
            "viewer_origin_mapping_hash": STABLE_ORIGINS,
            "adapter_origin_mapping_hash": STABLE_ORIGINS,
        },
        "projection_context_state": {
            "projection_context_hash": "sha256:" + "1" * 64,
            "master_relation_count": 0,
            "cmo_relation_count": 0,
            "carried_outside_editor_project": True,
            "cmo_layout_consumed": False,
        },
        "invariants": {
            "source_reparse_after_edit_count": 0,
            "viewer_private_mapping_used": False,
            "browser_layout_authoritative": False,
            "second_geometry_model_created": False,
            "context_extension_seam_present": True,
            "graph_only_wrapper_is_empty_context": True,
            "projection_context_carried_outside_editor_project": True,
            "unsupported_cmo_layout_deferred": True,
            "raw_source_bytes_emitted": False,
        },
    }


class LayoutResolvedSceneReceiptTests(unittest.TestCase):
    def test_valid_receipt_is_admitted(self):
        receipt = valid_receipt()
        validate_schema(receipt)
        validate_semantics(receipt)

    def test_private_source_field_fails_closed(self):
        receipt = valid_receipt()
        receipt["private_checkout_path"] = "/home/private/core"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_accepted_must_match_canonical_after(self):
        receipt = valid_receipt()
        receipt["states"]["accepted"]["bounds"]["x"] += 1
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_undo_must_restore_baseline(self):
        receipt = valid_receipt()
        receipt["states"]["undo"]["bounds"]["y"] += 1
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_redo_and_replay_must_equal_accepted_snapshot(self):
        receipt = valid_receipt()
        receipt["states"]["replay"]["scene_snapshot_id"] = BASE_SNAPSHOT
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_node_identity_must_remain_stable(self):
        receipt = valid_receipt()
        receipt["states"]["redo"]["origin_node_id"] = "10000000-0000-4000-8000-000000000099"
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_page_surface_set_must_remain_stable(self):
        receipt = valid_receipt()
        receipt["states"]["accepted"]["surface_page_ids_hash"] = "sha256:" + "1" * 64
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_baseline_viewer_adapter_equivalence_is_required(self):
        receipt = valid_receipt()
        receipt["baseline_equivalence"]["adapter_geometry_hash"] = "sha256:" + "2" * 64
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_post_edit_source_reparse_is_forbidden(self):
        receipt = valid_receipt()
        receipt["invariants"]["source_reparse_after_edit_count"] = 1
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_context_extension_seam_is_required(self):
        receipt = valid_receipt()
        receipt["invariants"]["context_extension_seam_present"] = False
        with self.assertRaises(AssertionError):
            validate_schema(receipt)


if __name__ == "__main__":
    unittest.main()
