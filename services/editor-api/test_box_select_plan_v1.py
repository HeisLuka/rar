#!/usr/bin/env python3
import unittest

from box_select_plan_v1 import (
    MAX_SAFE_EMU,
    BoxSelectCandidateV1,
    BoxSelectPlanError,
    PointEmu,
    RectEmu,
    plan_box_select_v1,
)


class BoxSelectPlanV1Tests(unittest.TestCase):
    def test_drag_direction_normalizes_to_same_result(self):
        candidates = (
            BoxSelectCandidateV1("node:b", RectEmu(20, 20, 10, 10)),
            BoxSelectCandidateV1("node:a", RectEmu(0, 0, 10, 10)),
        )
        a = plan_box_select_v1(
            start=PointEmu(-5, -5),
            end=PointEmu(35, 35),
            candidates=candidates,
        )
        b = plan_box_select_v1(
            start=PointEmu(35, 35),
            end=PointEmu(-5, -5),
            candidates=tuple(reversed(candidates)),
        )
        self.assertEqual(a, b)
        self.assertEqual(("node:a", "node:b"), a.selected_node_ids)

    def test_full_containment_only_boundary_equality_counts(self):
        candidates = (
            BoxSelectCandidateV1("exact", RectEmu(0, 0, 100, 100)),
            BoxSelectCandidateV1("inside", RectEmu(1, 1, 98, 98)),
            BoxSelectCandidateV1("intersects", RectEmu(90, 90, 20, 20)),
            BoxSelectCandidateV1("outside", RectEmu(101, 0, 10, 10)),
        )
        plan = plan_box_select_v1(
            start=PointEmu(0, 0),
            end=PointEmu(100, 100),
            candidates=candidates,
        )
        self.assertEqual(("exact", "inside"), plan.selected_node_ids)

    def test_zero_area_returns_no_change(self):
        for start, end in (
            (PointEmu(10, 10), PointEmu(10, 30)),
            (PointEmu(10, 10), PointEmu(30, 10)),
            (PointEmu(10, 10), PointEmu(10, 10)),
        ):
            with self.subTest(start=start, end=end):
                plan = plan_box_select_v1(start=start, end=end, candidates=())
                self.assertEqual("no_change", plan.status)
                self.assertIsNone(plan.selection_bounds)
                self.assertEqual((), plan.selected_node_ids)

    def test_empty_nonzero_box_is_explicit(self):
        plan = plan_box_select_v1(
            start=PointEmu(0, 0),
            end=PointEmu(10, 10),
            candidates=(BoxSelectCandidateV1("x", RectEmu(20, 20, 5, 5)),),
        )
        self.assertEqual("empty", plan.status)
        self.assertEqual((), plan.selected_node_ids)

    def test_duplicate_ids_invalid_bounds_and_overflow_fail_closed(self):
        with self.assertRaisesRegex(BoxSelectPlanError, "unique"):
            plan_box_select_v1(
                start=PointEmu(0, 0),
                end=PointEmu(10, 10),
                candidates=(
                    BoxSelectCandidateV1("x", RectEmu(1, 1, 2, 2)),
                    BoxSelectCandidateV1("x", RectEmu(3, 3, 2, 2)),
                ),
            )
        with self.assertRaises(BoxSelectPlanError):
            plan_box_select_v1(
                start=PointEmu(0, 0),
                end=PointEmu(10, 10),
                candidates=(BoxSelectCandidateV1("x", RectEmu(1, 1, 0, 2)),),
            )
        with self.assertRaisesRegex(BoxSelectPlanError, "safe EMU"):
            plan_box_select_v1(
                start=PointEmu(0, 0),
                end=PointEmu(10, 10),
                candidates=(BoxSelectCandidateV1("x", RectEmu(MAX_SAFE_EMU - 1, 0, 10, 10)),),
            )

    def test_signed_coordinates_are_valid_document_space(self):
        plan = plan_box_select_v1(
            start=PointEmu(-100, -100),
            end=PointEmu(100, 100),
            candidates=(BoxSelectCandidateV1("x", RectEmu(-50, -50, 25, 25)),),
        )
        self.assertEqual(("x",), plan.selected_node_ids)


if __name__ == "__main__":
    unittest.main()
