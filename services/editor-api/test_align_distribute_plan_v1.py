#!/usr/bin/env python3
import unittest

from align_distribute_plan_v1 import (
    MAX_SAFE_EMU,
    AlignDistributePlanError,
    GeometryMemberV1,
    RectEmu,
    plan_align_distribute_v1,
)


def m(node_id, x, y, width, height):
    return GeometryMemberV1(node_id, RectEmu(x, y, width, height))


def after_map(plan):
    return {item.node_id: item.after for item in plan.members}


class AlignDistributePlanV1Tests(unittest.TestCase):
    def test_edge_alignment_uses_selection_outer_edges_and_preserves_size(self):
        members = (
            m("b", 30, 40, 20, 10),
            m("a", -10, 5, 10, 30),
        )
        left = after_map(plan_align_distribute_v1(members=members, mode="align_left"))
        right = after_map(plan_align_distribute_v1(members=members, mode="align_right"))
        top = after_map(plan_align_distribute_v1(members=members, mode="align_top"))
        bottom = after_map(plan_align_distribute_v1(members=members, mode="align_bottom"))

        self.assertEqual(-10, left["a"].x)
        self.assertEqual(-10, left["b"].x)
        self.assertEqual(50, right["a"].right)
        self.assertEqual(50, right["b"].right)
        self.assertEqual(5, top["a"].y)
        self.assertEqual(5, top["b"].y)
        self.assertEqual(50, bottom["a"].bottom)
        self.assertEqual(50, bottom["b"].bottom)
        self.assertEqual((20, 10), (left["b"].width, left["b"].height))

    def test_center_alignment_uses_deterministic_half_emu_rounding(self):
        members = (
            m("a", 0, 0, 2, 2),
            m("b", 9, 9, 2, 2),
            m("odd", 3, 4, 3, 3),
        )
        horizontal = after_map(
            plan_align_distribute_v1(members=members, mode="align_horizontal_center")
        )
        vertical = after_map(
            plan_align_distribute_v1(members=members, mode="align_vertical_center")
        )
        # Selection span center is 5.5. A 3-wide member ideal left/top is 4.0.
        self.assertEqual(4, horizontal["odd"].x)
        self.assertEqual(4, vertical["odd"].y)
        # A 2-wide member ideal left/top is 4.5; ties round away from zero to 5.
        self.assertEqual(5, horizontal["a"].x)
        self.assertEqual(5, vertical["a"].y)

    def test_horizontal_distribution_keeps_outer_members_and_equalizes_integer_gaps(self):
        members = (
            m("middle2", 80, 0, 10, 10),
            m("last", 120, 0, 20, 10),
            m("first", 0, 0, 10, 10),
            m("middle1", 30, 0, 10, 10),
        )
        plan = plan_align_distribute_v1(members=members, mode="distribute_horizontal")
        out = after_map(plan)
        self.assertEqual(RectEmu(0, 0, 10, 10), out["first"])
        self.assertEqual(RectEmu(120, 0, 20, 10), out["last"])

        ordered = [out[name] for name in ("first", "middle1", "middle2", "last")]
        gaps = [ordered[i + 1].x - ordered[i].right for i in range(3)]
        self.assertLessEqual(max(gaps) - min(gaps), 1)
        self.assertEqual(100, sum(gaps))

    def test_distribution_allows_equal_overlap_when_extents_exceed_span(self):
        members = (
            m("first", 0, 0, 50, 10),
            m("middle", 30, 0, 50, 10),
            m("last", 60, 0, 50, 10),
        )
        out = after_map(
            plan_align_distribute_v1(members=members, mode="distribute_horizontal")
        )
        self.assertEqual(0, out["first"].x)
        self.assertEqual(60, out["last"].x)
        gaps = (
            out["middle"].x - out["first"].right,
            out["last"].x - out["middle"].right,
        )
        self.assertEqual((-20, -20), gaps)

    def test_remainder_allocation_and_input_order_are_deterministic(self):
        members = (
            m("a", 0, 0, 10, 10),
            m("b", 20, 0, 10, 10),
            m("c", 40, 0, 10, 10),
            m("d", 61, 0, 10, 10),
        )
        first = plan_align_distribute_v1(
            members=members,
            mode="distribute_horizontal",
        )
        second = plan_align_distribute_v1(
            members=tuple(reversed(members)),
            mode="distribute_horizontal",
        )
        self.assertEqual(first, second)
        out = after_map(first)
        gaps = (
            out["b"].x - out["a"].right,
            out["c"].x - out["b"].right,
            out["d"].x - out["c"].right,
        )
        self.assertEqual((11, 10, 10), gaps)

    def test_vertical_distribution_orders_by_geometry_then_node_id(self):
        members = (
            m("b", 0, 20, 10, 10),
            m("a", 20, 20, 10, 10),
            m("first", 0, 0, 10, 10),
            m("last", 0, 60, 10, 10),
        )
        plan = plan_align_distribute_v1(members=members, mode="distribute_vertical")
        out = after_map(plan)
        self.assertEqual(0, out["first"].y)
        self.assertEqual(60, out["last"].y)
        self.assertLess(out["a"].y, out["b"].y)

    def test_distribution_requires_three_and_duplicate_ids_fail_closed(self):
        with self.assertRaisesRegex(AlignDistributePlanError, "at least three"):
            plan_align_distribute_v1(
                members=(m("a", 0, 0, 10, 10), m("b", 20, 0, 10, 10)),
                mode="distribute_horizontal",
            )
        with self.assertRaisesRegex(AlignDistributePlanError, "unique"):
            plan_align_distribute_v1(
                members=(m("a", 0, 0, 10, 10), m("a", 20, 0, 10, 10)),
                mode="align_left",
            )

    def test_no_change_is_explicit(self):
        plan = plan_align_distribute_v1(
            members=(m("a", 0, 0, 10, 10), m("b", 0, 20, 10, 10)),
            mode="align_left",
        )
        self.assertEqual("no_change", plan.status)

    def test_overflow_fails_closed(self):
        with self.assertRaisesRegex(AlignDistributePlanError, "safe EMU"):
            plan_align_distribute_v1(
                members=(
                    m("a", MAX_SAFE_EMU - 5, 0, 10, 10),
                    m("b", 0, 0, 10, 10),
                ),
                mode="align_left",
            )


if __name__ == "__main__":
    unittest.main()
