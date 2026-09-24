#!/usr/bin/env python3
"""Exact Page/Group-local placement plan for one page-space creation box."""

from __future__ import annotations

from dataclasses import dataclass

from authored_group_geometry_v1 import RectEmu, _validate_rect
from fragment_container_placement_v1 import (
    FragmentContainerPlacementError,
    FragmentMemberV2,
    FragmentV2,
    GroupDestinationV1,
    PageDestinationV1,
    PointEmu,
    plan_fragment_container_placement_v1,
)


class ContainerCreatePlacementError(ValueError):
    pass


@dataclass(frozen=True)
class ContainerCreatePlacementPlanV1:
    destination_kind: str
    destination_id: str
    page_id: str
    desired_effective_page_rect: RectEmu
    destination_local_rect: RectEmu | None
    status: str
    diagnostic: str | None = None


def plan_container_create_placement_v1(
    *,
    desired_effective_page_rect: RectEmu,
    destination: PageDestinationV1 | GroupDestinationV1,
) -> ContainerCreatePlacementPlanV1:
    try:
        _validate_rect(desired_effective_page_rect, "desired_effective_page_rect")
    except Exception as exc:
        raise ContainerCreatePlacementError(str(exc)) from exc

    synthetic_fragment = FragmentV2(
        (
            FragmentMemberV2(
                member_key="creation-box",
                relative_effective_page_bounds=RectEmu(
                    0,
                    0,
                    desired_effective_page_rect.width,
                    desired_effective_page_rect.height,
                ),
            ),
        )
    )
    try:
        fragment_plan = plan_fragment_container_placement_v1(
            fragment=synthetic_fragment,
            destination=destination,
            desired_origin_page=PointEmu(
                desired_effective_page_rect.x,
                desired_effective_page_rect.y,
            ),
        )
    except FragmentContainerPlacementError as exc:
        raise ContainerCreatePlacementError(str(exc)) from exc

    if isinstance(destination, PageDestinationV1):
        member = fragment_plan.members[0]
        if member.destination_local_bounds != desired_effective_page_rect:
            raise ContainerCreatePlacementError("page destination identity invariant failed")
        return ContainerCreatePlacementPlanV1(
            destination_kind="page",
            destination_id=destination.page_id,
            page_id=destination.page_id,
            desired_effective_page_rect=desired_effective_page_rect,
            destination_local_rect=desired_effective_page_rect,
            status="contained",
        )

    if fragment_plan.status == "not_exactly_representable":
        return ContainerCreatePlacementPlanV1(
            destination_kind="group",
            destination_id=destination.group_id,
            page_id=destination.page_id,
            desired_effective_page_rect=desired_effective_page_rect,
            destination_local_rect=None,
            status="not_exactly_representable",
            diagnostic=fragment_plan.diagnostic,
        )

    if fragment_plan.status == "destination_refit_required":
        member = fragment_plan.members[0]
        return ContainerCreatePlacementPlanV1(
            destination_kind="group",
            destination_id=destination.group_id,
            page_id=destination.page_id,
            desired_effective_page_rect=desired_effective_page_rect,
            destination_local_rect=member.destination_local_bounds,
            status="destination_refit_required",
            diagnostic=fragment_plan.diagnostic,
        )

    if fragment_plan.status != "planned" or len(fragment_plan.members) != 1:
        raise ContainerCreatePlacementError("unexpected fragment placement result")

    member = fragment_plan.members[0]
    if member.verified_effective_page_bounds != desired_effective_page_rect:
        raise ContainerCreatePlacementError("group round-trip invariant failed")
    return ContainerCreatePlacementPlanV1(
        destination_kind="group",
        destination_id=destination.group_id,
        page_id=destination.page_id,
        desired_effective_page_rect=desired_effective_page_rect,
        destination_local_rect=member.destination_local_bounds,
        status="contained",
    )
