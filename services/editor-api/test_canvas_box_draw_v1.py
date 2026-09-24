#!/usr/bin/env python3
import unittest

from canvas_box_draw_v1 import (
    MAX_SAFE_EMU,
    BoxDrawError,
    PointEmu,
    RectEmu,
    cancel_box_draw_v1,
    commit_box_draw_v1,
    preview_box_draw_v1,
    start_box_draw_v1,
    update_box_draw_v1,
)


class CanvasBoxDrawV1Tests(unittest.TestCase):
    def test_all_drag_directions_normalize_identically(self):
        expected = RectEmu(-10, -20, 40, 60)
        for anchor, current in (
            (PointEmu(-10, -20), PointEmu(30, 40)),
            (PointEmu(30, -20), PointEmu(-10, 40)),
            (PointEmu(-10, 40), PointEmu(30, -20)),
            (PointEmu(30, 40), PointEmu(-10, -20)),
        ):
            with self.subTest(anchor=anchor, current=current):
                tx = start_box_draw_v1(page_id="page:1", anchor=anchor)
                tx = update_box_draw_v1(tx, current=current)
                self.assertEqual(expected, preview_box_draw_v1(tx).bounds)
                result = commit_box_draw_v1(tx)
                self.assertEqual("commit", result.status)
                self.assertEqual(expected, result.bounds)

    def test_pointer_motion_is_transient_until_commit(self):
        tx = start_box_draw_v1(page_id="page:1", anchor=PointEmu(0, 0))
        self.assertEqual("no_change", preview_box_draw_v1(tx).status)
        tx2 = update_box_draw_v1(tx, current=PointEmu(100, 50))
        self.assertEqual("preview", preview_box_draw_v1(tx2).status)
        self.assertEqual(PointEmu(0, 0), tx.anchor)
        self.assertEqual(PointEmu(0, 0), tx.current)
        self.assertEqual("commit", commit_box_draw_v1(tx2).status)

    def test_zero_size_release_is_no_change(self):
        for current in (PointEmu(0, 20), PointEmu(20, 0), PointEmu(0, 0)):
            tx = update_box_draw_v1(
                start_box_draw_v1(page_id="page:1", anchor=PointEmu(0, 0)),
                current=current,
            )
            result = commit_box_draw_v1(tx)
            self.assertEqual("no_change", result.status)
            self.assertIsNone(result.bounds)

    def test_cancel_emits_no_authoring_intent(self):
        tx = update_box_draw_v1(
            start_box_draw_v1(page_id="page:1", anchor=PointEmu(0, 0)),
            current=PointEmu(20, 30),
        )
        cancelled = cancel_box_draw_v1(tx)
        self.assertEqual("cancelled", preview_box_draw_v1(cancelled).status)
        result = commit_box_draw_v1(cancelled)
        self.assertEqual("cancelled", result.status)
        self.assertIsNone(result.bounds)
        with self.assertRaisesRegex(BoxDrawError, "cancelled"):
            update_box_draw_v1(cancelled, current=PointEmu(40, 50))

    def test_page_identity_is_explicit_and_stable(self):
        tx = start_box_draw_v1(page_id="customer-page:42", anchor=PointEmu(1, 2))
        tx = update_box_draw_v1(tx, current=PointEmu(11, 22))
        result = commit_box_draw_v1(tx)
        self.assertEqual("customer-page:42", result.page_id)

    def test_invalid_identity_and_overflow_fail_closed(self):
        with self.assertRaisesRegex(BoxDrawError, "page_id"):
            start_box_draw_v1(page_id="", anchor=PointEmu(0, 0))
        with self.assertRaisesRegex(BoxDrawError, "safe EMU"):
            start_box_draw_v1(
                page_id="page:1",
                anchor=PointEmu(MAX_SAFE_EMU + 1, 0),
            )


if __name__ == "__main__":
    unittest.main()
