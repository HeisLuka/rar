#!/usr/bin/env python3
import unittest

from authored_group_geometry_v1 import RectEmu
from container_create_placement_v1 import (
    ContainerCreatePlacementError,
    plan_container_create_placement_v1,
)
from fragment_container_placement_v1 import GroupDestinationV1, PageDestinationV1
from group_transform_chain_v1 import AuthoredGroupEdgeV1


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


class ContainerCreatePlacementV1Tests(unittest.TestCase):
    def test_page_destination_is_exact_identity(self):
        desired = RectEmu(-50, 20, 100, 80)
        plan = plan_container_create_placement_v1(
            desired_effective_page_rect=desired,
            destination=PageDestinationV1("page:1"),
        )
        self.assertEqual("contained", plan.status)
        self.assertEqual(desired, plan.destination_local_rect)
        self.assertEqual(desired, plan.desired_effective_page_rect)

    def test_group_destination_returns_exact_local_rect(self):
        desired = RectEmu(120, 140, 20, 30)
        plan = plan_container_create_placement_v1(
            desired_effective_page_rect=desired,
            destination=GroupDestinationV1(
                page_id="page:1",
                group_id="g",
                ancestry=(edge("g"),),
            ),
        )
        self.assertEqual("contained", plan.status)
        self.assertEqual(RectEmu(20, 40, 20, 30), plan.destination_local_rect)

    def test_scaled_group_exact_mapping(self):
        desired = RectEmu(120, 120, 40, 20)
        plan = plan_container_create_placement_v1(
            desired_effective_page_rect=desired,
            destination=GroupDestinationV1(
                page_id="page:1",
                group_id="g",
                ancestry=(
                    edge(
                        "g",
                        bounds=RectEmu(100, 100, 200, 100),
                        local=RectEmu(0, 0, 100, 50),
                    ),
                ),
            ),
        )
        self.assertEqual("contained", plan.status)
        self.assertEqual(RectEmu(10, 10, 20, 10), plan.destination_local_rect)

    def test_inexact_integer_mapping_is_explicit(self):
        plan = plan_container_create_placement_v1(
            desired_effective_page_rect=RectEmu(1, 1, 1, 1),
            destination=GroupDestinationV1(
                page_id="page:1",
                group_id="g",
                ancestry=(
                    edge(
                        "g",
                        bounds=RectEmu(0, 0, 3, 3),
                        local=RectEmu(0, 0, 2, 2),
                    ),
                ),
            ),
        )
        self.assertEqual("not_exactly_representable", plan.status)
        self.assertIsNone(plan.destination_local_rect)

    def test_exact_but_outside_envelope_requests_refit(self):
        desired = RectEmu(290, 120, 20, 10)
        plan = plan_container_create_placement_v1(
            desired_effective_page_rect=desired,
            destination=GroupDestinationV1(
                page_id="page:1",
                group_id="g",
                ancestry=(edge("g"),),
            ),
        )
        self.assertEqual("destination_refit_required", plan.status)
        self.assertEqual(RectEmu(190, 20, 20, 10), plan.destination_local_rect)
        self.assertEqual(desired, plan.desired_effective_page_rect)

    def test_invalid_rect_fails_closed(self):
        with self.assertRaises(ContainerCreatePlacementError):
            plan_container_create_placement_v1(
                desired_effective_page_rect=RectEmu(0, 0, 0, 10),
                destination=PageDestinationV1("page:1"),
            )


if __name__ == "__main__":
    unittest.main()
