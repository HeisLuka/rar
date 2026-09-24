#!/usr/bin/env python3
import unittest
from authored_group_geometry_v1 import RectEmu
from group_transform_chain_v1 import (
    AuthoredGroupEdgeV1,
    GroupPointEmu,
    GroupTransformChainError,
    MAX_AUTHORED_GROUP_DEPTH_V1,
    inverse_group_transform_chain_v1,
    inverse_group_transform_point_v1,
    project_group_transform_chain_v1,
    project_group_transform_point_v1,
)

def edge(g,p,parent,children,bounds,local):
    return AuthoredGroupEdgeV1(g,p,parent,tuple(children),bounds,local)

class GroupTransformChainV1Tests(unittest.TestCase):
    def test_empty_path_identity(self):
        r = RectEmu(10,20,30,40)
        p = project_group_transform_chain_v1(target_id="n", target_page_id="p", ancestry=(), target_local_rect=r)
        self.assertEqual(r,p.effective_page_rect)
        self.assertEqual((),p.group_path)

    def test_depth_one_matches_existing_boundary_law(self):
        a=(edge("g0","p",None,["n"],RectEmu(100,200,200,100),RectEmu(0,0,100,100)),)
        p=project_group_transform_chain_v1(target_id="n",target_page_id="p",ancestry=a,target_local_rect=RectEmu(25,25,25,25))
        self.assertEqual(RectEmu(150,225,50,25),p.effective_page_rect)

    def test_depth_two_boundarywise_rounding(self):
        a=(
            edge("g0","p",None,["g1"],RectEmu(0,0,101,101),RectEmu(0,0,100,100)),
            edge("g1","p","g0",["n"],RectEmu(25,25,50,50),RectEmu(0,0,100,100)),
        )
        p=project_group_transform_chain_v1(target_id="n",target_page_id="p",ancestry=a,target_local_rect=RectEmu(50,50,50,50))
        self.assertEqual(("g0","g1"),p.group_path)
        self.assertEqual(RectEmu(51,51,25,25),p.effective_page_rect)
        self.assertEqual(2,len(p.steps))

    def test_inverse_reprojects_effective_rect(self):
        a=(
            edge("g0","p",None,["g1"],RectEmu(10,20,200,100),RectEmu(0,0,100,100)),
            edge("g1","p","g0",["n"],RectEmu(20,10,50,80),RectEmu(0,0,100,100)),
        )
        inv=inverse_group_transform_chain_v1(target_id="n",target_page_id="p",ancestry=a,desired_page_rect=RectEmu(90,50,20,20))
        repro=project_group_transform_chain_v1(target_id="n",target_page_id="p",ancestry=a,target_local_rect=inv.canonical_local_rect)
        self.assertEqual(inv.effective_page_rect,repro.effective_page_rect)

    def test_point_forward_inverse_uses_same_boundary_rounding(self):
        a=(
            edge("g0","p",None,["n"],RectEmu(100,200,200,100),RectEmu(0,0,100,100)),
        )
        projected=project_group_transform_point_v1(
            target_id="n",
            target_page_id="p",
            ancestry=a,
            local_point=GroupPointEmu(25,40),
        )
        self.assertEqual(GroupPointEmu(150,240), projected.effective_page_point)
        inverse=inverse_group_transform_point_v1(
            target_id="n",
            target_page_id="p",
            ancestry=a,
            page_point=projected.effective_page_point,
        )
        self.assertEqual(GroupPointEmu(25,40), inverse.canonical_local_point)

    def test_point_inverse_rejects_non_exact_quantized_page_point(self):
        a=(
            edge("g0","p",None,["n"],RectEmu(0,0,3,3),RectEmu(0,0,2,2)),
        )
        with self.assertRaisesRegex(GroupTransformChainError, "not exactly representable"):
            inverse_group_transform_point_v1(
                target_id="n",
                target_page_id="p",
                ancestry=a,
                page_point=GroupPointEmu(1,0),
            )

    def test_point_mapping_allows_signed_extrapolation_but_requires_exact_roundtrip(self):
        a=(
            edge("g0","p",None,["n"],RectEmu(100,100,200,200),RectEmu(0,0,100,100)),
        )
        projected=project_group_transform_point_v1(
            target_id="n",
            target_page_id="p",
            ancestry=a,
            local_point=GroupPointEmu(-10,120),
        )
        self.assertEqual(GroupPointEmu(80,340), projected.effective_page_point)
        inverse=inverse_group_transform_point_v1(
            target_id="n",
            target_page_id="p",
            ancestry=a,
            page_point=GroupPointEmu(80,340),
        )
        self.assertEqual(GroupPointEmu(-10,120), inverse.canonical_local_point)

    def test_depth_cycle_membership_and_page_fail_closed(self):
        too_deep=tuple(edge(f"g{i}","p",None if i==0 else f"g{i-1}", [("n" if i==MAX_AUTHORED_GROUP_DEPTH_V1 else f"g{i+1}")], RectEmu(0,0,10,10), RectEmu(0,0,10,10)) for i in range(MAX_AUTHORED_GROUP_DEPTH_V1+1))
        with self.assertRaises(GroupTransformChainError):
            project_group_transform_chain_v1(target_id="n",target_page_id="p",ancestry=too_deep,target_local_rect=RectEmu(1,1,2,2))
        cyc=(edge("g0","p",None,["g0"],RectEmu(0,0,10,10),RectEmu(0,0,10,10)),)
        with self.assertRaises(GroupTransformChainError):
            project_group_transform_chain_v1(target_id="n",target_page_id="p",ancestry=cyc,target_local_rect=RectEmu(1,1,2,2))
        bad=(edge("g0","q",None,["n"],RectEmu(0,0,10,10),RectEmu(0,0,10,10)),)
        with self.assertRaises(GroupTransformChainError):
            project_group_transform_chain_v1(target_id="n",target_page_id="p",ancestry=bad,target_local_rect=RectEmu(1,1,2,2))

if __name__=="__main__":
    unittest.main()
