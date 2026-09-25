#!/usr/bin/env python3
import json
import os
from pathlib import Path
import unittest

from resolved_text_caret_map_v1 import (
    GDEF_FORMAT1_AUTHORITY_SOURCE,
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    ResolvedTextCaretMapError,
    build_resolved_text_caret_map_v1,
    caret_map_to_dict_v1,
    internal_caret_stop_from_shaping_authority_v1,
    resolve_story_position_v1,
    selection_geometry_v1,
)


RECEIPT_PATH = os.environ.get("INTERNAL_CARET_RECEIPT")


@unittest.skipUnless(RECEIPT_PATH, "cross-language GDEF receipt path not configured")
class InternalCaretAuthorityCrossLanguageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.receipt = json.loads(Path(RECEIPT_PATH).read_text())

    def test_real_fi_gdef_authority_reaches_canonical_story_caret(self):
        receipt = self.receipt
        self.assertEqual(
            "chaptera.internal-caret-authority.gdef-format1.v1",
            receipt["protocol_version"],
        )
        self.assertEqual(1, len(receipt["stops"]))
        self.assertEqual([], receipt["unsupported"])
        stop = receipt["stops"][0]
        self.assertEqual(101, stop["scalar_boundary"])
        self.assertEqual(7, stop["glyph_id"])
        self.assertEqual(332, stop["gdef_coordinate_font_units"])
        self.assertEqual(50_597, stop["caret_x_emu"])
        self.assertEqual(GDEF_FORMAT1_AUTHORITY_SOURCE, stop["authority_source"])

        page_run_origin = 1_000_000
        frame_run_origin = 50_000
        internal = internal_caret_stop_from_shaping_authority_v1(
            stop,
            page_run_origin_x_emu=page_run_origin,
            frame_run_origin_x_emu=frame_run_origin,
        )

        advance = receipt["shaped"]["total_x_advance"]
        cluster = ResolvedClusterV1(
            start_scalar=100,
            end_scalar=102,
            page_x_start_emu=page_run_origin,
            page_x_end_emu=page_run_origin + advance,
            frame_x_start_emu=frame_run_origin,
            frame_x_end_emu=frame_run_origin + advance,
            internal_caret_stops=(internal,),
        )
        line = ResolvedLineFragmentV1(
            story_id="story:fi",
            page_id="page:1",
            frame_id="frame:1",
            line_id="line:1",
            flow_ordinal=0,
            previous_line_id=None,
            next_line_id=None,
            page_y_top_emu=2_000_000,
            page_y_bottom_emu=2_020_000,
            frame_y_top_emu=0,
            frame_y_bottom_emu=20_000,
            clusters=(cluster,),
        )
        caret_map = build_resolved_text_caret_map_v1(
            layout_revision_id="layout:fi:gdef",
            story_id="story:fi",
            story_scalar_len=102,
            lines=(line,),
        )
        resolved = resolve_story_position_v1(
            caret_map=caret_map,
            scalar_boundary=101,
        )
        self.assertEqual(1_050_597, resolved.page_x_emu)
        self.assertEqual(100_597, resolved.frame_x_emu)
        self.assertEqual(( "internal", ), resolved.affinities)
        self.assertEqual(GDEF_FORMAT1_AUTHORITY_SOURCE, resolved.authority_source)

        selection = selection_geometry_v1(
            caret_map=caret_map,
            start_scalar=100,
            end_scalar=101,
        )
        self.assertEqual("complete", selection.coverage_state)
        self.assertEqual(1_050_597, selection.rectangles[0].page_x_end_emu)

        encoded = caret_map_to_dict_v1(caret_map)
        internal_rows = [
            row
            for row in encoded["caret_stops"]
            if row["scalar_boundary"] == 101
        ]
        self.assertEqual(1, len(internal_rows))
        self.assertEqual(
            GDEF_FORMAT1_AUTHORITY_SOURCE,
            internal_rows[0]["authority_source"],
        )

    def test_canonical_adapter_rejects_unadmitted_authority_source(self):
        stop = dict(self.receipt["stops"][0])
        stop["authority_source"] = "heuristic_advance_division"
        with self.assertRaises(ResolvedTextCaretMapError) as caught:
            internal_caret_stop_from_shaping_authority_v1(
                stop,
                page_run_origin_x_emu=0,
                frame_run_origin_x_emu=0,
            )
        self.assertEqual(
            "unsupported_internal_caret_authority",
            caught.exception.code,
        )


if __name__ == "__main__":
    unittest.main()
