#!/usr/bin/env python3
import copy
import unittest

from scene_v1 import finalize_snapshot, hash_id
from test_revision_producer_receipt_schema import valid_receipt
from validate_browser_acceptance_receipt import (
    expected_history_revision_ids,
    expected_scene_snapshot_ids,
    validate_schema,
    validate_semantics,
)


def initial_scene(revision):
    operation = revision["accepted"]["canonical_operation"]
    scene = {
        "protocol_version": "chaptera.scene.v1",
        "document_id": revision["document_id"],
        "source_hash": revision["source_hash"],
        "revision_id": revision["baseline"]["revision_id"],
        "snapshot_id": "sha256:" + "0" * 64,
        "layout_environment": {
            "environment_id": hash_id({"environment": "test"}),
            "engine_revision": "test-engine",
            "font_set_fingerprint": hash_id({"fonts": "test"}),
            "resource_fingerprint": hash_id({"resources": "test"}),
        },
        "stacking_fidelity": "unknown",
        "pages": [{
            "page_id": "20000000-0000-4000-8000-000000000001",
            "order": 0,
            "width_emu": 9144000,
            "height_emu": 6858000,
        }],
        "nodes": [{
            "node_id": operation["node_id"],
            "page_id": "20000000-0000-4000-8000-000000000001",
            "parent_node_id": None,
            "kind": "unknown",
            "bounds": copy.deepcopy(operation["before"]),
            "transform": {"a": "1", "b": "0", "c": "0", "d": "1", "tx": 0, "ty": 0},
            "z_order": None,
            "paint_order": None,
            "paint_id": None,
            "resource_id": None,
        }],
        "stories": [],
        "story_frames": [],
        "paints": [],
        "resources": [],
        "diagnostics": [],
        "capabilities": [],
        "fidelity": {"state": "supported", "reasons": []},
    }
    return finalize_snapshot(scene)


def valid_browser_receipt(revision, scene):
    operation = revision["accepted"]["canonical_operation"]
    undo_revision_id, redo_revision_id = expected_history_revision_ids(revision)
    snapshots = expected_scene_snapshot_ids(revision, scene)
    source_hash = revision["source_hash"]
    return {
        "receipt_version": "chaptera.web-acceptance-receipt.v1",
        "receipt_class": "real_pub_browser",
        "repository_commit_sha": "1" * 40,
        "browser": {"name": "chromium", "version": "test", "headless": True},
        "fixture": {
            "name": "SampleNewsletter.pub",
            "sha256": source_hash,
            "byte_len": 291840,
            "family": "mature-0x2c",
        },
        "initial_revision_id": revision["baseline"]["revision_id"],
        "selected_node_id": operation["node_id"],
        "before_rect": copy.deepcopy(operation["before"]),
        "after_rect": copy.deepcopy(operation["after"]),
        "client_operation_id": revision["request"]["client_operation_id"],
        "accepted_revision_id": revision["accepted"]["revision_id"],
        "undo_revision_id": undo_revision_id,
        "redo_revision_id": redo_revision_id,
        "scene_snapshot_ids": snapshots,
        "export": {
            "target": "idml",
            "sha256": "b" * 64,
            "geometry_reflects_edit": True,
        },
        "source_immutability": {
            "before_sha256": source_hash,
            "after_sha256": source_hash,
            "unchanged": True,
        },
        "capability_state_visible": True,
        "loss_state_visible": True,
        "native_pub_save_enabled": False,
        "semantic_assertions": {
            "pointermove_created_no_revision": True,
            "one_release_one_move": True,
            "node_id_stable": True,
            "server_before_state_won": True,
            "reopen_independent_of_browser_memory": True,
            "stale_base_not_silently_accepted": True,
        },
    }


class BrowserAcceptanceBindingTests(unittest.TestCase):
    def setUp(self):
        self.revision = valid_receipt()
        self.scene = initial_scene(self.revision)
        self.browser = valid_browser_receipt(self.revision, self.scene)

    def test_current_bounded_chain_is_fully_bound(self):
        validate_schema(self.browser)
        validate_semantics(self.browser, self.revision, self.scene)

    def test_selected_node_must_match_canonical_move(self):
        self.browser["selected_node_id"] = "30000000-0000-4000-8000-000000000099"
        with self.assertRaises(AssertionError):
            validate_semantics(self.browser, self.revision, self.scene)

    def test_before_rect_must_match_server_state(self):
        self.browser["before_rect"]["x"] += 1
        with self.assertRaises(AssertionError):
            validate_semantics(self.browser, self.revision, self.scene)

    def test_client_operation_id_must_match_revision_receipt(self):
        self.browser["client_operation_id"] = "90000000-0000-4000-8000-000000000099"
        with self.assertRaises(AssertionError):
            validate_semantics(self.browser, self.revision, self.scene)

    def test_undo_revision_is_derived_not_self_asserted(self):
        self.browser["undo_revision_id"] = "sha256:" + "c" * 64
        with self.assertRaises(AssertionError):
            validate_semantics(self.browser, self.revision, self.scene)

    def test_accepted_snapshot_is_derived_from_canonical_move(self):
        self.browser["scene_snapshot_ids"]["accepted"] = "sha256:" + "d" * 64
        with self.assertRaises(AssertionError):
            validate_semantics(self.browser, self.revision, self.scene)

    def test_reopen_must_reproduce_final_redo_scene(self):
        self.browser["scene_snapshot_ids"]["reopen"] = "sha256:" + "e" * 64
        with self.assertRaises(AssertionError):
            validate_semantics(self.browser, self.revision, self.scene)


if __name__ == "__main__":
    unittest.main()
