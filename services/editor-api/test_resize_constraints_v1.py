import unittest

from authored_group_geometry_v1 import MAX_SAFE_EMU, RectEmu
from resize_constraints_v1 import (
    ResizeConstraintError,
    ResizeModifierMaskV1,
    plan_resize_constraint_v1,
)


class ResizeConstraintV1Tests(unittest.TestCase):
    def test_no_modifier_east_preserves_base_left_and_inactive_y(self):
        base = RectEmu(100, 200, 80, 40)
        raw = RectEmu(100, 999, 120, 7)
        plan = plan_resize_constraint_v1(
            base_rect=base,
            handle="e",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(),
        )
        self.assertEqual(RectEmu(100, 200, 120, 40), plan.constrained_rect)
        self.assertFalse(plan.centered_applied)
        self.assertFalse(plan.aspect_applied)
        self.assertIsNone(plan.aspect_control_axis)

    def test_shift_aspect_has_no_effect_on_edge_handle_v1(self):
        base = RectEmu(100, 200, 80, 40)
        raw = RectEmu(100, 200, 120, 100)
        plain = plan_resize_constraint_v1(
            base_rect=base,
            handle="e",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(),
        )
        shifted = plan_resize_constraint_v1(
            base_rect=base,
            handle="e",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(aspect_lock=True),
        )
        self.assertEqual(plain.constrained_rect, shifted.constrained_rect)
        self.assertFalse(shifted.aspect_applied)

    def test_centered_east_mirrors_across_exact_doubled_center(self):
        base = RectEmu(100, 200, 80, 40)
        raw = RectEmu(100, 200, 100, 40)  # dragged east edge = 200
        plan = plan_resize_constraint_v1(
            base_rect=base,
            handle="e",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(centered=True),
        )
        self.assertEqual(RectEmu(80, 200, 120, 40), plan.constrained_rect)
        self.assertEqual(
            base.x + base.right,
            plan.constrained_rect.x + plan.constrained_rect.right,
        )
        self.assertEqual(base.y, plan.constrained_rect.y)
        self.assertEqual(base.height, plan.constrained_rect.height)

    def test_aspect_corner_x_control_uses_exact_rational_rounding(self):
        base = RectEmu(0, 0, 3, 2)
        raw = RectEmu(0, 0, 4, 2)
        plan = plan_resize_constraint_v1(
            base_rect=base,
            handle="se",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(aspect_lock=True),
        )
        self.assertEqual("x", plan.aspect_control_axis)
        self.assertTrue(plan.aspect_applied)
        # 4 * 2 / 3 = 2.666... -> nearest EMU = 3.
        self.assertEqual(RectEmu(0, 0, 4, 3), plan.constrained_rect)

    def test_aspect_control_axis_uses_normalized_change_tie_to_x(self):
        base = RectEmu(10, 20, 100, 50)
        # +50% on both axes -> normalized tie -> X.
        raw = RectEmu(10, 20, 150, 75)
        plan = plan_resize_constraint_v1(
            base_rect=base,
            handle="se",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(aspect_lock=True),
        )
        self.assertEqual("x", plan.aspect_control_axis)
        self.assertEqual(RectEmu(10, 20, 150, 75), plan.constrained_rect)

    def test_aspect_y_control_can_expand_other_axis_beyond_raw(self):
        base = RectEmu(0, 0, 100, 50)
        # X +50%, Y +100% -> Y controls -> width becomes 200.
        raw = RectEmu(0, 0, 150, 100)
        plan = plan_resize_constraint_v1(
            base_rect=base,
            handle="se",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(aspect_lock=True),
        )
        self.assertEqual("y", plan.aspect_control_axis)
        self.assertEqual(RectEmu(0, 0, 200, 100), plan.constrained_rect)

    def test_centered_aspect_uses_nearest_required_parity_tie_to_larger(self):
        base = RectEmu(0, 0, 4, 2)
        # centered east desired width = 10: center2_x=4, raw.right=7.
        # X controls; ideal derived height = 10*2/4 = 5 exactly, but
        # centered Y needs even extent (base height parity=0). 4 and 6 are
        # equally distant -> V1 tie chooses larger = 6.
        raw = RectEmu(0, 0, 7, 2)
        plan = plan_resize_constraint_v1(
            base_rect=base,
            handle="se",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(
                centered=True,
                aspect_lock=True,
            ),
        )
        self.assertEqual("x", plan.aspect_control_axis)
        self.assertEqual(RectEmu(-3, -2, 10, 6), plan.constrained_rect)
        self.assertEqual(
            base.x + base.right,
            plan.constrained_rect.x + plan.constrained_rect.right,
        )
        self.assertEqual(
            base.y + base.bottom,
            plan.constrained_rect.y + plan.constrained_rect.bottom,
        )

    def test_centered_odd_base_extent_preserves_required_parity(self):
        base = RectEmu(0, 0, 5, 5)
        raw = RectEmu(0, 0, 6, 6)
        plan = plan_resize_constraint_v1(
            base_rect=base,
            handle="se",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(centered=True),
        )
        self.assertEqual(7, plan.constrained_rect.width)
        self.assertEqual(7, plan.constrained_rect.height)
        self.assertEqual(1, plan.constrained_rect.width % 2)
        self.assertEqual(1, plan.constrained_rect.height % 2)

    def test_modifier_recompute_is_stateless_from_same_base_and_raw(self):
        base = RectEmu(100, 100, 80, 40)
        raw = RectEmu(100, 100, 140, 90)
        plain1 = plan_resize_constraint_v1(
            base_rect=base,
            handle="se",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(),
        )
        _ = plan_resize_constraint_v1(
            base_rect=base,
            handle="se",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(centered=True, aspect_lock=True),
        )
        plain2 = plan_resize_constraint_v1(
            base_rect=base,
            handle="se",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(),
        )
        self.assertEqual(plain1, plain2)

    def test_crossing_fixed_edge_or_center_fails_closed(self):
        base = RectEmu(100, 100, 80, 40)
        with self.assertRaisesRegex(ResizeConstraintError, "crossed"):
            plan_resize_constraint_v1(
                base_rect=base,
                handle="w",
                raw_target_rect=RectEmu(190, 100, 10, 40),
                modifiers=ResizeModifierMaskV1(),
            )
        with self.assertRaisesRegex(ResizeConstraintError, "crossed"):
            plan_resize_constraint_v1(
                base_rect=base,
                handle="e",
                raw_target_rect=RectEmu(0, 100, 130, 40),
                modifiers=ResizeModifierMaskV1(centered=True),
            )

    def test_large_safe_coordinates_do_not_require_center_when_uncentered(self):
        base = RectEmu(MAX_SAFE_EMU - 1000, 0, 400, 100)
        raw = RectEmu(MAX_SAFE_EMU - 1000, 0, 500, 100)
        plan = plan_resize_constraint_v1(
            base_rect=base,
            handle="e",
            raw_target_rect=raw,
            modifiers=ResizeModifierMaskV1(),
        )
        self.assertEqual(500, plan.constrained_rect.width)

    def test_centered_mode_rejects_unsafe_doubled_center(self):
        base = RectEmu(MAX_SAFE_EMU - 1000, 0, 400, 100)
        raw = RectEmu(MAX_SAFE_EMU - 1000, 0, 500, 100)
        with self.assertRaisesRegex(ResizeConstraintError, "center2_x"):
            plan_resize_constraint_v1(
                base_rect=base,
                handle="e",
                raw_target_rect=raw,
                modifiers=ResizeModifierMaskV1(centered=True),
            )

    def test_invalid_modifier_shape_and_handle_fail_closed(self):
        with self.assertRaisesRegex(ResizeConstraintError, "boolean"):
            ResizeModifierMaskV1(centered=1)
        with self.assertRaisesRegex(ResizeConstraintError, "handle"):
            plan_resize_constraint_v1(
                base_rect=RectEmu(0, 0, 10, 10),
                handle="center",
                raw_target_rect=RectEmu(0, 0, 20, 20),
                modifiers=ResizeModifierMaskV1(),
            )

    def test_changed_flag_is_explicit(self):
        base = RectEmu(0, 0, 10, 10)
        same = plan_resize_constraint_v1(
            base_rect=base,
            handle="se",
            raw_target_rect=base,
            modifiers=ResizeModifierMaskV1(),
        )
        self.assertFalse(same.changed)
        changed = plan_resize_constraint_v1(
            base_rect=base,
            handle="se",
            raw_target_rect=RectEmu(0, 0, 20, 20),
            modifiers=ResizeModifierMaskV1(),
        )
        self.assertTrue(changed.changed)


if __name__ == "__main__":
    unittest.main()
