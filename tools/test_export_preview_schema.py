#!/usr/bin/env python3
import copy
import unittest

from validate_export_preview import (
    validate_schema,
    validate_semantics,
    validate_scene_binding,
)

DOCUMENT_ID = "9c2f8d5e-3f1b-4a57-9d40-6a7e2c11b001"
SOURCE_HASH = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
REVISION_ID = "sha256:" + "1" * 64


def valid_preview():
    return {
        "protocol_version": "chaptera.export-preview.v1",
        "report_schema_version": "0.1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "revision_id": REVISION_ID,
        "target": {
            "format": "idml",
            "adapter_version": "idml-v0.1",
            "profile": "bounded-editable",
            "schema_fence": "legacy-dom-7",
        },
        "conversion_fence_sha256": None,
        "can_serialize": True,
        "counts": {
            "preserved": 2,
            "approximated": 1,
            "flattened": 0,
            "rasterized": 0,
            "unsupported": 0,
            "blocking": 0,
        },
        "items": [
            {
                "feature": "page.geometry",
                "origin": "11111111-1111-4111-8111-111111111111",
                "property_path": "page.size",
                "disposition": "preserved",
            },
            {
                "feature": "node.geometry.position",
                "origin": "21111111-1111-4111-8111-111111111111",
                "property_path": "node.bounds.position",
                "disposition": "preserved",
            },
            {
                "feature": "story.typography",
                "origin": "31111111-1111-4111-8111-111111111111",
                "property_path": "story.runs",
                "disposition": "approximated",
                "loss_kind": "approximated",
                "severity": "visual",
                "reversible": True,
                "code": "export.story_typography.approximated",
            },
        ],
    }


class ExportPreviewContractTests(unittest.TestCase):
    def test_source_free_current_preview_is_valid(self):
        preview = valid_preview()
        validate_schema(preview)
        summary = validate_semantics(preview)
        self.assertTrue(summary["can_serialize"])
        self.assertFalse(summary["source_label_present"])

    def test_source_label_is_not_admitted(self):
        preview = valid_preview()
        preview["source_label"] = "/private/work/SampleNewsletter.pub"
        with self.assertRaises(AssertionError):
            validate_schema(preview)

    def test_unknown_private_field_fails_closed(self):
        preview = valid_preview()
        preview["items"][0]["parser_carrier"] = {"offset": 42}
        with self.assertRaises(AssertionError):
            validate_schema(preview)

    def test_preserved_item_cannot_claim_loss(self):
        preview = valid_preview()
        preview["items"][0]["loss_kind"] = "approximated"
        with self.assertRaises(AssertionError):
            validate_semantics(preview)

    def test_lossy_item_requires_complete_loss_tuple(self):
        preview = valid_preview()
        del preview["items"][-1]["code"]
        with self.assertRaises(AssertionError):
            validate_semantics(preview)

    def test_counts_are_derived_not_asserted(self):
        preview = valid_preview()
        preview["counts"]["preserved"] += 1
        with self.assertRaises(AssertionError):
            validate_semantics(preview)

    def test_blocking_loss_makes_export_unserializable(self):
        preview = valid_preview()
        item = preview["items"][-1]
        item["severity"] = "blocking"
        preview["counts"]["blocking"] = 1
        preview["can_serialize"] = False
        validate_semantics(preview)

        preview["can_serialize"] = True
        with self.assertRaises(AssertionError):
            validate_semantics(preview)

    def test_preview_must_bind_current_scene_identity(self):
        preview = valid_preview()
        scene = {
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "revision_id": REVISION_ID,
        }
        validate_scene_binding(preview, scene)
        scene["revision_id"] = "sha256:" + "2" * 64
        with self.assertRaises(AssertionError):
            validate_scene_binding(preview, scene)


if __name__ == "__main__":
    unittest.main()
