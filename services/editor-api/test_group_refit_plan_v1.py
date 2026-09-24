import unittest

from authored_group_geometry_v1 import RectEmu, materialize_group_child_rect_v1
from group_refit_plan_v1 import (
    GroupRefitChildV1,
    GroupRefitPlanError,
    plan_group_refit_v1,
)


def child(node_id, rect):
    return GroupRefitChildV1(node_id=node_id, local_rect=rect)


class GroupRefitPlanV1Tests(unittest.TestCase):
    def test_tight_fit_rebases_parameterization_without_moving_pixels(self):
        group = RectEmu(100, 200, 200, 100)
        local = RectEmu(0, 0, 100, 100)
        children = (
            child("a", RectEmu(10, 20, 20, 30)),
            child("b", RectEmu(60, 50, 30, 40)),
        )
        before = [
            materialize_group_child_rect_v1(
                local_coordinate_space=local,
                child_local_bounds=c.local_rect,
                current_group_bounds=group,
            )
            for c in children
        ]

        plan = plan_group_refit_v1(
            mode="tight_fit_current_contents",
            group_bounds=group,
            local_coordinate_space=local,
            children=children,
        )

        self.assertEqual("planned", plan.status)
        self.assertEqual(RectEmu(120, 220, 160, 70), plan.new_group_bounds)
        self.assertEqual(RectEmu(0, 0, 160, 70), plan.new_local_coordinate_space)
        self.assertEqual(["a", "b"], [c.node_id for c in plan.children])
        self.assertEqual(before, [c.effective_parent_rect for c in plan.children])

        after = [
            materialize_group_child_rect_v1(
                local_coordinate_space=plan.new_local_coordinate_space,
                child_local_bounds=c.local_rect,
                current_group_bounds=plan.new_group_bounds,
            )
            for c in plan.children
        ]
        self.assertEqual(before, after)

    def test_tight_fit_from_odd_scaled_group_preserves_exact_effective_rects(self):
        group = RectEmu(-10, 30, 101, 101)
        local = RectEmu(0, 0, 100, 100)
        children = (
            child("a", RectEmu(25, 25, 25, 25)),
            child("b", RectEmu(50, 50, 25, 25)),
        )
        old_effective = tuple(
            materialize_group_child_rect_v1(
                local_coordinate_space=local,
                child_local_bounds=c.local_rect,
                current_group_bounds=group,
            )
            for c in children
        )
        plan = plan_group_refit_v1(
            mode="tight_fit_current_contents",
            group_bounds=group,
            local_coordinate_space=local,
            children=children,
        )
        self.assertEqual(old_effective, tuple(c.effective_parent_rect for c in plan.children))
        self.assertEqual(
            old_effective,
            tuple(
                materialize_group_child_rect_v1(
                    local_coordinate_space=plan.new_local_coordinate_space,
                    child_local_bounds=c.local_rect,
                    current_group_bounds=plan.new_group_bounds,
                )
                for c in plan.children
            ),
        )

    def test_expand_only_expands_edges_and_preserves_unselected_sibling(self):
        group = RectEmu(100, 100, 100, 100)
        local = RectEmu(0, 0, 100, 100)
        children = (
            child("a", RectEmu(10, 10, 20, 20)),
            child("b", RectEmu(60, 60, 20, 20)),
        )
        sibling_before = RectEmu(160, 160, 20, 20)
        desired = RectEmu(70, 150, 25, 30)

        plan = plan_group_refit_v1(
            mode="expand_only_with_proposed_child_rect",
            group_bounds=group,
            local_coordinate_space=local,
            children=children,
            target_node_id="a",
            desired_target_parent_rect=desired,
        )

        self.assertEqual("planned", plan.status)
        self.assertEqual(RectEmu(70, 100, 130, 100), plan.new_group_bounds)
        self.assertEqual(desired, plan.children[0].effective_parent_rect)
        self.assertEqual(sibling_before, plan.children[1].effective_parent_rect)
        self.assertLessEqual(plan.new_group_bounds.x, group.x)
        self.assertLessEqual(plan.new_group_bounds.y, group.y)
        self.assertGreaterEqual(plan.new_group_bounds.right, group.right)
        self.assertGreaterEqual(plan.new_group_bounds.bottom, group.bottom)

    def test_expand_only_contained_target_returns_no_envelope_change(self):
        group = RectEmu(100, 100, 100, 100)
        local = RectEmu(0, 0, 100, 100)
        children = (child("a", RectEmu(10, 10, 20, 20)),)
        plan = plan_group_refit_v1(
            mode="expand_only_with_proposed_child_rect",
            group_bounds=group,
            local_coordinate_space=local,
            children=children,
            target_node_id="a",
            desired_target_parent_rect=RectEmu(20, 20, 10, 10),
        )
        # The desired rect is outside the Group because page-space intent is required.
        self.assertEqual("planned", plan.status)

        contained = plan_group_refit_v1(
            mode="expand_only_with_proposed_child_rect",
            group_bounds=group,
            local_coordinate_space=local,
            children=children,
            target_node_id="a",
            desired_target_parent_rect=RectEmu(120, 120, 10, 10),
        )
        self.assertEqual("no_envelope_change", contained.status)
        self.assertEqual(group, contained.new_group_bounds)
        self.assertEqual(local, contained.new_local_coordinate_space)

    def test_nested_overflow_requests_ancestor_refit(self):
        group = RectEmu(20, 20, 50, 50)
        local = RectEmu(0, 0, 50, 50)
        children = (child("a", RectEmu(10, 10, 20, 20)),)
        plan = plan_group_refit_v1(
            mode="expand_only_with_proposed_child_rect",
            group_bounds=group,
            local_coordinate_space=local,
            children=children,
            target_node_id="a",
            desired_target_parent_rect=RectEmu(80, 30, 20, 20),
            parent_local_coordinate_space=RectEmu(0, 0, 90, 100),
        )
        self.assertEqual("ancestor_refit_required", plan.status)
        self.assertEqual(RectEmu(20, 20, 80, 50), plan.new_group_bounds)
        self.assertEqual(RectEmu(80, 30, 20, 20), plan.children[0].effective_parent_rect)

    def test_top_level_signed_off_page_expansion_is_allowed(self):
        plan = plan_group_refit_v1(
            mode="expand_only_with_proposed_child_rect",
            group_bounds=RectEmu(10, 10, 50, 50),
            local_coordinate_space=RectEmu(0, 0, 50, 50),
            children=(child("a", RectEmu(10, 10, 20, 20)),),
            target_node_id="a",
            desired_target_parent_rect=RectEmu(-30, -40, 20, 20),
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual(RectEmu(-30, -40, 90, 100), plan.new_group_bounds)

    def test_invalid_inputs_fail_closed(self):
        with self.assertRaisesRegex(GroupRefitPlanError, "at least one"):
            plan_group_refit_v1(
                mode="tight_fit_current_contents",
                group_bounds=RectEmu(0, 0, 100, 100),
                local_coordinate_space=RectEmu(0, 0, 100, 100),
                children=(),
            )

        duplicate = (
            child("a", RectEmu(0, 0, 10, 10)),
            child("a", RectEmu(20, 20, 10, 10)),
        )
        with self.assertRaisesRegex(GroupRefitPlanError, "unique"):
            plan_group_refit_v1(
                mode="tight_fit_current_contents",
                group_bounds=RectEmu(0, 0, 100, 100),
                local_coordinate_space=RectEmu(0, 0, 100, 100),
                children=duplicate,
            )

        with self.assertRaisesRegex(GroupRefitPlanError, "not a current direct child"):
            plan_group_refit_v1(
                mode="expand_only_with_proposed_child_rect",
                group_bounds=RectEmu(0, 0, 100, 100),
                local_coordinate_space=RectEmu(0, 0, 100, 100),
                children=(child("a", RectEmu(10, 10, 20, 20)),),
                target_node_id="missing",
                desired_target_parent_rect=RectEmu(120, 10, 20, 20),
            )

        with self.assertRaises(GroupRefitPlanError):
            plan_group_refit_v1(
                mode="expand_only_with_proposed_child_rect",
                group_bounds=RectEmu(0, 0, 100, 100),
                local_coordinate_space=RectEmu(0, 0, 100, 100),
                children=(child("a", RectEmu(10, 10, 20, 20)),),
                target_node_id="a",
                desired_target_parent_rect=RectEmu(120, 10, 0, 20),
            )


if __name__ == "__main__":
    unittest.main()
