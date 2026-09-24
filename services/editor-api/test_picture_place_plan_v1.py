#!/usr/bin/env python3
import unittest

from picture_place_plan_v1 import (
    MAX_SAFE_EMU,
    PicturePlacePlanError,
    RectEmu,
    plan_picture_place_v1,
)


class PicturePlacePlanV1Tests(unittest.TestCase):
    def test_reduces_ratio_and_uses_largest_integer_scale(self):
        plan = plan_picture_place_v1(
            envelope=RectEmu(10, 20, 173, 120),
            intrinsic_width_px=1920,
            intrinsic_height_px=1080,
        )
        self.assertEqual((16, 9), plan.intrinsic_ratio)
        self.assertEqual(10, plan.scale_k)
        self.assertEqual(RectEmu(16, 35, 160, 90), plan.frame)
        self.assertEqual((6, 15, 7, 15), plan.residual_emu)

    def test_odd_residual_goes_to_right_and_bottom(self):
        plan = plan_picture_place_v1(
            envelope=RectEmu(-10, -20, 11, 9),
            intrinsic_width_px=4,
            intrinsic_height_px=3,
        )
        self.assertEqual(RectEmu(-9, -19, 8, 6), plan.frame)
        self.assertEqual((1, 1, 2, 2), plan.residual_emu)

    def test_portrait_ratio_is_exact_and_centered(self):
        plan = plan_picture_place_v1(
            envelope=RectEmu(100, 200, 100, 100),
            intrinsic_width_px=2,
            intrinsic_height_px=3,
        )
        self.assertEqual((2, 3), plan.intrinsic_ratio)
        self.assertEqual(33, plan.scale_k)
        self.assertEqual(RectEmu(117, 200, 66, 99), plan.frame)
        self.assertEqual((17, 0, 17, 1), plan.residual_emu)

    def test_exact_envelope_has_zero_residual(self):
        plan = plan_picture_place_v1(
            envelope=RectEmu(5, 7, 160, 90),
            intrinsic_width_px=16,
            intrinsic_height_px=9,
        )
        self.assertEqual(plan.envelope, plan.frame)
        self.assertEqual((0, 0, 0, 0), plan.residual_emu)

    def test_tiny_envelope_that_cannot_fit_one_ratio_unit_fails(self):
        with self.assertRaisesRegex(PicturePlacePlanError, "cannot represent"):
            plan_picture_place_v1(
                envelope=RectEmu(0, 0, 15, 8),
                intrinsic_width_px=16,
                intrinsic_height_px=9,
            )

    def test_invalid_intrinsic_and_envelope_fail_closed(self):
        for width, height in ((0, 1), (1, 0), (-1, 1), (1, -1)):
            with self.subTest(width=width, height=height):
                with self.assertRaises(PicturePlacePlanError):
                    plan_picture_place_v1(
                        envelope=RectEmu(0, 0, 100, 100),
                        intrinsic_width_px=width,
                        intrinsic_height_px=height,
                    )
        with self.assertRaises(PicturePlacePlanError):
            plan_picture_place_v1(
                envelope=RectEmu(0, 0, 0, 100),
                intrinsic_width_px=1,
                intrinsic_height_px=1,
            )

    def test_safe_emu_overflow_is_rejected(self):
        with self.assertRaisesRegex(PicturePlacePlanError, "safe EMU"):
            plan_picture_place_v1(
                envelope=RectEmu(MAX_SAFE_EMU - 5, 0, 10, 10),
                intrinsic_width_px=1,
                intrinsic_height_px=1,
            )

    def test_planner_has_no_crop_fit_fill_or_asset_identity(self):
        plan = plan_picture_place_v1(
            envelope=RectEmu(0, 0, 100, 100),
            intrinsic_width_px=4,
            intrinsic_height_px=3,
        )
        fields = set(plan.__dataclass_fields__)
        self.assertNotIn("crop", fields)
        self.assertNotIn("fit_mode", fields)
        self.assertNotIn("asset", fields)


if __name__ == "__main__":
    unittest.main()
