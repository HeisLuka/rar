#!/usr/bin/env python3
import unittest

from authored_group_geometry_v1 import RectEmu
from fragment_container_placement_v1 import (
    FragmentContainerPlacementError,
    FragmentMemberV2,
    FragmentV2,
    GroupDestinationV1,
    PageDestinationV1,
    PointEmu,
    plan_fragment_container_placement_v1,
)
from group_transform_chain_v1 import AuthoredGroupEdgeV1


def fragment(*members):
    return FragmentV2(tuple(FragmentMemberV2(key, rect) for key, rect in members))


def edge(
    group_id,
    *,
    page_id="page:1",
    parent=None,
    bounds=RectEmu(100, 100, 200, 200),
    local=RectEmu(0, 0, 200, 200),
    children=("existing",),
):
    return AuthoredGroupEdgeV1(
        group_id=group_id,
        page_id=page_id,
        parent_group_id=parent,
        children=children,
        bounds_in_parent=bounds,
        local_coordinate_space=local,
    )


class FragmentContainerPlacementV1Tests(unittest.TestCase):
    def test_page_destination_applies_one_shared_translation(self):
        f = fragment(
            ("a", RectEmu(-10, 0, 20, 30)),
            ("b", RectEmu(40, 50, 10, 10)),
        )
        plan = plan_fragment_container_placement_v1(
            fragment=f,
            destination=PageDestinationV1("page:1"),
            desired_origin_page=PointEmu(100, 200),
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual(
            (RectEmu(90, 200, 20, 30), RectEmu(140, 250, 10, 10)),
            tuple(item.desired_effective_page_bounds for item in plan.members),
        )
        self.assertEqual(
            tuple(item.desired_effective_page_bounds for item in plan.members),
            tuple(item.destination_local_bounds for item in plan.members),
        )

    def test_group_identity_transform_maps_exactly_to_local(self):
        f = fragment(("a", RectEmu(0, 0, 20, 30)))
        destination = GroupDestinationV1(
            page_id="page:1",
            group_id="g",
            ancestry=(edge("g"),),
        )
        plan = plan_fragment_container_placement_v1(
            fragment=f,
            destination=destination,
            desired_origin_page=PointEmu(120, 140),
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual(RectEmu(20, 40, 20, 30), plan.members[0].destination_local_bounds)
        self.assertEqual(
            plan.members[0].desired_effective_page_bounds,
            plan.members[0].verified_effective_page_bounds,
        )

    def test_scaled_group_requires_exact_inverse_forward_roundtrip(self):
        destination = GroupDestinationV1(
            page_id="page:1",
            group_id="g",
            ancestry=(
                edge(
                    "g",
                    bounds=RectEmu(100, 100, 200, 100),
                    local=RectEmu(0, 0, 100, 50),
                ),
            ),
        )
        exact = plan_fragment_container_placement_v1(
            fragment=fragment(("a", RectEmu(0, 0, 20, 10))),
            destination=destination,
            desired_origin_page=PointEmu(120, 120),
        )
        self.assertEqual("planned", exact.status)
        self.assertEqual(RectEmu(10, 10, 10, 5), exact.members[0].destination_local_bounds)

        lossy_destination = GroupDestinationV1(
            page_id="page:1",
            group_id="g",
            ancestry=(
                edge(
                    "g",
                    bounds=RectEmu(0, 0, 3, 3),
                    local=RectEmu(0, 0, 2, 2),
                ),
            ),
        )
        lossy = plan_fragment_container_placement_v1(
            fragment=fragment(("a", RectEmu(0, 0, 1, 1))),
            destination=lossy_destination,
            desired_origin_page=PointEmu(1, 1),
        )
        self.assertEqual("not_exactly_representable", lossy.status)
        self.assertEqual((), lossy.members)

    def test_exact_mapping_outside_group_envelope_requests_refit(self):
        destination = GroupDestinationV1(
            page_id="page:1",
            group_id="g",
            ancestry=(edge("g"),),
        )
        plan = plan_fragment_container_placement_v1(
            fragment=fragment(("a", RectEmu(0, 0, 20, 10))),
            destination=destination,
            desired_origin_page=PointEmu(290, 120),
        )
        self.assertEqual("destination_refit_required", plan.status)
        self.assertEqual(RectEmu(190, 20, 20, 10), plan.members[0].destination_local_bounds)

    def test_nested_chain_is_exact_and_preserves_member_order(self):
        ancestry = (
            edge(
                "outer",
                bounds=RectEmu(100, 100, 400, 400),
                local=RectEmu(0, 0, 200, 200),
                children=("inner",),
            ),
            edge(
                "inner",
                parent="outer",
                bounds=RectEmu(50, 50, 100, 100),
                local=RectEmu(0, 0, 100, 100),
            ),
        )
        plan = plan_fragment_container_placement_v1(
            fragment=fragment(
                ("first", RectEmu(0, 0, 10, 10)),
                ("second", RectEmu(20, 20, 10, 10)),
            ),
            destination=GroupDestinationV1("page:1", "inner", ancestry),
            desired_origin_page=PointEmu(220, 220),
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual(("first", "second"), tuple(item.member_key for item in plan.members))
        for item in plan.members:
            self.assertEqual(
                item.desired_effective_page_bounds,
                item.verified_effective_page_bounds,
            )

    def test_bad_path_and_duplicate_fragment_keys_fail_closed(self):
        with self.assertRaisesRegex(FragmentContainerPlacementError, "unique"):
            plan_fragment_container_placement_v1(
                fragment=fragment(
                    ("x", RectEmu(0, 0, 10, 10)),
                    ("x", RectEmu(20, 0, 10, 10)),
                ),
                destination=PageDestinationV1("page:1"),
                desired_origin_page=PointEmu(0, 0),
            )

        with self.assertRaisesRegex(FragmentContainerPlacementError, "innermost"):
            plan_fragment_container_placement_v1(
                fragment=fragment(("x", RectEmu(0, 0, 10, 10))),
                destination=GroupDestinationV1(
                    "page:1",
                    "wrong",
                    (edge("g"),),
                ),
                desired_origin_page=PointEmu(0, 0),
            )


if __name__ == "__main__":
    unittest.main()
