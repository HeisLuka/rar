#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import pathlib
import unittest

SCRIPT = pathlib.Path(__file__).with_name("decode_default_inheritance.py")
spec = importlib.util.spec_from_file_location("default_inherit", SCRIPT)
assert spec is not None and spec.loader is not None
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


def story_catalog() -> dict:
    return {
        "text_offset": 100,
        "text_end": 120,
        "total_utf16_units": 10,
        "stories": [
            {
                "index": 0,
                "syid": 7,
                "global_start_utf16": 0,
                "global_end_utf16": 10,
            }
        ],
    }


def fdpc_range(
    start: int,
    end: int,
    *,
    font_indices: list[int] | None = None,
    font_names: list[str] | None = None,
    sizes: list[int] | None = None,
) -> dict:
    return {
        "global_start_utf16": start,
        "global_end_utf16": end,
        "font_indices": font_indices or [],
        "font_names": font_names or [],
        "text_size_emu": sizes or [],
    }


def fdpp_range(start: int, end: int, style: int = 0, source: str = "explicit_fdpp_0x19") -> dict:
    return {
        "global_start_utf16": start,
        "global_end_utf16": end,
        "selected_style_index": style,
        "selector_source": source,
    }


def default_row(style: int, *, font: str | None, size_emu: int | None) -> dict:
    return {
        "logical_style_index": style,
        "font_unambiguous": font is not None,
        "font_names": [font] if font is not None else [],
        "text_size_unambiguous": size_emu is not None,
        "text_size_emu": [size_emu] if size_emu is not None else [],
    }


class DefaultInheritanceTests(unittest.TestCase):
    def test_explicit_font_wins_and_default_fills_only_missing_size(self) -> None:
        rows = mod.materialize_effective_segments(
            story_catalog(),
            [
                fdpc_range(
                    0,
                    10,
                    font_indices=[3],
                    font_names=["Explicit Face"],
                    sizes=[],
                )
            ],
            [fdpp_range(0, 10, 2)],
            [default_row(2, font="Default Face", size_emu=12 * 12_700)],
        )
        self.assertEqual(len(rows), 1)
        row = rows[0]
        self.assertEqual(row["effective_font_name"], "Explicit Face")
        self.assertEqual(row["font_source"], "explicit_fdpc")
        self.assertEqual(row["effective_size_emu"], 12 * 12_700)
        self.assertEqual(row["size_source"], "inherited_stsh1")
        self.assertTrue(row["inherited_any"])

    def test_explicit_size_wins_and_default_fills_only_missing_font(self) -> None:
        rows = mod.materialize_effective_segments(
            story_catalog(),
            [fdpc_range(0, 10, sizes=[18 * 12_700])],
            [fdpp_range(0, 10, 1)],
            [default_row(1, font="Inherited Face", size_emu=10 * 12_700)],
        )
        row = rows[0]
        self.assertEqual(row["effective_font_name"], "Inherited Face")
        self.assertEqual(row["font_source"], "inherited_stsh1")
        self.assertEqual(row["effective_size_emu"], 18 * 12_700)
        self.assertEqual(row["size_source"], "explicit_fdpc")

    def test_ambiguous_explicit_property_is_not_reclassified_as_missing(self) -> None:
        rows = mod.materialize_effective_segments(
            story_catalog(),
            [
                fdpc_range(
                    0,
                    10,
                    font_indices=[1, 2],
                    font_names=["Face A", "Face B"],
                    sizes=[14 * 12_700],
                )
            ],
            [fdpp_range(0, 10, 0)],
            [default_row(0, font="Default Face", size_emu=10 * 12_700)],
        )
        row = rows[0]
        self.assertTrue(row["explicit_font_present"])
        self.assertFalse(row["explicit_font_unambiguous"])
        self.assertIsNone(row["effective_font_name"])
        self.assertIsNone(row["font_source"])
        self.assertEqual(row["size_source"], "explicit_fdpc")
        self.assertFalse(row["after_complete_effective_font_and_size"])

    def test_absent_selector_is_bounded_prior_evidence_style_zero(self) -> None:
        styles = [
            {
                "text_offset": 120,
                "descriptor_ordinal": 5,
                "style_ordinal": 0,
                "default_style_index_raw": [],
            }
        ]
        rows = mod.materialize_fdpp_ranges(styles, story_catalog())
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["selected_style_index"], 0)
        self.assertEqual(
            rows[0]["selector_source"],
            "implicit_zero_from_prior_evidence",
        )

    def test_fdpp_stored_order_regression_fails_closed(self) -> None:
        styles = [
            {
                "text_offset": 116,
                "descriptor_ordinal": 5,
                "style_ordinal": 0,
                "default_style_index_raw": [0],
            },
            {
                "text_offset": 112,
                "descriptor_ordinal": 5,
                "style_ordinal": 1,
                "default_style_index_raw": [0],
            },
        ]
        with self.assertRaisesRegex(mod.donor.DecodeError, "regress in stored order"):
            mod.materialize_fdpp_ranges(styles, story_catalog())


if __name__ == "__main__":
    unittest.main()
