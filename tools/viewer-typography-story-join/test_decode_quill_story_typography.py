#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import pathlib
import sys
import types
import unittest

SCRIPT = pathlib.Path(__file__).with_name("decode_quill_story_typography.py")

# These contract tests exercise only pure byte/range helpers. Keep them
# source-free and independent from the optional olefile runtime dependency.
fake_olefile = types.ModuleType("olefile")
fake_olefile.OleFileIO = object
sys.modules.setdefault("olefile", fake_olefile)

spec = importlib.util.spec_from_file_location("viewer_typography_decoder", SCRIPT)
assert spec is not None and spec.loader is not None
decoder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(decoder)


class BoundedTypographyDecoderTests(unittest.TestCase):
    def test_unknown_fixed_block_width_fails_closed(self) -> None:
        with self.assertRaisesRegex(
            decoder.DecodeError,
            "physical width is not proven",
        ):
            decoder.parse_block(bytes([0x0C, 0x33]), 0, 2)

    def test_checked_in_real_viewer_receipt_keeps_bounded_typography_census(self) -> None:
        repo_root = pathlib.Path(__file__).resolve().parents[2]
        receipt_path = (
            repo_root
            / "apps"
            / "web"
            / "acceptance"
            / "receipts"
            / "viewer-geometry.real.json"
        )
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))

        self.assertEqual(
            receipt["document"]["source"]["source_hash"],
            "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf",
        )
        self.assertEqual(len(receipt["text_fragments"]), 58)
        self.assertEqual(len(receipt["typography_runs"]), 12)
        self.assertTrue(
            any(
                run["source_font_name"] == "Rockwell Condensed"
                and run["text_size_emu"] == 24 * decoder.EMU_PER_POINT
                for run in receipt["typography_runs"]
            )
        )
        self.assertTrue(
            all(
                run["render_disposition"]
                == "source_identity_known_render_fallback"
                for run in receipt["typography_runs"]
            )
        )

    def test_fdpc_offsets_must_be_monotone_in_stored_order(self) -> None:
        styles = [
            {
                "text_offset": 516,
                "descriptor_ordinal": 2,
                "style_ordinal": 0,
            },
            {
                "text_offset": 514,
                "descriptor_ordinal": 2,
                "style_ordinal": 1,
            },
        ]
        story_catalog = {
            "text_offset": 512,
            "text_end": 520,
            "total_utf16_units": 4,
            "stories": [],
        }

        with self.assertRaisesRegex(
            decoder.DecodeError,
            "regress in stored order",
        ):
            decoder.materialize_ranges(styles, story_catalog)


if __name__ == "__main__":
    unittest.main()
