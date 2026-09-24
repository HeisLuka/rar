#!/usr/bin/env python3
import json
from authored_group_geometry_v1 import RectEmu
from group_ancestor_refit_v1 import GroupCascadeSnapshotV1
from group_refit_plan_v1 import GroupRefitChildV1
from group_transform_chain_v1 import AuthoredGroupEdgeV1
from nested_group_selection_v1 import NestedGroupPathEdgeV1, NestedGroupPathSnapshotV1, NestedGroupSelectionScopeV1, NestedGroupSelectionTargetV1
from nested_absolute_target_v1 import DesiredPageRectV1, NestedAbsoluteTargetMemberV1, plan_nested_absolute_targets_v1

children=(("a",RectEmu(10,10,20,20)),("s",RectEmu(40,40,10,10)))
edge=AuthoredGroupEdgeV1("g0","page:1",None,tuple(n for n,_ in children),RectEmu(100,100,100,100),RectEmu(0,0,100,100))
scope=NestedGroupSelectionScopeV1("page:1",("g0",),(NestedGroupSelectionTargetV1("page:1",("g0",),"a"),),NestedGroupSelectionTargetV1("page:1",("g0",),"a"))
snapshot=NestedGroupPathSnapshotV1("page:1",(NestedGroupPathEdgeV1("g0",None,tuple(n for n,_ in children)),))
cascade=GroupCascadeSnapshotV1(edge=edge,children=tuple(GroupRefitChildV1(n,r) for n,r in children))
plan=plan_nested_absolute_targets_v1(
    scope=scope,
    selection_snapshot=snapshot,
    ancestry=(edge,),
    cascade_snapshots=(cascade,),
    members=(NestedAbsoluteTargetMemberV1("a",RectEmu(10,10,20,20)),),
    desired_page_rects=(DesiredPageRectV1("a",RectEmu(190,190,20,20)),),
)
print(json.dumps({
    "schema":"chaptera.nested-absolute-target-receipt.v1",
    "measurement_class":"synthetic_authored_group_geometry",
    "status":plan.status,
    "mutation_route":plan.mutation_route,
    "selected_node_ids":plan.selected_node_ids,
    "desired_page_rect":{"x":plan.move_entries[0].after_page.x,"y":plan.move_entries[0].after_page.y,"width":plan.move_entries[0].after_page.width,"height":plan.move_entries[0].after_page.height},
    "group_refit_status":plan.immediate_refit.status if plan.immediate_refit else None,
    "unselected_siblings_preserved":plan.unselected_siblings_preserved,
    "authoring_mutations_during_planning":plan.authoring_mutations,
    "rich_family_admission":"not_in_v1_until_separate_gate",
    "native_publisher_claim":False,
},indent=2,sort_keys=True))
