import unittest

from authored_group_geometry_v1 import RectEmu
from multi_resize_plan_v1 import (
    MultiResizeMemberV1,
    MultiResizePlanError,
    plan_multi_resize_v1,
)


def member(node_id, rect, *, provenance="chaptera-authored", transform="identity"):
    return MultiResizeMemberV1(
        node_id=node_id,
        rect=rect,
        provenance=provenance,
        transform=transform,
    )


class MultiResizePlanV1Tests(unittest.TestCase):
    def setUp(self):
        self.members = (
            member("b", RectEmu(60, 20, 40, 40)),
            member("a", RectEmu(0, 0, 40, 20)),
            member("c", RectEmu(20, 60, 20, 40)),
        )
        self.base = RectEmu(0, 0, 100, 100)

    def test_east_resize_uses_one_aggregate_axis_map_and_preserves_y(self):
        plan = plan_multi_resize_v1(
            members=self.members,
            base_aggregate=self.base,
            handle="e",
            target_aggregate=RectEmu(0, 0, 200, 100),
        )
        self.assertEqual(["a", "b", "c"], [m.node_id for m in plan.members])
        by_id = {m.node_id: m for m in plan.members}
        self.assertEqual(RectEmu(0, 0, 80, 20), by_id["a"].after)
        self.assertEqual(RectEmu(120, 20, 80, 40), by_id["b"].after)
        self.assertEqual(RectEmu(40, 60, 40, 40), by_id["c"].after)
        for result in plan.members:
            self.assertEqual(result.before.y, result.after.y)
            self.assertEqual(result.before.height, result.after.height)

    def test_north_west_corner_keeps_opposite_corner_fixed(self):
        plan = plan_multi_resize_v1(
            members=self.members,
            base_aggregate=self.base,
            handle="nw",
            target_aggregate=RectEmu(-100, -50, 200, 150),
        )
        self.assertEqual(100, plan.target_aggregate.right)
        self.assertEqual(100, plan.target_aggregate.bottom)
        by_id = {m.node_id: m for m in plan.members}
        self.assertEqual(RectEmu(-100, -50, 80, 30), by_id["a"].after)
        self.assertEqual(RectEmu(20, -20, 80, 60), by_id["b"].after)
        self.assertEqual(RectEmu(-60, 40, 40, 60), by_id["c"].after)

    def test_interior_half_tie_rounds_up_deterministically(self):
        members = (
            member("a", RectEmu(0, 0, 25, 10)),
            member("b", RectEmu(25, 0, 25, 10)),
            member("c", RectEmu(50, 0, 50, 10)),
        )
        plan = plan_multi_resize_v1(
            members=members,
            base_aggregate=RectEmu(0, 0, 100, 10),
            handle="e",
            target_aggregate=RectEmu(0, 0, 202, 10),
        )
        by_id = {m.node_id: m for m in plan.members}
        # 25 * 202 / 100 = 50.5 -> 51.
        self.assertEqual(51, by_id["a"].after.right)
        # 50 * 202 / 100 = 101 exactly.
        self.assertEqual(101, by_id["b"].after.right)
        self.assertEqual(202, by_id["c"].after.right)

    def test_input_enumeration_does_not_affect_output(self):
        first = plan_multi_resize_v1(
            members=self.members,
            base_aggregate=self.base,
            handle="se",
            target_aggregate=RectEmu(0, 0, 150, 120),
        )
        second = plan_multi_resize_v1(
            members=tuple(reversed(self.members)),
            base_aggregate=self.base,
            handle="se",
            target_aggregate=RectEmu(0, 0, 150, 120),
        )
        self.assertEqual(first, second)

    def test_fixed_edge_law_rejects_wrong_target_anchor(self):
        with self.assertRaisesRegex(MultiResizePlanError, "left edge fixed"):
            plan_multi_resize_v1(
                members=self.members,
                base_aggregate=self.base,
                handle="e",
                target_aggregate=RectEmu(1, 0, 200, 100),
            )
        with self.assertRaisesRegex(MultiResizePlanError, "bottom edge fixed"):
            plan_multi_resize_v1(
                members=self.members,
                base_aggregate=self.base,
                handle="n",
                target_aggregate=RectEmu(0, -50, 100, 149),
            )

    def test_unaffected_axis_must_be_identical(self):
        with self.assertRaisesRegex(MultiResizePlanError, "vertical"):
            plan_multi_resize_v1(
                members=self.members,
                base_aggregate=self.base,
                handle="e",
                target_aggregate=RectEmu(0, 1, 200, 100),
            )

    def test_base_aggregate_must_equal_exact_member_union(self):
        with self.assertRaisesRegex(MultiResizePlanError, "exact member union"):
            plan_multi_resize_v1(
                members=self.members,
                base_aggregate=RectEmu(0, 0, 101, 100),
                handle="e",
                target_aggregate=RectEmu(0, 0, 202, 100),
            )

    def test_rounding_collapse_rejects_whole_plan(self):
        members = (
            member("a", RectEmu(0, 0, 1, 10)),
            member("b", RectEmu(1, 0, 1, 10)),
            member("c", RectEmu(2, 0, 98, 10)),
        )
        with self.assertRaisesRegex(MultiResizePlanError, "collapsed"):
            plan_multi_resize_v1(
                members=members,
                base_aggregate=RectEmu(0, 0, 100, 10),
                handle="e",
                target_aggregate=RectEmu(0, 0, 2, 10),
            )

    def test_unsupported_member_fails_before_partial_plan(self):
        bad = (
            self.members[0],
            member("bad", RectEmu(0, 0, 40, 20), provenance="source-backed"),
        )
        with self.assertRaisesRegex(MultiResizePlanError, "provenance"):
            plan_multi_resize_v1(
                members=bad,
                base_aggregate=RectEmu(0, 0, 100, 60),
                handle="e",
                target_aggregate=RectEmu(0, 0, 200, 60),
            )

        bad_transform = (
            self.members[0],
            member("bad", RectEmu(0, 0, 40, 20), transform="rotate90"),
        )
        with self.assertRaisesRegex(MultiResizePlanError, "transform"):
            plan_multi_resize_v1(
                members=bad_transform,
                base_aggregate=RectEmu(0, 0, 100, 60),
                handle="e",
                target_aggregate=RectEmu(0, 0, 200, 60),
            )

    def test_duplicate_ids_and_noop_resize_fail_closed(self):
        duplicate = (
            member("a", RectEmu(0, 0, 50, 100)),
            member("a", RectEmu(50, 0, 50, 100)),
        )
        with self.assertRaisesRegex(MultiResizePlanError, "unique"):
            plan_multi_resize_v1(
                members=duplicate,
                base_aggregate=self.base,
                handle="e",
                target_aggregate=RectEmu(0, 0, 200, 100),
            )
        with self.assertRaisesRegex(MultiResizePlanError, "change at least one"):
            plan_multi_resize_v1(
                members=self.members,
                base_aggregate=self.base,
                handle="se",
                target_aggregate=self.base,
            )


if __name__ == "__main__":
    unittest.main()
