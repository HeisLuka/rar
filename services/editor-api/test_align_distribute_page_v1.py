#!/usr/bin/env python3
import unittest

from align_distribute_page_v1 import (
    AlignDistributePagePlanError,
    plan_align_distribute_page_v1,
)
from align_distribute_plan_v1 import GeometryMemberV1, RectEmu


def m(node_id, x, y, width, height):
    return GeometryMemberV1(node_id=node_id, bounds=RectEmu(x, y, width, height))


def after_map(plan):
    return {item.node_id: item.after for item in plan.members}


class AlignDistributePageV1Tests(unittest.TestCase):
    def test_page_edge_alignment_uses_authoritative_page_rect(self):
        page = RectEmu(-100, 50, 400, 300)
        members = (m("a", 0, 0, 20, 30), m("b", 50, 80, 40, 10))

        left = after_map(plan_align_distribute_page_v1(
            page_bounds=page, members=members, mode="align_left"
        ))
        right = after_map(plan_align_distribute_page_v1(
            page_bounds=page, members=members, mode="align_right"
        ))
        top = after_map(plan_align_distribute_page_v1(
            page_bounds=page, members=members, mode="align_top"
        ))
        bottom = after_map(plan_align_distribute_page_v1(
            page_bounds=page, members=members, mode="align_bottom"
        ))

        self.assertTrue(all(rect.x == page.x for rect in left.values()))
        self.assertTrue(all(rect.right == page.right for rect in right.values()))
        self.assertTrue(all(rect.y == page.y for rect in top.values()))
        self.assertTrue(all(rect.bottom == page.bottom for rect in bottom.values()))

    def test_page_center_alignment_uses_versioned_half_emu_rounding(self):
        page = RectEmu(0, 0, 11, 11)
        members = (m("a", 20, 20, 2, 2), m("b", 30, 30, 3, 3))

        horizontal = after_map(plan_align_distribute_page_v1(
            page_bounds=page, members=members, mode="align_horizontal_center"
        ))
        vertical = after_map(plan_align_distribute_page_v1(
            page_bounds=page, members=members, mode="align_vertical_center"
        ))

        self.assertEqual(5, horizontal["a"].x)  # ideal 4.5 -> away from zero
        self.assertEqual(4, horizontal["b"].x)
        self.assertEqual(5, vertical["a"].y)
        self.assertEqual(4, vertical["b"].y)

    def test_free_space_distribution_includes_leading_and_trailing_gaps(self):
        page = RectEmu(0, 0, 100, 50)
        plan = plan_align_distribute_page_v1(
            page_bounds=page,
            members=(m("right", 70, 0, 10, 10), m("left", 5, 0, 10, 10)),
            mode="distribute_horizontal",
        )
        out = after_map(plan)
        self.assertEqual(27, out["left"].x)
        self.assertEqual(64, out["right"].x)
        gaps = (
            out["left"].x - page.x,
            out["right"].x - out["left"].right,
            page.right - out["right"].right,
        )
        self.assertEqual((27, 27, 26), gaps)

    def test_overlap_distribution_pins_page_outer_edges(self):
        page = RectEmu(0, 0, 100, 40)
        plan = plan_align_distribute_page_v1(
            page_bounds=page,
            members=(
                m("c", 60, 0, 50, 10),
                m("a", 0, 0, 50, 10),
                m("b", 30, 0, 50, 10),
            ),
            mode="distribute_horizontal",
        )
        out = after_map(plan)
        self.assertEqual(page.x, out["a"].x)
        self.assertEqual(page.right, out["c"].right)
        self.assertEqual(-25, out["b"].x - out["a"].right)
        self.assertEqual(-25, out["c"].x - out["b"].right)

    def test_vertical_distribution_and_input_order_are_deterministic(self):
        page = RectEmu(0, -20, 100, 100)
        members = (
            m("b", 0, 30, 10, 10),
            m("a", 0, 0, 10, 10),
            m("c", 0, 60, 10, 10),
        )
        first = plan_align_distribute_page_v1(
            page_bounds=page,
            members=members,
            mode="distribute_vertical",
        )
        second = plan_align_distribute_page_v1(
            page_bounds=page,
            members=tuple(reversed(members)),
            mode="distribute_vertical",
        )
        self.assertEqual(first, second)

    def test_distribution_requires_two_but_align_accepts_one(self):
        single = (m("a", 10, 10, 20, 20),)
        aligned = plan_align_distribute_page_v1(
            page_bounds=RectEmu(0, 0, 100, 100),
            members=single,
            mode="align_left",
        )
        self.assertEqual(0, aligned.members[0].after.x)
        with self.assertRaisesRegex(AlignDistributePagePlanError, "at least two"):
            plan_align_distribute_page_v1(
                page_bounds=RectEmu(0, 0, 100, 100),
                members=single,
                mode="distribute_horizontal",
            )

    def test_no_change_and_duplicate_ids(self):
        plan = plan_align_distribute_page_v1(
            page_bounds=RectEmu(0, 0, 100, 100),
            members=(m("a", 0, 10, 10, 10),),
            mode="align_left",
        )
        self.assertEqual("no_change", plan.status)

        with self.assertRaisesRegex(AlignDistributePagePlanError, "unique"):
            plan_align_distribute_page_v1(
                page_bounds=RectEmu(0, 0, 100, 100),
                members=(m("x", 0, 0, 10, 10), m("x", 20, 0, 10, 10)),
                mode="align_left",
            )


if __name__ == "__main__":
    unittest.main()
