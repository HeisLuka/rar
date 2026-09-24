#!/usr/bin/env python3
import copy
import unittest

from validate_viewer_geometry_receipt import validate_schema


def minimal_receipt():
    return {
        "schema_version": "0.1",
        "document": {
            "schema_version": "0.1",
            "source": {
                "format": "publisher",
                "format_version": "0x2c",
                "source_hash": "a" * 64,
                "byte_len": 123,
            },
            "pages": [{
                "index": 1,
                "id": "10000000-0000-4000-8000-000000000001",
                "width_emu": 914400,
                "height_emu": 914400,
            }],
            "stories": [{
                "id": "20000000-0000-4000-8000-000000000001",
                "text": "public fixture text",
            }],
            "diagnostics": [],
        },
        "scene": {
            "environment": {
                "engine_revision": "viewer-geometry-v0.1",
                "font_set_fingerprint": "fonts:not-consumed:geometry-only",
                "resource_fingerprint": "resources:not-consumed:geometry-only",
            },
            "surfaces": [{
                "origin": "10000000-0000-4000-8000-000000000001",
                "size": {"width": 914400, "height": 914400},
                "bleed": None,
                "margins": None,
            }],
            "nodes": [{
                "origin": "30000000-0000-4000-8000-000000000001",
                "parent_origin": "10000000-0000-4000-8000-000000000001",
                "bounds": {"x": 0, "y": 0, "width": 127000, "height": 254000},
                "transform": {"a": "1", "b": "0", "c": "0", "d": "1", "tx": 0, "ty": 0},
            }],
            "origin_mapping": [{
                "authoring_origin": "30000000-0000-4000-8000-000000000001",
                "resolved_node_origin": "30000000-0000-4000-8000-000000000001",
            }],
            "diagnostics": [],
        },
        "paints": [],
        "story_frames": [],
        "images": [{
            "resource_id": "40000000-0000-4000-8000-000000000001",
            "mime": "image/png",
            "node_ids": ["30000000-0000-4000-8000-000000000001"],
        }],
    }


class ViewerGeometryReceiptSchemaTests(unittest.TestCase):
    def test_current_source_free_shape_is_admitted(self):
        validate_schema(minimal_receipt())

    def test_unknown_top_level_private_field_fails_closed(self):
        receipt = minimal_receipt()
        receipt["private_checkout_path"] = "/home/private/yab"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_unknown_nested_source_field_fails_closed(self):
        receipt = minimal_receipt()
        receipt["document"]["source"]["local_path"] = "C:\\Users\\private\\fixture.pub"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_serialized_image_bytes_are_forbidden(self):
        receipt = minimal_receipt()
        receipt["images"][0]["bytes"] = "iVBORw0KGgo="
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_unreviewed_fetch_handle_is_forbidden_in_v0_1_receipt(self):
        receipt = minimal_receipt()
        receipt["images"][0]["fetch_handle"] = "file:///private/cache/image.png"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)


if __name__ == "__main__":
    unittest.main()
