import unittest

from authored_group_geometry_v1 import RectEmu, materialize_group_child_rect_v1
from group_ancestor_refit_v1 import (
    GroupCascadeSnapshotV1,
    _map_edge_forward_unbounded,
    plan_group_ancestor_refit_cascade_v1,
)
from group_refit_plan_v1 import GroupRefitChildV1
from group_transform_chain_v1 import AuthoredGroupEdgeV1


def snap(group_id, page_id, parent_group_id, bounds, local, children):
    child_ids = tuple(node_id for node_id, _ in children)
    return GroupCascadeSnapshotV1(
        edge=AuthoredGroupEdgeV1(
            group_id=group_id,
            page_id=page_id,
            parent_group_id=parent_group_id,
            children=child_ids,
            bounds_in_parent=bounds,
            local_coordinate_space=local,
        ),
        children=tuple(
            GroupRefitChildV1(node_id=node_id, local_rect=rect)
            for node_id, rect in children
        ),
    )


class GroupAncestorRefitCascadeV1Tests(unittest.TestCase):
    def test_unbounded_forward_matches_existing_mapper_for_contained_rect(self):
        local = RectEmu(0, 0, 100, 100)
        group = RectEmu(-20, 30, 101, 99)
        child = RectEmu(25, 10, 50, 40)
        existing = materialize_group_child_rect_v1(
            local_coordinate_space=local,
            child_local_bounds=child,
            current_group_bounds=group,
        )
        candidate = _map_edge_forward_unbounded(
            local_coordinate_space=local,
            local_rect=child,
            group_bounds=group,
        )
        self.assertEqual(existing, candidate)

    def test_one_level_expansion_returns_one_deepest_patch(self):
        g0 = snap(
            "g0",
            "page:1",
            None,
            RectEmu(100, 100, 100, 100),
            RectEmu(0, 0, 100, 100),
            (("n", RectEmu(20, 20, 20, 20)),),
        )
        plan = plan_group_ancestor_refit_cascade_v1(
            target_id="n",
            page_id="page:1",
            snapshots=(g0,),
            desired_page_rect=RectEmu(190, 190, 20, 20),
        )
        self.assertEqual("planned", plan.status)
        self.assertIsNone(plan.reason)
        self.assertEqual(["g0"], [p.group_id for p in plan.patches])
        self.assertEqual(RectEmu(100, 100, 110, 110), plan.patches[0].refit.new_group_bounds)
        self.assertEqual(
            RectEmu(190, 190, 20, 20),
            plan.patches[0].refit.children[0].effective_parent_rect,
        )

    def test_contained_one_level_intent_is_no_change_for_cascade(self):
        g0 = snap(
            "g0",
            "page:1",
            None,
            RectEmu(100, 100, 100, 100),
            RectEmu(0, 0, 100, 100),
            (("n", RectEmu(20, 20, 20, 20)),),
        )
        plan = plan_group_ancestor_refit_cascade_v1(
            target_id="n",
            page_id="page:1",
            snapshots=(g0,),
            desired_page_rect=RectEmu(150, 150, 20, 20),
        )
        self.assertEqual("no_change", plan.status)
        self.assertEqual("no_change", plan.reason)
        self.assertEqual((), plan.patches)

    def test_depth_two_stops_when_deepest_expansion_still_fits_outer_local_space(self):
        g0 = snap(
            "g0",
            "page:1",
            None,
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            (
                ("g1", RectEmu(20, 20, 40, 40)),
                ("sibling", RectEmu(70, 10, 10, 10)),
            ),
        )
        g1 = snap(
            "g1",
            "page:1",
            "g0",
            RectEmu(20, 20, 40, 40),
            RectEmu(0, 0, 40, 40),
            (("n", RectEmu(10, 10, 10, 10)),),
        )
        plan = plan_group_ancestor_refit_cascade_v1(
            target_id="n",
            page_id="page:1",
            snapshots=(g0, g1),
            desired_page_rect=RectEmu(55, 55, 10, 10),
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual(["g1"], [p.group_id for p in plan.patches])
        self.assertEqual(RectEmu(20, 20, 45, 45), plan.patches[0].refit.new_group_bounds)

    def test_depth_two_crossing_outer_envelope_returns_deepest_to_outer_patches(self):
        g0 = snap(
            "g0",
            "page:1",
            None,
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            (
                ("g1", RectEmu(20, 20, 40, 40)),
                ("sibling", RectEmu(70, 10, 10, 10)),
            ),
        )
        g1 = snap(
            "g1",
            "page:1",
            "g0",
            RectEmu(20, 20, 40, 40),
            RectEmu(0, 0, 40, 40),
            (("n", RectEmu(10, 10, 10, 10)),),
        )
        plan = plan_group_ancestor_refit_cascade_v1(
            target_id="n",
            page_id="page:1",
            snapshots=(g0, g1),
            desired_page_rect=RectEmu(95, 95, 10, 10),
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual(["g1", "g0"], [p.group_id for p in plan.patches])
        self.assertEqual([1, 0], [p.path_index for p in plan.patches])
        self.assertEqual(RectEmu(20, 20, 85, 85), plan.patches[0].refit.new_group_bounds)
        self.assertEqual(RectEmu(0, 0, 105, 105), plan.patches[1].refit.new_group_bounds)

        outer = plan.patches[1].refit
        sibling = next(c for c in outer.children if c.node_id == "sibling")
        self.assertEqual(RectEmu(70, 10, 10, 10), sibling.effective_parent_rect)

    def test_left_up_expansion_uses_signed_same_rounding_law(self):
        g0 = snap(
            "g0",
            "page:1",
            None,
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            (("g1", RectEmu(20, 20, 40, 40)),),
        )
        g1 = snap(
            "g1",
            "page:1",
            "g0",
            RectEmu(20, 20, 40, 40),
            RectEmu(0, 0, 40, 40),
            (("n", RectEmu(10, 10, 10, 10)),),
        )
        plan = plan_group_ancestor_refit_cascade_v1(
            target_id="n",
            page_id="page:1",
            snapshots=(g0, g1),
            desired_page_rect=RectEmu(-10, -10, 10, 10),
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual(["g1", "g0"], [p.group_id for p in plan.patches])
        self.assertEqual(RectEmu(-10, -10, 110, 110), plan.patches[-1].refit.new_group_bounds)

    def test_non_exact_outer_inverse_is_rejected_instead_of_drifting(self):
        g0 = snap(
            "g0",
            "page:1",
            None,
            RectEmu(0, 0, 3, 3),
            RectEmu(0, 0, 2, 2),
            (("g1", RectEmu(0, 0, 2, 2)),),
        )
        g1 = snap(
            "g1",
            "page:1",
            "g0",
            RectEmu(0, 0, 2, 2),
            RectEmu(0, 0, 2, 2),
            (("n", RectEmu(0, 0, 1, 1)),),
        )
        plan = plan_group_ancestor_refit_cascade_v1(
            target_id="n",
            page_id="page:1",
            snapshots=(g0, g1),
            desired_page_rect=RectEmu(1, 0, 2, 3),
        )
        self.assertEqual("rejected", plan.status)
        self.assertEqual("not_exactly_representable", plan.reason)
        self.assertEqual((), plan.patches)

    def test_stale_path_is_explicit(self):
        g0 = snap(
            "g0",
            "page:1",
            None,
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            (("g1", RectEmu(20, 20, 40, 40)),),
        )
        # Child snapshot disagrees with the nested edge current bounds.
        g1 = snap(
            "g1",
            "page:1",
            "g0",
            RectEmu(21, 20, 40, 40),
            RectEmu(0, 0, 40, 40),
            (("n", RectEmu(10, 10, 10, 10)),),
        )
        plan = plan_group_ancestor_refit_cascade_v1(
            target_id="n",
            page_id="page:1",
            snapshots=(g0, g1),
            desired_page_rect=RectEmu(95, 95, 10, 10),
        )
        self.assertEqual("rejected", plan.status)
        self.assertEqual("stale_path", plan.reason)

    def test_unsupported_provenance_is_explicit(self):
        edge = AuthoredGroupEdgeV1(
            group_id="g0",
            page_id="page:1",
            parent_group_id=None,
            children=("n",),
            bounds_in_parent=RectEmu(0, 0, 100, 100),
            local_coordinate_space=RectEmu(0, 0, 100, 100),
            provenance="source-backed",
        )
        snapshot = GroupCascadeSnapshotV1(
            edge=edge,
            children=(GroupRefitChildV1("n", RectEmu(10, 10, 20, 20)),),
        )
        plan = plan_group_ancestor_refit_cascade_v1(
            target_id="n",
            page_id="page:1",
            snapshots=(snapshot,),
            desired_page_rect=RectEmu(90, 90, 20, 20),
        )
        self.assertEqual("rejected", plan.status)
        self.assertEqual("unsupported", plan.reason)


if __name__ == "__main__":
    unittest.main()
