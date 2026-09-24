#!/usr/bin/env python3
import unittest

from authored_group_geometry_v1 import (
    MAX_SAFE_EMU,
    AuthoredGroupGeometryError,
    GroupMemberInput,
    RectEmu,
    materialize_group_child_rect_v1,
    plan_authored_group_geometry_v1,
)


class AuthoredGroupGeometryV1Tests(unittest.TestCase):
    def test_creation_plan_uses_exact_bbox_and_translation(self):
        members = [
            GroupMemberInput("node:a", "page:1", RectEmu(-100, 50, 80, 40)),
            GroupMemberInput("node:b", "page:1", RectEmu(20, -30, 60, 100)),
            GroupMemberInput("node:c", "page:1", RectEmu(-40, 80, 20, 10)),
        ]
        plan = plan_authored_group_geometry_v1(
            group_id="group:1",
            members=members,
        )
        self.assertEqual("page:1", plan.page_id)
        self.assertEqual(RectEmu(-100, -30, 180, 120), plan.group_bounds)
        self.assertEqual(RectEmu(0, 0, 180, 120), plan.local_coordinate_space)
        self.assertEqual(
            [
                RectEmu(0, 80, 80, 40),
                RectEmu(120, 0, 60, 100),
                RectEmu(60, 110, 20, 10),
            ],
            [child.bounds for child in plan.children],
        )
        self.assertEqual(["node:a", "node:b", "node:c"], [c.node_id for c in plan.children])
        self.assertEqual("identity", plan.transform)

    def test_creation_roundtrip_reproduces_every_original_rect_exactly(self):
        members = [
            GroupMemberInput("node:a", "page:1", RectEmu(10, 20, 31, 47)),
            GroupMemberInput("node:b", "page:1", RectEmu(55, 99, 101, 13)),
        ]
        plan = plan_authored_group_geometry_v1(group_id="group:1", members=members)
        restored = [
            materialize_group_child_rect_v1(
                local_coordinate_space=plan.local_coordinate_space,
                child_local_bounds=child.bounds,
                current_group_bounds=plan.group_bounds,
            )
            for child in plan.children
        ]
        self.assertEqual([member.bounds for member in members], restored)

    def test_resize_maps_each_edge_independently_with_nearest_tie_rule(self):
        # Local [25, 50] inside a 100-wide space, resized to 101:
        # 25*101/100 = 25.25 -> 25
        # 50*101/100 = 50.5 -> 51 (tie rounds away/up)
        rect = materialize_group_child_rect_v1(
            local_coordinate_space=RectEmu(0, 0, 100, 100),
            child_local_bounds=RectEmu(25, 25, 25, 25),
            current_group_bounds=RectEmu(-10, -20, 101, 101),
        )
        self.assertEqual(RectEmu(15, 5, 26, 26), rect)

    def test_resize_can_move_group_without_rewriting_child_local_coordinates(self):
        local_space = RectEmu(0, 0, 200, 100)
        local = RectEmu(50, 20, 100, 40)
        first = materialize_group_child_rect_v1(
            local_coordinate_space=local_space,
            child_local_bounds=local,
            current_group_bounds=RectEmu(0, 0, 200, 100),
        )
        moved_scaled = materialize_group_child_rect_v1(
            local_coordinate_space=local_space,
            child_local_bounds=local,
            current_group_bounds=RectEmu(1000, -500, 400, 200),
        )
        self.assertEqual(RectEmu(50, 20, 100, 40), first)
        self.assertEqual(RectEmu(1100, -460, 200, 80), moved_scaled)
        self.assertEqual(RectEmu(50, 20, 100, 40), local)

    def test_duplicate_cross_page_invalid_rect_and_group_id_collisions_fail_closed(self):
        cases = [
            [
                GroupMemberInput("node:a", "page:1", RectEmu(0, 0, 10, 10)),
                GroupMemberInput("node:a", "page:1", RectEmu(20, 0, 10, 10)),
            ],
            [
                GroupMemberInput("node:a", "page:1", RectEmu(0, 0, 10, 10)),
                GroupMemberInput("node:b", "page:2", RectEmu(20, 0, 10, 10)),
            ],
            [
                GroupMemberInput("node:a", "page:1", RectEmu(0, 0, 0, 10)),
                GroupMemberInput("node:b", "page:1", RectEmu(20, 0, 10, 10)),
            ],
        ]
        for members in cases:
            with self.subTest(members=members):
                with self.assertRaises(AuthoredGroupGeometryError):
                    plan_authored_group_geometry_v1(group_id="group:1", members=members)

        with self.assertRaisesRegex(AuthoredGroupGeometryError, "group_id"):
            plan_authored_group_geometry_v1(
                group_id="node:a",
                members=[
                    GroupMemberInput("node:a", "page:1", RectEmu(0, 0, 10, 10)),
                    GroupMemberInput("node:b", "page:1", RectEmu(20, 0, 10, 10)),
                ],
            )

    def test_overflow_is_rejected_before_plan_or_materialization(self):
        with self.assertRaisesRegex(AuthoredGroupGeometryError, "safe EMU"):
            plan_authored_group_geometry_v1(
                group_id="group:1",
                members=[
                    GroupMemberInput("node:a", "page:1", RectEmu(MAX_SAFE_EMU - 5, 0, 10, 10)),
                    GroupMemberInput("node:b", "page:1", RectEmu(0, 0, 10, 10)),
                ],
            )

        with self.assertRaisesRegex(AuthoredGroupGeometryError, "safe EMU"):
            materialize_group_child_rect_v1(
                local_coordinate_space=RectEmu(0, 0, 10, 10),
                child_local_bounds=RectEmu(5, 0, 5, 10),
                current_group_bounds=RectEmu(MAX_SAFE_EMU - 2, 0, 10, 10),
            )

    def test_malformed_local_coordinate_state_fails_closed(self):
        for local_space, child in [
            (RectEmu(1, 0, 100, 100), RectEmu(10, 10, 20, 20)),
            (RectEmu(0, 0, 100, 100), RectEmu(-1, 10, 20, 20)),
            (RectEmu(0, 0, 100, 100), RectEmu(90, 10, 20, 20)),
        ]:
            with self.subTest(local_space=local_space, child=child):
                with self.assertRaises(AuthoredGroupGeometryError):
                    materialize_group_child_rect_v1(
                        local_coordinate_space=local_space,
                        child_local_bounds=child,
                        current_group_bounds=RectEmu(0, 0, 100, 100),
                    )


if __name__ == "__main__":
    unittest.main()
