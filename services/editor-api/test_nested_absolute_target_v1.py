import unittest

from authored_group_geometry_v1 import RectEmu
from group_ancestor_refit_v1 import GroupCascadeSnapshotV1
from group_refit_plan_v1 import GroupRefitChildV1
from group_transform_chain_v1 import AuthoredGroupEdgeV1
from nested_group_selection_v1 import (
    NestedGroupPathEdgeV1,
    NestedGroupPathSnapshotV1,
    NestedGroupSelectionScopeV1,
    NestedGroupSelectionTargetV1,
)
from nested_absolute_target_v1 import (
    DesiredPageRectV1,
    NestedAbsoluteTargetMemberV1,
    plan_nested_absolute_targets_v1,
)


def edge(group_id, parent, children, bounds, local, page="page:1"):
    return AuthoredGroupEdgeV1(
        group_id=group_id,
        page_id=page,
        parent_group_id=parent,
        children=tuple(node_id for node_id, _ in children),
        bounds_in_parent=bounds,
        local_coordinate_space=local,
    )


def cascade(edge_value, children):
    return GroupCascadeSnapshotV1(
        edge=edge_value,
        children=tuple(GroupRefitChildV1(node_id, rect) for node_id, rect in children),
    )


def selection(path, children, selected, page="page:1"):
    edges=[]
    for index, group_id in enumerate(path):
        parent=None if index==0 else path[index-1]
        next_children=children[index]
        edges.append(NestedGroupPathEdgeV1(group_id,parent,tuple(node_id for node_id,_ in next_children)))
    targets=tuple(NestedGroupSelectionTargetV1(page,tuple(path),node_id) for node_id in selected)
    return (
        NestedGroupSelectionScopeV1(page,tuple(path),targets,targets[0] if len(targets)==1 else None),
        NestedGroupPathSnapshotV1(page,tuple(edges)),
    )


