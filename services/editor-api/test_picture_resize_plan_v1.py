#!/usr/bin/env python3
import unittest

from picture_resize_plan_v1 import (
    MAX_SAFE_EMU,
    PictureResizePlanError,
    PointEmu,
    RectEmu,
    plan_picture_resize_v1,
)


class PictureResizePlanV1Tests(unittest.TestCase):
    def test_ratio_is_reduced_and_maximal_inside_envelope(self):
        plan = plan_picture_resize_v1(
            before=RectEmu(0, 0, 100, 50),
            handle="se",
            pointer_target=PointEmu(173, 120),
            intrinsic_width_px=1920,
            intrinsic_height_px=1080,
        )
        self.assertEqual((16, 9), plan.intrinsic_ratio)
        self.assertEqual(10, plan.scale_k)
        self.assertEqual(RectEmu(0, 0, 160, 90), plan.after)
        self.assertLessEqual(plan.after.right, 173)
        self.assertLessEqual(plan.after.bottom, 120)

    def test_each_corner_keeps_opposite_corner_exact(self):
        before = RectEmu(100, 200, 80, 40)
        cases = {
            "nw": (PointEmu(20, 140), PointEmu(before.right, before.bottom)),
            "ne": (PointEmu(260, 140), PointEmu(before.x, before.bottom)),
            "se": (PointEmu(260, 280), PointEmu(before.x, before.y)),
            "sw": (PointEmu(20, 280), PointEmu(before.right, before.y)),
        }
        for handle, (pointer, fixed) in cases.items():
            with self.subTest(handle=handle):
                plan = plan_picture_resize_v1(
                    before=before,
                    handle=handle,
                    pointer_target=pointer,
                    intrinsic_width_px=4,
                    intrinsic_height_px=3,
                )
                after = plan.after
                if handle == "nw":
                    actual_fixed = PointEmu(after.right, after.bottom)
                elif handle == "ne":
                    actual_fixed = PointEmu(after.x, after.bottom)
                elif handle == "se":
                    actual_fixed = PointEmu(after.x, after.y)
                else:
                    actual_fixed = PointEmu(after.right, after.y)
                self.assertEqual(fixed, actual_fixed)
                self.assertEqual(after.width * 3, after.height * 4)

    def test_pointer_crossing_and_zero_span_fail_closed(self):
        before = RectEmu(0, 0, 100, 100)
        for pointer in (
            PointEmu(0, 150),
            PointEmu(-1, 100),
            PointEmu(100, 0),
        ):
            with self.subTest(pointer=pointer):
                with self.assertRaises(PictureResizePlanError):
                    plan_picture_resize_v1(
                        before=before,
                        handle="se",
                        pointer_target=pointer,
                        intrinsic_width_px=1,
                        intrinsic_height_px=1,
                    )

    def test_tiny_envelope_that_cannot_fit_ratio_unit_fails_closed(self):
        with self.assertRaisesRegex(PictureResizePlanError, "cannot represent"):
            plan_picture_resize_v1(
                before=RectEmu(0, 0, 10, 10),
                handle="se",
                pointer_target=PointEmu(15, 1),
                intrinsic_width_px=16,
                intrinsic_height_px=9,
            )

    def test_exact_before_can_be_no_change(self):
        plan = plan_picture_resize_v1(
            before=RectEmu(0, 0, 160, 90),
            handle="se",
            pointer_target=PointEmu(160, 90),
            intrinsic_width_px=1920,
            intrinsic_height_px=1080,
        )
        self.assertEqual("no_change", plan.status)
        self.assertEqual(RectEmu(0, 0, 160, 90), plan.after)

    def test_invalid_intrinsic_dimensions_handle_and_overflow_fail_closed(self):
        with self.assertRaises(PictureResizePlanError):
            plan_picture_resize_v1(
                before=RectEmu(0, 0, 100, 100),
                handle="se",
                pointer_target=PointEmu(200, 200),
                intrinsic_width_px=0,
                intrinsic_height_px=1,
            )
        with self.assertRaisesRegex(PictureResizePlanError, "unsupported"):
            plan_picture_resize_v1(
                before=RectEmu(0, 0, 100, 100),
                handle="e",
                pointer_target=PointEmu(200, 200),
                intrinsic_width_px=1,
                intrinsic_height_px=1,
            )
        with self.assertRaisesRegex(PictureResizePlanError, "safe EMU"):
            plan_picture_resize_v1(
                before=RectEmu(MAX_SAFE_EMU - 5, 0, 10, 10),
                handle="se",
                pointer_target=PointEmu(MAX_SAFE_EMU, 20),
                intrinsic_width_px=1,
                intrinsic_height_px=1,
            )


if __name__ == "__main__":
    unittest.main()
