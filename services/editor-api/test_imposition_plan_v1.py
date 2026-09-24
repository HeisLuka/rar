#!/usr/bin/env python3
import unittest

from imposition_plan_v1 import (
    ImpositionPlanError,
    fixture_booklet_08p_v1,
    fixture_edu_label_16up_v1,
    plan_booklet_v1,
    plan_distinct_n_up_v1,
    plan_duplex_pairs_v1,
    plan_one_page_per_sheet_v1,
    plan_repeated_n_up_v1,
)


class ImpositionPlanV1Tests(unittest.TestCase):
    def test_one_page_per_sheet_keeps_logical_pages_distinct(self):
        plan = plan_one_page_per_sheet_v1(logical_page_ids=("p1", "p2", "p3"))
        self.assertEqual("one_page_per_sheet", plan.mode)
        self.assertEqual(("p1", "p2", "p3"), plan.logical_page_ids)
        self.assertEqual(3, len(plan.sheets))
        self.assertEqual(
            ["p1", "p2", "p3"],
            [sheet.front.placements[0].logical_page_id for sheet in plan.sheets],
        )

    def test_repeated_copies_reference_one_logical_page(self):
        plan = plan_repeated_n_up_v1(
            logical_page_id="label",
            copies=18,
            rows=4,
            columns=4,
        )
        self.assertEqual(("label",), plan.logical_page_ids)
        self.assertEqual(2, len(plan.sheets))
        self.assertEqual(18, plan.placement_count)
        self.assertEqual(
            16,
            len(plan.sheets[0].front.placements),
        )
        self.assertEqual(2, len(plan.sheets[1].front.placements))
        self.assertTrue(
            all(
                placement.logical_page_id == "label"
                for sheet in plan.sheets
                for placement in sheet.front.placements
            )
        )

    def test_distinct_n_up_is_row_major_and_spills_deterministically(self):
        pages = tuple(f"p{i}" for i in range(1, 7))
        plan = plan_distinct_n_up_v1(
            logical_page_ids=pages,
            rows=2,
            columns=2,
        )
        self.assertEqual(2, len(plan.sheets))
        first = plan.sheets[0].front.placements
        self.assertEqual(["p1", "p2", "p3", "p4"], [x.logical_page_id for x in first])
        self.assertEqual(
            [(0,0), (0,1), (1,0), (1,1)],
            [(x.row, x.column) for x in first],
        )
        self.assertEqual(
            ["p5", "p6"],
            [x.logical_page_id for x in plan.sheets[1].front.placements],
        )

    def test_duplex_pairing_preserves_explicit_flip_and_back_orientation(self):
        plan = plan_duplex_pairs_v1(
            front_page_ids=("f1", "f2"),
            back_page_ids=("b1", "b2"),
            flip="long_edge",
            back_rotation_quarter_turns=2,
        )
        self.assertEqual(2, len(plan.sheets))
        self.assertTrue(all(sheet.duplex_flip == "long_edge" for sheet in plan.sheets))
        self.assertEqual(
            [2, 2],
            [sheet.back.placements[0].rotation_quarter_turns for sheet in plan.sheets],
        )
        self.assertEqual(
            [("f1", "b1"), ("f2", "b2")],
            [
                (
                    sheet.front.placements[0].logical_page_id,
                    sheet.back.placements[0].logical_page_id,
                )
                for sheet in plan.sheets
            ],
        )

    def test_fix_edu_label_16up_semantics(self):
        plan = fixture_edu_label_16up_v1()
        self.assertEqual("repeated_n_up", plan.mode)
        self.assertEqual(1, len(plan.logical_page_ids))
        self.assertEqual(1, len(plan.sheets))
        self.assertEqual(16, plan.placement_count)
        placements = plan.sheets[0].front.placements
        self.assertEqual(4, plan.sheets[0].front.rows)
        self.assertEqual(4, plan.sheets[0].front.columns)
        self.assertEqual(
            [(r, c) for r in range(4) for c in range(4)],
            [(x.row, x.column) for x in placements],
        )
        self.assertEqual(
            {"FIX-EDU-LABEL-16UP-01:page:1"},
            {x.logical_page_id for x in placements},
        )

    def test_fix_booklet_08p_semantics(self):
        plan = fixture_booklet_08p_v1()
        self.assertEqual("booklet", plan.mode)
        self.assertEqual(2, len(plan.sheets))
        self.assertEqual(8, plan.placement_count)
        pairs = []
        for sheet in plan.sheets:
            pairs.append(
                (
                    [x.logical_page_id.rsplit(":", 1)[-1] for x in sheet.front.placements],
                    [x.logical_page_id.rsplit(":", 1)[-1] for x in sheet.back.placements],
                )
            )
            self.assertEqual("short_edge", sheet.duplex_flip)
        self.assertEqual(
            [
                (["8", "1"], ["2", "7"]),
                (["6", "3"], ["4", "5"]),
            ],
            pairs,
        )

    def test_booklet_general_four_page_signature(self):
        plan = plan_booklet_v1(logical_page_ids=("1", "2", "3", "4"))
        self.assertEqual(1, len(plan.sheets))
        self.assertEqual(
            ["4", "1"],
            [x.logical_page_id for x in plan.sheets[0].front.placements],
        )
        self.assertEqual(
            ["2", "3"],
            [x.logical_page_id for x in plan.sheets[0].back.placements],
        )

    def test_invalid_ambiguous_or_oversized_inputs_fail_closed(self):
        with self.assertRaises(ImpositionPlanError):
            plan_booklet_v1(logical_page_ids=("1", "2", "3"))
        with self.assertRaises(ImpositionPlanError):
            plan_distinct_n_up_v1(
                logical_page_ids=("p1", "p1"),
                rows=1,
                columns=2,
            )
        with self.assertRaises(ImpositionPlanError):
            plan_repeated_n_up_v1(
                logical_page_id="p1",
                copies=0,
                rows=1,
                columns=1,
            )
        with self.assertRaises(ImpositionPlanError):
            plan_duplex_pairs_v1(
                front_page_ids=("f1",),
                back_page_ids=("b1", "b2"),
                flip="long_edge",
            )


if __name__ == "__main__":
    unittest.main()
