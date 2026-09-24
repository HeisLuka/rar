#!/usr/bin/env python3
import unittest

from offpage_staging_v1 import (
    MAX_SAFE_EMU,
    OffpageStagingError,
    RectEmu,
    admit_authored_offpage_node,
    classify_placement,
    move_bounds_preserving_size,
    printable_intersection,
    work_area_bounds,
)


PAGE = RectEmu(0, 0, 10_000, 8_000)
MARGIN = 2_000


class OffpageStagingV1Tests(unittest.TestCase):
    def admit(self, bounds):
        return admit_authored_offpage_node(
            node_id="node:authored:1",
            page_id="page:1",
            parent_id="page:1",
            author_created=True,
            source_backed=False,
            transform_identity=True,
            bounds=bounds,
            page_bounds=PAGE,
            margin_emu=MARGIN,
        )

    def test_work_area_expands_around_page_but_printable_page_is_unchanged(self):
        self.assertEqual(RectEmu(-2_000, -2_000, 14_000, 12_000), work_area_bounds(PAGE, MARGIN))
        self.assertEqual(PAGE, PAGE)

    def test_signed_partial_and_full_offpage_geometry_remains_page_owned(self):
        partial = self.admit(RectEmu(-500, 100, 1_000, 500))
        full = self.admit(RectEmu(-1_500, 100, 500, 500))
        self.assertEqual("partially_offpage", partial.placement)
        self.assertEqual("fully_offpage", full.placement)
        self.assertEqual("page:1", partial.page_id)
        self.assertEqual("page:1", full.page_id)

    def test_move_into_margin_and_back_preserves_size(self):
        before = RectEmu(100, 200, 600, 400)
        staged = move_bounds_preserving_size(before, -1_000, 200)
        self.assertEqual(RectEmu(-1_000, 200, 600, 400), staged)
        self.assertEqual("fully_offpage", self.admit(staged).placement)
        restored = move_bounds_preserving_size(staged, 100, 200)
        self.assertEqual(before, restored)
        self.assertEqual("inside_page", self.admit(restored).placement)

    def test_source_backed_nested_or_transformed_nodes_fail_closed(self):
        base = dict(
            node_id="node:1",
            page_id="page:1",
            parent_id="page:1",
            author_created=True,
            source_backed=False,
            transform_identity=True,
            bounds=RectEmu(-500, 0, 500, 500),
            page_bounds=PAGE,
            margin_emu=MARGIN,
        )
        mutations = [
            {"author_created": False, "source_backed": True},
            {"parent_id": "group:1"},
            {"transform_identity": False},
        ]
        for mutation in mutations:
            args = dict(base)
            args.update(mutation)
            with self.subTest(mutation=mutation):
                with self.assertRaises(OffpageStagingError):
                    admit_authored_offpage_node(**args)

    def test_bounded_work_area_rejects_far_away_object(self):
        with self.assertRaisesRegex(OffpageStagingError, "outside bounded"):
            self.admit(RectEmu(-10_000, 0, 500, 500))

    def test_fixed_output_clips_to_printable_page(self):
        self.assertEqual(
            RectEmu(0, 100, 500, 500),
            printable_intersection(RectEmu(-500, 100, 1_000, 500), PAGE),
        )
        self.assertIsNone(
            printable_intersection(RectEmu(-1_500, 100, 500, 500), PAGE)
        )
        self.assertEqual(
            RectEmu(100, 100, 500, 500),
            printable_intersection(RectEmu(100, 100, 500, 500), PAGE),
        )

    def test_classification_has_no_scratch_area_ownership_state(self):
        self.assertEqual("inside_page", classify_placement(RectEmu(1, 1, 10, 10), PAGE))
        self.assertEqual("partially_offpage", classify_placement(RectEmu(-1, 1, 10, 10), PAGE))
        self.assertEqual("fully_offpage", classify_placement(RectEmu(-20, 1, 10, 10), PAGE))

    def test_overflow_and_noop_fail_closed(self):
        with self.assertRaises(OffpageStagingError):
            work_area_bounds(RectEmu(MAX_SAFE_EMU - 10, 0, 10, 10), 100)
        with self.assertRaisesRegex(OffpageStagingError, "no-op"):
            move_bounds_preserving_size(RectEmu(0, 0, 10, 10), 0, 0)


if __name__ == "__main__":
    unittest.main()
