import unittest

from authored_group_geometry_v1 import RectEmu, materialize_group_child_rect_v1
from group_member_geometry_v1 import (
    GroupMemberGeometryError,
    inverse_group_member_rect_v1,
    plan_group_member_move_v1,
    plan_group_member_resize_v1,
    project_group_member_rect_v1,
)


class GroupMemberGeometryV1Tests(unittest.TestCase):
    def setUp(self):
        self.group = RectEmu(1000, 2000, 101, 101)
        self.local = RectEmu(0, 0, 100, 100)

    def test_forward_is_bit_identical_to_existing_group_mapping_law(self):
        child = RectEmu(25, 25, 25, 25)
        plan = project_group_member_rect_v1(
            group_bounds=self.group,
            local_coordinate_space=self.local,
            local_rect=child,
        )
        existing = materialize_group_child_rect_v1(
            local_coordinate_space=self.local,
            child_local_bounds=child,
            current_group_bounds=self.group,
        )
        self.assertEqual(existing, plan.effective_page_rect)
        self.assertEqual(RectEmu(1025, 2025, 26, 26), plan.effective_page_rect)

    def test_inverse_reprojects_the_canonical_local_result(self):
        desired = RectEmu(1025, 2025, 26, 26)
        plan = inverse_group_member_rect_v1(
            group_bounds=self.group,
            local_coordinate_space=self.local,
            desired_page_rect=desired,
        )
        self.assertEqual(RectEmu(25, 25, 25, 25), plan.canonical_local_rect)
        self.assertEqual(desired, plan.effective_page_rect)

    def test_move_preserves_local_size_and_returns_actual_rounded_preview(self):
        base = RectEmu(20, 20, 30, 40)
        plan = plan_group_member_move_v1(
            group_bounds=self.group,
            local_coordinate_space=self.local,
            base_local_rect=base,
            desired_page_x=1051,
            desired_page_y=2051,
        )
        self.assertEqual(RectEmu(50, 50, 30, 40), plan.canonical_local_rect)
        self.assertEqual(base.width, plan.canonical_local_rect.width)
        self.assertEqual(base.height, plan.canonical_local_rect.height)
        self.assertEqual(RectEmu(1051, 2051, 30, 40), plan.effective_page_rect)

    def test_inverse_half_tie_is_canonicalized_and_preview_shows_reprojected_edge(self):
        plan = plan_group_member_move_v1(
            group_bounds=RectEmu(0, 0, 4, 4),
            local_coordinate_space=RectEmu(0, 0, 2, 2),
            base_local_rect=RectEmu(0, 0, 1, 1),
            desired_page_x=1,
            desired_page_y=1,
        )
        self.assertEqual(RectEmu(1, 1, 1, 1), plan.canonical_local_rect)
        self.assertEqual(RectEmu(2, 2, 2, 2), plan.effective_page_rect)

    def test_resize_changes_only_active_local_edges(self):
        base = RectEmu(20, 20, 40, 40)
        plan = plan_group_member_resize_v1(
            group_bounds=self.group,
            local_coordinate_space=self.local,
            base_local_rect=base,
            desired_page_left=1010,
            desired_page_bottom=2081,
        )
        self.assertEqual(10, plan.canonical_local_rect.x)
        self.assertEqual(base.right, plan.canonical_local_rect.right)
        self.assertEqual(base.y, plan.canonical_local_rect.y)
        self.assertEqual(80, plan.canonical_local_rect.bottom)

    def test_move_and_resize_cannot_escape_group_envelope(self):
        with self.assertRaisesRegex(GroupMemberGeometryError, "exceeds local coordinate space"):
            plan_group_member_move_v1(
                group_bounds=self.group,
                local_coordinate_space=self.local,
                base_local_rect=RectEmu(20, 20, 30, 30),
                desired_page_x=1090,
                desired_page_y=2020,
            )
        with self.assertRaisesRegex(GroupMemberGeometryError, "outside Group envelope"):
            plan_group_member_resize_v1(
                group_bounds=self.group,
                local_coordinate_space=self.local,
                base_local_rect=RectEmu(20, 20, 30, 30),
                desired_page_right=1102,
            )

    def test_resize_rejects_two_edges_on_one_axis_and_collapse(self):
        base = RectEmu(20, 20, 40, 40)
        with self.assertRaisesRegex(GroupMemberGeometryError, "one horizontal edge"):
            plan_group_member_resize_v1(
                group_bounds=self.group,
                local_coordinate_space=self.local,
                base_local_rect=base,
                desired_page_left=1010,
                desired_page_right=1050,
            )
        with self.assertRaisesRegex(GroupMemberGeometryError, "remain positive"):
            plan_group_member_resize_v1(
                group_bounds=self.group,
                local_coordinate_space=self.local,
                base_local_rect=base,
                desired_page_left=1061,
            )

    def test_each_update_recomputes_from_immutable_base(self):
        base = RectEmu(10, 10, 20, 20)
        _ = plan_group_member_move_v1(
            group_bounds=self.group,
            local_coordinate_space=self.local,
            base_local_rect=base,
            desired_page_x=1030,
            desired_page_y=2030,
        )
        after_intermediate = plan_group_member_move_v1(
            group_bounds=self.group,
            local_coordinate_space=self.local,
            base_local_rect=base,
            desired_page_x=1040,
            desired_page_y=2040,
        )
        direct = plan_group_member_move_v1(
            group_bounds=self.group,
            local_coordinate_space=self.local,
            base_local_rect=base,
            desired_page_x=1040,
            desired_page_y=2040,
        )
        self.assertEqual(direct, after_intermediate)
        self.assertEqual(RectEmu(10, 10, 20, 20), base)

    def test_zero_or_non_positive_spaces_fail_closed(self):
        with self.assertRaises(GroupMemberGeometryError):
            project_group_member_rect_v1(
                group_bounds=RectEmu(0, 0, 0, 100),
                local_coordinate_space=self.local,
                local_rect=RectEmu(10, 10, 20, 20),
            )


if __name__ == "__main__":
    unittest.main()
