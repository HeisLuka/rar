#!/usr/bin/env python3
import unittest

from authored_group_geometry_v1 import RectEmu
from resize_snap_v1 import ResizeSnapError, plan_resize_snap_v1
from snap_index_v1 import SnapCandidateV1, SnapIndexError, SnapIndexV1


def index(*candidates):
    return SnapIndexV1.build(tuple(candidates))


class SnapIndexV1Tests(unittest.TestCase):
    def test_old_stable_tie_prefers_page_edge_before_object_edge(self):
        snap = index(
            SnapCandidateV1("x", 0, "min", "page_edge", None),
            SnapCandidateV1("x", 10, "min", "object_edge", "peer:1"),
        )
        match = snap.best_axis_match_v1(
            axis="x",
            moving_anchors=(("min", 5),),
            tolerance_emu=5,
        )
        self.assertIsNotNone(match)
        self.assertEqual(-5, match.correction_emu)
        self.assertEqual("page_edge", match.feedback.target_kind)

    def test_excluded_peer_is_not_a_snap_authority(self):
        snap = index(
            SnapCandidateV1("x", 105, "min", "object_edge", "peer:1"),
        )
        match = snap.best_axis_match_v1(
            axis="x",
            moving_anchors=(("max", 103),),
            tolerance_emu=5,
            excluded_node_ids=("peer:1",),
        )
        self.assertIsNone(match)

    def test_invalid_candidate_geometry_and_unprojected_kind_fail_closed(self):
        with self.assertRaises(SnapIndexError):
            index(SnapCandidateV1("x", 0, "center", "page_edge", None))
        with self.assertRaises(SnapIndexError):
            SnapIndexV1.build(
                (
                    SnapCandidateV1(
                        "x",
                        9_007_199_254_740_992,
                        "min",
                        "page_edge",
                        None,
                    ),
                )
            )

    def test_negative_tolerance_fails_closed(self):
        snap = index(SnapCandidateV1("x", 0, "min", "page_edge", None))
        with self.assertRaises(SnapIndexError):
            snap.best_axis_match_v1(
                axis="x",
                moving_anchors=(("min", 5),),
                tolerance_emu=-1,
            )


class ResizeSnapV1Tests(unittest.TestCase):
    def test_east_handle_snaps_only_moving_right_edge(self):
        snap = index(SnapCandidateV1("x", 205, "min", "object_edge", "peer:1"))
        ordinary = RectEmu(100, 50, 101, 80)
        plan = plan_resize_snap_v1(
            handle="e",
            ordinary_target=ordinary,
            snap_index=snap,
            tolerance_emu=5,
        )
        self.assertEqual(RectEmu(100, 50, 105, 80), plan.corrected_target)
        self.assertEqual(100, plan.corrected_target.x)
        self.assertEqual(205, plan.corrected_target.right)
        self.assertEqual("max", plan.x_feedback.moving_anchor)

    def test_west_handle_preserves_fixed_right_edge_exactly(self):
        snap = index(SnapCandidateV1("x", 95, "max", "object_edge", "peer:1"))
        ordinary = RectEmu(98, 50, 102, 80)
        plan = plan_resize_snap_v1(
            handle="w",
            ordinary_target=ordinary,
            snap_index=snap,
            tolerance_emu=5,
        )
        self.assertEqual(RectEmu(95, 50, 105, 80), plan.corrected_target)
        self.assertEqual(200, ordinary.right)
        self.assertEqual(200, plan.corrected_target.right)

    def test_corner_resolves_x_and_y_independently(self):
        snap = index(
            SnapCandidateV1("x", 300, "max", "page_edge", None),
            SnapCandidateV1("y", 400, "max", "page_edge", None),
        )
        ordinary = RectEmu(100, 200, 197, 196)
        plan = plan_resize_snap_v1(
            handle="se",
            ordinary_target=ordinary,
            snap_index=snap,
            tolerance_emu=5,
        )
        self.assertEqual(RectEmu(100, 200, 200, 200), plan.corrected_target)
        self.assertEqual("x", plan.x_feedback.axis)
        self.assertEqual("y", plan.y_feedback.axis)

    def test_resize_center_is_never_a_moving_snap_anchor(self):
        snap = index(SnapCandidateV1("x", 150, "center", "page_center", None))
        ordinary = RectEmu(100, 50, 100, 80)
        plan = plan_resize_snap_v1(
            handle="e",
            ordinary_target=ordinary,
            snap_index=snap,
            tolerance_emu=5,
        )
        # Rect center equals 150 exactly, but the east moving edge is 200.
        self.assertEqual(ordinary, plan.corrected_target)
        self.assertIsNone(plan.x_feedback)

    def test_target_center_is_allowed_when_moving_edge_reaches_it(self):
        snap = index(SnapCandidateV1("x", 200, "center", "page_center", None))
        ordinary = RectEmu(100, 50, 98, 80)
        plan = plan_resize_snap_v1(
            handle="e",
            ordinary_target=ordinary,
            snap_index=snap,
            tolerance_emu=5,
        )
        self.assertEqual(100, plan.corrected_target.width)
        self.assertEqual("center", plan.x_feedback.target_anchor)
        self.assertEqual("max", plan.x_feedback.moving_anchor)

    def test_excluded_selection_member_is_not_used_as_peer(self):
        snap = index(SnapCandidateV1("x", 200, "min", "object_edge", "selected:2"))
        ordinary = RectEmu(100, 50, 98, 80)
        plan = plan_resize_snap_v1(
            handle="e",
            ordinary_target=ordinary,
            snap_index=snap,
            tolerance_emu=5,
            excluded_node_ids=("selected:2",),
        )
        self.assertEqual(ordinary, plan.corrected_target)

    def test_non_positive_result_fails_closed(self):
        snap = index(SnapCandidateV1("x", 106, "min", "object_edge", "peer:1"))
        ordinary = RectEmu(100, 0, 5, 10)
        with self.assertRaisesRegex(ResizeSnapError, "positive"):
            plan_resize_snap_v1(
                handle="w",
                ordinary_target=ordinary,
                snap_index=snap,
                tolerance_emu=10,
            )

    def test_no_candidate_within_tolerance_returns_ordinary_target(self):
        snap = index(SnapCandidateV1("x", 300, "max", "page_edge", None))
        ordinary = RectEmu(100, 50, 100, 80)
        plan = plan_resize_snap_v1(
            handle="e",
            ordinary_target=ordinary,
            snap_index=snap,
            tolerance_emu=5,
        )
        self.assertEqual(ordinary, plan.corrected_target)
        self.assertIsNone(plan.x_feedback)
        self.assertIsNone(plan.y_feedback)


if __name__ == "__main__":
    unittest.main()
