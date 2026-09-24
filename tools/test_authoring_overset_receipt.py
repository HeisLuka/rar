#!/usr/bin/env python3
import copy
import unittest

from validate_authoring_overset_receipt import validate_schema, validate_semantics

SOURCE_HASH = "a" * 64
STORY = "10000000-0000-4000-8000-000000000001"
FRAME = "20000000-0000-4000-8000-000000000001"
BEFORE = "sha256:" + "b" * 64
AFTER = "sha256:" + "c" * 64
ENV = "sha256:" + "d" * 64


def state(story_hash, scalar_count, layout_state):
    return {
        "story_hash": story_hash,
        "scalar_count": scalar_count,
        "state": layout_state,
        "reason_code": None,
        "environment_authoritative": True,
        "layout_environment_hash": ENV,
    }


def valid_receipt():
    baseline = state(BEFORE, 20, "fits")
    accepted = state(AFTER, 200, "overset")
    return {
        "receipt_version": "chaptera.authoring-overset-receipt.v1",
        "producer": {
            "implementation": "chaptera-private-authoring-layout",
            "commit_or_build": "deadbeef",
            "core_integration": True,
        },
        "source_hash": SOURCE_HASH,
        "story_id": STORY,
        "frame_node_id": FRAME,
        "canonical_edit": {
            "before_story_hash": BEFORE,
            "after_story_hash": AFTER,
            "before_scalar_count": 20,
            "after_scalar_count": 200,
        },
        "states": {
            "baseline": copy.deepcopy(baseline),
            "accepted": copy.deepcopy(accepted),
            "undo": copy.deepcopy(baseline),
            "redo": copy.deepcopy(accepted),
            "replay": copy.deepcopy(accepted),
        },
        "layout_unknown_probe": {
            "story_hash": AFTER,
            "scalar_count": 200,
            "state": "layout_unknown",
            "reason_code": "layout.environment_unavailable",
            "environment_authoritative": False,
            "layout_environment_hash": None,
        },
        "output_probe": {
            "editable_export_story_hash": AFTER,
            "fixed_output_outcome": "explicit_overset_loss",
            "overset_state_explicit": True,
        },
        "invariants": {
            "canonical_story_truncated": False,
            "autofit_mutation_count": 0,
            "source_write_count": 0,
            "linked_frame_flow_used": False,
            "host_font_fallback_used": False,
            "raw_story_text_emitted": False,
        },
    }


class AuthoringOversetReceiptTests(unittest.TestCase):
    def test_valid_receipt_is_admitted(self):
        receipt = valid_receipt()
        validate_schema(receipt)
        validate_semantics(receipt)

    def test_raw_story_text_field_fails_closed(self):
        receipt = valid_receipt()
        receipt["canonical_edit"]["after_text"] = "secret"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_accepted_story_must_equal_canonical_edit(self):
        receipt = valid_receipt()
        receipt["states"]["accepted"]["story_hash"] = BEFORE
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_undo_must_restore_exact_state(self):
        receipt = valid_receipt()
        receipt["states"]["undo"]["state"] = "overset"
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_replay_must_equal_post_edit_state(self):
        receipt = valid_receipt()
        receipt["states"]["replay"]["scalar_count"] -= 1
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_non_authoritative_environment_cannot_claim_fits(self):
        receipt = valid_receipt()
        receipt["states"]["baseline"]["environment_authoritative"] = False
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_layout_unknown_requires_reason(self):
        receipt = valid_receipt()
        receipt["layout_unknown_probe"]["reason_code"] = None
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_editable_export_must_keep_full_story(self):
        receipt = valid_receipt()
        receipt["output_probe"]["editable_export_story_hash"] = BEFORE
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_truncation_is_forbidden(self):
        receipt = valid_receipt()
        receipt["invariants"]["canonical_story_truncated"] = True
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_host_font_fallback_is_forbidden(self):
        receipt = valid_receipt()
        receipt["invariants"]["host_font_fallback_used"] = True
        with self.assertRaises(AssertionError):
            validate_schema(receipt)


if __name__ == "__main__":
    unittest.main()