class NestedAbsoluteTargetTests(unittest.TestCase):
    def one_level(self, selected=("a",)):
        children=(("a",RectEmu(10,10,20,20)),("b",RectEmu(60,60,20,20)),("s",RectEmu(40,40,10,10)))
        g=edge("g0",None,children,RectEmu(100,100,100,100),RectEmu(0,0,100,100))
        scope,snap=selection(("g0",),(children,),selected)
        members=tuple(NestedAbsoluteTargetMemberV1(node_id,next(r for n,r in children if n==node_id)) for node_id in selected)
        return scope,snap,(g,),(cascade(g,children),),members

    def test_one_contained_target_returns_exact_move_node(self):
        scope,snap,ancestry,cascades,members=self.one_level(("a",))
        plan=plan_nested_absolute_targets_v1(
            scope=scope,selection_snapshot=snap,ancestry=ancestry,cascade_snapshots=cascades,members=members,
            desired_page_rects=(DesiredPageRectV1("a",RectEmu(130,140,20,20)),),
        )
        self.assertEqual("contained_exact",plan.status)
        self.assertEqual("move_node",plan.mutation_route)
        self.assertEqual(RectEmu(30,40,20,20),plan.move_entries[0].after_local)
        self.assertEqual(RectEmu(130,140,20,20),plan.move_entries[0].after_page)
        self.assertIsNone(plan.immediate_refit)

    def test_multi_contained_targets_return_deterministic_move_nodes_v2(self):
        scope,snap,ancestry,cascades,members=self.one_level(("b","a"))
        desired=(
            DesiredPageRectV1("b",RectEmu(150,150,20,20)),
            DesiredPageRectV1("a",RectEmu(120,130,20,20)),
        )
        plan=plan_nested_absolute_targets_v1(
            scope=scope,selection_snapshot=snap,ancestry=ancestry,cascade_snapshots=cascades,members=members,
            desired_page_rects=desired,
        )
        self.assertEqual("contained_exact",plan.status)
        self.assertEqual("move_nodes_v2",plan.mutation_route)
        self.assertEqual(("a","b"),plan.selected_node_ids)
        self.assertEqual(["a","b"],[x.node_id for x in plan.move_entries])

    def test_contained_non_exact_inverse_rejects_without_refit(self):
        children=(("a",RectEmu(0,0,1,1)),)
        g=edge("g0",None,children,RectEmu(0,0,3,3),RectEmu(0,0,2,2))
        scope,snap=selection(("g0",),(children,),("a",))
        plan=plan_nested_absolute_targets_v1(
            scope=scope,selection_snapshot=snap,ancestry=(g,),cascade_snapshots=(cascade(g,children),),
            members=(NestedAbsoluteTargetMemberV1("a",RectEmu(0,0,1,1)),),
            desired_page_rects=(DesiredPageRectV1("a",RectEmu(1,0,2,2)),),
        )
        self.assertEqual("rejected",plan.status)
        self.assertEqual("not_exactly_representable",plan.reason)
        self.assertIsNone(plan.immediate_refit)

    def test_one_member_expansion_uses_existing_refit_and_preserves_sibling(self):
        scope,snap,ancestry,cascades,members=self.one_level(("a",))
        plan=plan_nested_absolute_targets_v1(
            scope=scope,selection_snapshot=snap,ancestry=ancestry,cascade_snapshots=cascades,members=members,
            desired_page_rects=(DesiredPageRectV1("a",RectEmu(190,190,20,20)),),
        )
        self.assertEqual("refit_exact",plan.status)
        self.assertEqual("group_refit_composite",plan.mutation_route)
        self.assertEqual(RectEmu(100,100,110,110),plan.immediate_refit.new_group_bounds)
        self.assertIsNone(plan.ancestor_cascade)
        self.assertTrue(plan.unselected_siblings_preserved)
        self.assertEqual(RectEmu(190,190,20,20),plan.move_entries[0].after_page)

    def test_multi_member_expansion_uses_one_multi_refit(self):
        scope,snap,ancestry,cascades,members=self.one_level(("a","b"))
        plan=plan_nested_absolute_targets_v1(
            scope=scope,selection_snapshot=snap,ancestry=ancestry,cascade_snapshots=cascades,members=members,
            desired_page_rects=(
                DesiredPageRectV1("a",RectEmu(80,90,20,20)),
                DesiredPageRectV1("b",RectEmu(210,160,20,20)),
            ),
        )
        self.assertEqual("refit_exact",plan.status)
        self.assertEqual(("a","b"),plan.immediate_refit.proposed_node_ids)
        self.assertEqual(RectEmu(80,90,150,110),plan.immediate_refit.new_group_bounds)
        self.assertTrue(plan.unselected_siblings_preserved)
        self.assertEqual(
            [RectEmu(80,90,20,20),RectEmu(210,160,20,20)],
            [x.after_page for x in plan.move_entries],
        )

    def test_depth_two_expansion_that_fits_outer_needs_no_ancestor_cascade(self):
        outer_children=(("g1",RectEmu(20,20,50,50)),("outer-s",RectEmu(75,10,10,10)))
        inner_children=(("a",RectEmu(10,10,10,10)),("s",RectEmu(30,30,10,10)))
        g0=edge("g0",None,outer_children,RectEmu(0,0,100,100),RectEmu(0,0,100,100))
        g1=edge("g1","g0",inner_children,RectEmu(20,20,50,50),RectEmu(0,0,50,50))
        scope,snap=selection(("g0","g1"),(outer_children,inner_children),("a",))
        plan=plan_nested_absolute_targets_v1(
            scope=scope,selection_snapshot=snap,ancestry=(g0,g1),
            cascade_snapshots=(cascade(g0,outer_children),cascade(g1,inner_children)),
            members=(NestedAbsoluteTargetMemberV1("a",RectEmu(10,10,10,10)),),
            desired_page_rects=(DesiredPageRectV1("a",RectEmu(60,60,10,10)),),
        )
        self.assertEqual("refit_exact",plan.status)
        self.assertIsNone(plan.ancestor_cascade)
        self.assertEqual(RectEmu(60,60,10,10),plan.move_entries[0].after_page)
        self.assertTrue(plan.unselected_siblings_preserved)

    def test_depth_two_crossing_outer_uses_ancestor_cascade_and_reverifies_leaf(self):
        outer_children=(("g1",RectEmu(20,20,40,40)),("outer-s",RectEmu(70,10,10,10)))
        inner_children=(("a",RectEmu(10,10,10,10)),("s",RectEmu(25,25,10,10)))
        g0=edge("g0",None,outer_children,RectEmu(0,0,100,100),RectEmu(0,0,100,100))
        g1=edge("g1","g0",inner_children,RectEmu(20,20,40,40),RectEmu(0,0,40,40))
        scope,snap=selection(("g0","g1"),(outer_children,inner_children),("a",))
        target=RectEmu(95,95,10,10)
        plan=plan_nested_absolute_targets_v1(
            scope=scope,selection_snapshot=snap,ancestry=(g0,g1),
            cascade_snapshots=(cascade(g0,outer_children),cascade(g1,inner_children)),
            members=(NestedAbsoluteTargetMemberV1("a",RectEmu(10,10,10,10)),),
            desired_page_rects=(DesiredPageRectV1("a",target),),
        )
        self.assertEqual("refit_exact",plan.status)
        self.assertIsNotNone(plan.ancestor_cascade)
        self.assertEqual("planned",plan.ancestor_cascade.status)
        self.assertEqual(target,plan.move_entries[0].after_page)
        self.assertTrue(plan.unselected_siblings_preserved)

    def test_page_size_change_is_rejected_as_not_translation(self):
        scope,snap,ancestry,cascades,members=self.one_level(("a",))
        plan=plan_nested_absolute_targets_v1(
            scope=scope,selection_snapshot=snap,ancestry=ancestry,cascade_snapshots=cascades,members=members,
            desired_page_rects=(DesiredPageRectV1("a",RectEmu(130,140,21,20)),),
        )
        self.assertEqual("rejected",plan.status)
        self.assertEqual("invalid_request",plan.reason)

    def test_rich_family_is_not_silently_admitted_before_family_gate(self):
        scope,snap,ancestry,cascades,_=self.one_level(("a",))
        plan=plan_nested_absolute_targets_v1(
            scope=scope,selection_snapshot=snap,ancestry=ancestry,cascade_snapshots=cascades,
            members=(NestedAbsoluteTargetMemberV1("a",RectEmu(10,10,20,20),family="text_frame"),),
            desired_page_rects=(DesiredPageRectV1("a",RectEmu(130,130,20,20)),),
        )
        self.assertEqual("rejected",plan.status)
        self.assertEqual("unsupported_family",plan.reason)

    def test_stale_member_geometry_rejects_whole_plan(self):
        scope,snap,ancestry,cascades,_=self.one_level(("a",))
        plan=plan_nested_absolute_targets_v1(
            scope=scope,selection_snapshot=snap,ancestry=ancestry,cascade_snapshots=cascades,
            members=(NestedAbsoluteTargetMemberV1("a",RectEmu(11,10,20,20)),),
            desired_page_rects=(DesiredPageRectV1("a",RectEmu(130,130,20,20)),),
        )
        self.assertEqual("rejected",plan.status)
        self.assertEqual("stale_path",plan.reason)


if __name__=="__main__":
    unittest.main()
