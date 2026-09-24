#!/usr/bin/env python3
import unittest

from authored_group_geometry_v1 import RectEmu
from resize_constraint_snap_compose_v1 import (
    ResizeConstraintSnapComposeError,
    plan_resize_constraint_snap_compose_v1,
)
from resize_constraints_v1 import ResizeModifierMaskV1, plan_resize_constraint_v1
from snap_index_v1 import SnapCandidateV1, SnapIndexV1


BASE = RectEmu(100, 100, 100, 100)


def idx(*candidates):
    return SnapIndexV1.build(tuple(candidates))


def compose(raw, modifiers, snap, handle="se", tolerance=5):
    return plan_resize_constraint_snap_compose_v1(
        base_rect=BASE,
        handle=handle,
        raw_target_rect=raw,
        modifiers=modifiers,
        snap_index=snap,
        tolerance_emu=tolerance,
    )


class ResizeConstraintSnapComposeV1Tests(unittest.TestCase):
    def test_ctrl_east_snap_mirrors_across_exact_base_center(self):
        result = compose(
            RectEmu(100, 100, 118, 100),
            ResizeModifierMaskV1(centered=True),
            idx(SnapCandidateV1("x", 220, "max", "page_edge", None)),
            handle="e",
        )
        self.assertEqual(RectEmu(80, 100, 140, 100), result.final_rect)
        self.assertEqual(300, result.final_rect.x + result.final_rect.right)
        self.assertEqual(220, result.final_rect.right)
        self.assertEqual(1, result.snapped_axis_count)

    def test_ctrl_corner_snaps_axes_independently_and_preserves_both_centers(self):
        result = compose(
            RectEmu(100, 100, 118, 129),
            ResizeModifierMaskV1(centered=True),
            idx(
                SnapCandidateV1("x", 220, "max", "page_edge", None),
                SnapCandidateV1("y", 230, "max", "page_edge", None),
            ),
        )
        self.assertEqual(RectEmu(80, 70, 140, 160), result.final_rect)
        self.assertEqual(300, result.final_rect.x + result.final_rect.right)
        self.assertEqual(300, result.final_rect.y + result.final_rect.bottom)
        self.assertEqual(2, result.snapped_axis_count)

    def test_shift_corner_x_driven_snap_derives_y_from_aspect(self):
        result = compose(
            RectEmu(100, 100, 118, 105),
            ResizeModifierMaskV1(aspect_lock=True),
            idx(SnapCandidateV1("x", 220, "max", "page_edge", None)),
        )
        self.assertEqual(RectEmu(100, 100, 120, 120), result.final_rect)
        self.assertEqual("x_driven", result.source)
        self.assertEqual(1, result.snapped_axis_count)

    def test_shift_corner_y_driven_snap_derives_x_from_aspect(self):
        result = compose(
            RectEmu(100, 100, 105, 118),
            ResizeModifierMaskV1(aspect_lock=True),
            idx(SnapCandidateV1("y", 220, "max", "page_edge", None)),
        )
        self.assertEqual(RectEmu(100, 100, 120, 120), result.final_rect)
        self.assertEqual("y_driven", result.source)

    def test_common_scale_two_axis_proposal_beats_single_axis(self):
        result = compose(
            RectEmu(100, 100, 118, 119),
            ResizeModifierMaskV1(aspect_lock=True),
            idx(
                SnapCandidateV1("x", 220, "max", "page_edge", None),
                SnapCandidateV1("y", 220, "max", "page_edge", None),
            ),
        )
        self.assertEqual(RectEmu(100, 100, 120, 120), result.final_rect)
        self.assertEqual(2, result.snapped_axis_count)
        self.assertIsNotNone(result.x_feedback)
        self.assertIsNotNone(result.y_feedback)

    def test_no_valid_proposal_retains_unsnapped_constrained_geometry(self):
        raw = RectEmu(100, 100, 118, 119)
        modifiers = ResizeModifierMaskV1(aspect_lock=True)
        expected = plan_resize_constraint_v1(
            base_rect=BASE,
            handle="se",
            raw_target_rect=raw,
            modifiers=modifiers,
        ).constrained_rect
        result = compose(
            raw,
            modifiers,
            idx(
                SnapCandidateV1("x", 300, "max", "page_edge", None),
                SnapCandidateV1("y", 300, "max", "page_edge", None),
            ),
        )
        self.assertEqual(expected, result.final_rect)
        self.assertEqual("fallback", result.source)
        self.assertEqual(0, result.snapped_axis_count)

    def test_ctrl_shift_preserves_centers_and_shared_aspect(self):
        result = compose(
            RectEmu(100, 100, 118, 119),
            ResizeModifierMaskV1(centered=True, aspect_lock=True),
            idx(
                SnapCandidateV1("x", 220, "max", "page_edge", None),
                SnapCandidateV1("y", 220, "max", "page_edge", None),
            ),
        )
        self.assertEqual(RectEmu(80, 80, 140, 140), result.final_rect)
        self.assertEqual(300, result.final_rect.x + result.final_rect.right)
        self.assertEqual(300, result.final_rect.y + result.final_rect.bottom)
        self.assertEqual(result.final_rect.width, result.final_rect.height)
        self.assertEqual(2, result.snapped_axis_count)

    def test_shift_edge_reuses_ordinary_snap_because_aspect_is_inert(self):
        result = compose(
            RectEmu(100, 100, 118, 100),
            ResizeModifierMaskV1(aspect_lock=True),
            idx(SnapCandidateV1("x", 220, "max", "page_edge", None)),
            handle="e",
        )
        self.assertEqual(RectEmu(100, 100, 120, 100), result.final_rect)
        self.assertEqual("ordinary", result.source)

    def test_stable_tie_break_reuses_page_before_object_policy(self):
        result = compose(
            RectEmu(100, 100, 118, 118),
            ResizeModifierMaskV1(aspect_lock=True),
            idx(
                SnapCandidateV1("x", 220, "max", "page_edge", None),
                SnapCandidateV1("x", 216, "min", "object_edge", "peer:1"),
            ),
        )
        self.assertEqual(220, result.final_rect.right)
        self.assertEqual("page_edge", result.x_feedback.target_kind)

    def test_empty_modifier_mask_is_not_this_compositors_job(self):
        with self.assertRaisesRegex(
            ResizeConstraintSnapComposeError,
            "non-empty",
        ):
            compose(
                RectEmu(100, 100, 118, 118),
                ResizeModifierMaskV1(),
                idx(),
            )


if __name__ == "__main__":
    unittest.main()
