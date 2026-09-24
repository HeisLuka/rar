import unittest

from authored_group_geometry_v1 import RectEmu, materialize_group_child_rect_v1
from group_multi_member_refit_v1 import (
    GroupMultiMemberProposalV1,
    GroupMultiMemberRefitError,
    plan_group_multi_member_refit_v1,
)
from group_refit_plan_v1 import GroupRefitChildV1
from group_transform_chain_v1 import AuthoredGroupEdgeV1


def edge(bounds, local, children, *, provenance="chaptera-authored-group-v1", parent=None):
    return AuthoredGroupEdgeV1(
        group_id="g",
        page_id="page:1",
        parent_group_id=parent,
        children=tuple(node_id for node_id, _ in children),
        bounds_in_parent=bounds,
        local_coordinate_space=local,
        provenance=provenance,
    )


def child_snapshot(items):
    return tuple(GroupRefitChildV1(node_id, rect) for node_id, rect in items)


class GroupMultiMemberRefitV1Tests(unittest.TestCase):
    def test_two_selected_members_expand_one_envelope_and_preserve_sibling(self):
        items = (
            ("a", RectEmu(10, 10, 20, 20)),
            ("b", RectEmu(60, 10, 20, 20)),
            ("s", RectEmu(40, 60, 10, 10)),
        )
        group = edge(
            RectEmu(100, 100, 100, 100),
            RectEmu(0, 0, 100, 100),
            items,
        )
        children = child_snapshot(items)
        proposals = (
            GroupMultiMemberProposalV1("b", RectEmu(190, 110, 30, 20)),
            GroupMultiMemberProposalV1("a", RectEmu(70, 80, 25, 20)),
        )

        plan = plan_group_multi_member_refit_v1(
            group=group,
            children=children,
            proposals=proposals,
        )

        self.assertEqual("planned", plan.status)
        self.assertEqual(("a", "b"), plan.proposed_node_ids)
        self.assertEqual(RectEmu(70, 80, 150, 120), plan.new_group_bounds)
        self.assertEqual(plan.new_group_bounds, plan.ancestor_path_child_candidate)
        self.assertEqual(["a", "b", "s"], [c.node_id for c in plan.children])

        by_id = {c.node_id: c for c in plan.children}
        self.assertEqual(RectEmu(70, 80, 25, 20), by_id["a"].effective_parent_rect)
        self.assertEqual(RectEmu(190, 110, 30, 20), by_id["b"].effective_parent_rect)
        self.assertEqual(RectEmu(140, 160, 10, 10), by_id["s"].effective_parent_rect)

        reprojected_sibling = materialize_group_child_rect_v1(
            local_coordinate_space=plan.new_local_coordinate_space,
            child_local_bounds=by_id["s"].local_rect,
            current_group_bounds=plan.new_group_bounds,
        )
        self.assertEqual(by_id["s"].effective_parent_rect, reprojected_sibling)

    def test_proposal_order_is_not_semantic(self):
        items = (
            ("a", RectEmu(10, 10, 20, 20)),
            ("b", RectEmu(60, 10, 20, 20)),
        )
        group = edge(
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            items,
        )
        children = child_snapshot(items)
        p1 = GroupMultiMemberProposalV1("a", RectEmu(-10, 10, 20, 20))
        p2 = GroupMultiMemberProposalV1("b", RectEmu(90, 10, 20, 20))

        first = plan_group_multi_member_refit_v1(
            group=group,
            children=children,
            proposals=(p1, p2),
        )
        second = plan_group_multi_member_refit_v1(
            group=group,
            children=children,
            proposals=(p2, p1),
        )
        self.assertEqual(first, second)

    def test_all_contained_returns_no_envelope_change(self):
        items = (
            ("a", RectEmu(10, 10, 20, 20)),
            ("b", RectEmu(60, 10, 20, 20)),
        )
        group = edge(
            RectEmu(100, 100, 100, 100),
            RectEmu(0, 0, 100, 100),
            items,
        )
        plan = plan_group_multi_member_refit_v1(
            group=group,
            children=child_snapshot(items),
            proposals=(
                GroupMultiMemberProposalV1("a", RectEmu(120, 130, 20, 20)),
                GroupMultiMemberProposalV1("b", RectEmu(150, 140, 20, 20)),
            ),
        )
        self.assertEqual("no_envelope_change", plan.status)
        self.assertEqual(group.bounds_in_parent, plan.new_group_bounds)
        self.assertEqual(group.local_coordinate_space, plan.new_local_coordinate_space)

    def test_nested_expansion_returns_single_ancestor_candidate(self):
        items = (
            ("a", RectEmu(10, 10, 20, 20)),
            ("s", RectEmu(40, 40, 10, 10)),
        )
        group = edge(
            RectEmu(20, 20, 50, 50),
            RectEmu(0, 0, 50, 50),
            items,
            parent="outer",
        )
        plan = plan_group_multi_member_refit_v1(
            group=group,
            children=child_snapshot(items),
            proposals=(
                GroupMultiMemberProposalV1("a", RectEmu(80, 30, 20, 20)),
            ),
            parent_local_coordinate_space=RectEmu(0, 0, 90, 100),
        )
        self.assertEqual("ancestor_refit_required", plan.status)
        self.assertEqual(RectEmu(20, 20, 80, 50), plan.ancestor_path_child_candidate)

    def test_odd_current_scale_preserves_unselected_effective_rect_exactly(self):
        items = (
            ("a", RectEmu(25, 25, 25, 25)),
            ("s", RectEmu(50, 50, 25, 25)),
        )
        group = edge(
            RectEmu(-10, 30, 101, 101),
            RectEmu(0, 0, 100, 100),
            items,
        )
        sibling_before = materialize_group_child_rect_v1(
            local_coordinate_space=group.local_coordinate_space,
            child_local_bounds=items[1][1],
            current_group_bounds=group.bounds_in_parent,
        )
        plan = plan_group_multi_member_refit_v1(
            group=group,
            children=child_snapshot(items),
            proposals=(
                GroupMultiMemberProposalV1("a", RectEmu(-30, 50, 25, 25)),
            ),
        )
        sibling = next(c for c in plan.children if c.node_id == "s")
        self.assertEqual(sibling_before, sibling.effective_parent_rect)
        self.assertEqual(
            sibling_before,
            materialize_group_child_rect_v1(
                local_coordinate_space=plan.new_local_coordinate_space,
                child_local_bounds=sibling.local_rect,
                current_group_bounds=plan.new_group_bounds,
            ),
        )

    def test_stale_membership_order_fails_closed(self):
        items = (
            ("a", RectEmu(10, 10, 20, 20)),
            ("b", RectEmu(40, 10, 20, 20)),
        )
        group = edge(
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            items,
        )
        stale_children = child_snapshot(tuple(reversed(items)))
        with self.assertRaisesRegex(GroupMultiMemberRefitError, "stale"):
            plan_group_multi_member_refit_v1(
                group=group,
                children=stale_children,
                proposals=(GroupMultiMemberProposalV1("a", RectEmu(-10, 10, 20, 20)),),
            )

    def test_duplicate_missing_and_unsupported_proposals_fail_closed(self):
        items = (("a", RectEmu(10, 10, 20, 20)),)
        group = edge(
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            items,
        )
        children = child_snapshot(items)
        proposal = GroupMultiMemberProposalV1("a", RectEmu(-10, 10, 20, 20))

        with self.assertRaisesRegex(GroupMultiMemberRefitError, "unique"):
            plan_group_multi_member_refit_v1(
                group=group,
                children=children,
                proposals=(proposal, proposal),
            )

        with self.assertRaisesRegex(GroupMultiMemberRefitError, "not a current direct child"):
            plan_group_multi_member_refit_v1(
                group=group,
                children=children,
                proposals=(GroupMultiMemberProposalV1("missing", RectEmu(-10, 10, 20, 20)),),
            )

        unsupported = edge(
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            items,
            provenance="source-backed",
        )
        with self.assertRaisesRegex(GroupMultiMemberRefitError, "provenance"):
            plan_group_multi_member_refit_v1(
                group=unsupported,
                children=children,
                proposals=(proposal,),
            )

    def test_envelope_never_shrinks_any_current_edge(self):
        items = (
            ("a", RectEmu(10, 10, 20, 20)),
            ("b", RectEmu(60, 60, 20, 20)),
        )
        group = edge(
            RectEmu(100, 100, 100, 100),
            RectEmu(0, 0, 100, 100),
            items,
        )
        plan = plan_group_multi_member_refit_v1(
            group=group,
            children=child_snapshot(items),
            proposals=(
                GroupMultiMemberProposalV1("a", RectEmu(50, 150, 20, 20)),
                GroupMultiMemberProposalV1("b", RectEmu(190, 90, 30, 20)),
            ),
        )
        old = group.bounds_in_parent
        new = plan.new_group_bounds
        self.assertLessEqual(new.x, old.x)
        self.assertLessEqual(new.y, old.y)
        self.assertGreaterEqual(new.right, old.right)
        self.assertGreaterEqual(new.bottom, old.bottom)


if __name__ == "__main__":
    unittest.main()
