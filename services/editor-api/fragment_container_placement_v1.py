#!/usr/bin/env python3
"""Exact Fragment V2 placement planning for Page/Group destinations.

This planner consumes frozen AuthoredGroupEdgeV1 transform state and reuses
GroupTransformChainV1's exact signed-rational rounding law. It allocates no
document IDs and mutates no graph or authored stack.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal, TypeAlias

from authored_group_geometry_v1 import RectEmu, _checked_int, _validate_rect
from group_transform_chain_v1 import (
    MAX_AUTHORED_GROUP_DEPTH_V1,
    AuthoredGroupEdgeV1,
    GroupTransformChainError,
    _round_signed_ratio_v1,
    _validate_edge,
)


class FragmentContainerPlacementError(ValueError):
    pass


@dataclass(frozen=True)
class PointEmu:
    x: int
    y: int


@dataclass(frozen=True)
class FragmentMemberV2:
    member_key: str
    relative_effective_page_bounds: RectEmu


@dataclass(frozen=True)
class FragmentV2:
    members: tuple[FragmentMemberV2, ...]


@dataclass(frozen=True)
class PageDestinationV1:
    page_id: str
    kind: Literal["page"] = "page"


@dataclass(frozen=True)
class GroupDestinationV1:
    page_id: str
    group_id: str
    ancestry: tuple[AuthoredGroupEdgeV1, ...]
    kind: Literal["group"] = "group"


FragmentDestinationV1: TypeAlias = PageDestinationV1 | GroupDestinationV1


@dataclass(frozen=True)
class FragmentPlacementMemberV1:
    member_key: str
    desired_effective_page_bounds: RectEmu
    destination_local_bounds: RectEmu
    verified_effective_page_bounds: RectEmu


@dataclass(frozen=True)
class FragmentContainerPlacementPlanV1:
    destination_kind: str
    destination_id: str
    page_id: str
    desired_origin_page: PointEmu
    members: tuple[FragmentPlacementMemberV1, ...]
    status: str
    diagnostic: str | None = None


def _fail(message: str) -> None:
    raise FragmentContainerPlacementError(message)


def _id(value: str, label: str) -> str:
    if not isinstance(value, str) or not value:
        _fail(f"{label} is required")
    return value


def _checked_point(point: PointEmu, label: str) -> PointEmu:
    if not isinstance(point, PointEmu):
        _fail(f"{label} must be PointEmu")
    try:
        _checked_int(point.x, f"{label}.x")
        _checked_int(point.y, f"{label}.y")
    except Exception as exc:
        raise FragmentContainerPlacementError(str(exc)) from exc
    return point


def _validate_fragment(fragment: FragmentV2) -> None:
    if not isinstance(fragment, FragmentV2):
        _fail("fragment must be FragmentV2")
    if not isinstance(fragment.members, tuple) or not fragment.members:
        _fail("fragment.members must be a non-empty ordered tuple")
    keys = []
    for index, member in enumerate(fragment.members):
        if not isinstance(member, FragmentMemberV2):
            _fail(f"fragment.members[{index}] must be FragmentMemberV2")
        _id(member.member_key, f"fragment.members[{index}].member_key")
        try:
            _validate_rect(
                member.relative_effective_page_bounds,
                f"fragment.members[{index}].relative_effective_page_bounds",
            )
        except Exception as exc:
            raise FragmentContainerPlacementError(str(exc)) from exc
        keys.append(member.member_key)
    if len(set(keys)) != len(keys):
        _fail("fragment member keys must be unique")


def _validate_destination_chain(destination: GroupDestinationV1) -> None:
    _id(destination.page_id, "destination.page_id")
    _id(destination.group_id, "destination.group_id")
    if not isinstance(destination.ancestry, tuple) or not destination.ancestry:
        _fail("Group destination requires non-empty ancestry")
    if len(destination.ancestry) > MAX_AUTHORED_GROUP_DEPTH_V1:
        _fail("Group destination exceeds MAX_AUTHORED_GROUP_DEPTH_V1")

    for index, edge in enumerate(destination.ancestry):
        try:
            _validate_edge(edge, index)
        except GroupTransformChainError as exc:
            raise FragmentContainerPlacementError(str(exc)) from exc
    ids = tuple(edge.group_id for edge in destination.ancestry)
    if len(set(ids)) != len(ids):
        _fail("destination ancestry contains repeated Group")
    if ids[-1] != destination.group_id:
        _fail("destination.group_id must equal innermost ancestry Group")
    if any(edge.page_id != destination.page_id for edge in destination.ancestry):
        _fail("destination ancestry must stay on one page")
    for index, edge in enumerate(destination.ancestry):
        expected_parent = None if index == 0 else destination.ancestry[index - 1].group_id
        if edge.parent_group_id != expected_parent:
            _fail(f"destination ancestry[{index}] parent mismatch")


def _translate_relative(rect: RectEmu, origin: PointEmu) -> RectEmu:
    try:
        x = _checked_int(origin.x + rect.x, "desired.x")
        y = _checked_int(origin.y + rect.y, "desired.y")
        translated = RectEmu(x, y, rect.width, rect.height)
        _validate_rect(translated, "desired_effective_page_bounds")
        return translated
    except Exception as exc:
        raise FragmentContainerPlacementError(str(exc)) from exc


def _inverse_boundary_unbounded(rect: RectEmu, edge: AuthoredGroupEdgeV1) -> RectEmu:
    group = edge.bounds_in_parent
    local = edge.local_coordinate_space

    def map_x(parent_edge: int) -> int:
        return _round_signed_ratio_v1(
            (parent_edge - group.x) * local.width,
            group.width,
            "fragment.inverse.x",
        )

    def map_y(parent_edge: int) -> int:
        return _round_signed_ratio_v1(
            (parent_edge - group.y) * local.height,
            group.height,
            "fragment.inverse.y",
        )

    left = map_x(rect.x)
    right = map_x(rect.right)
    top = map_y(rect.y)
    bottom = map_y(rect.bottom)
    width = right - left
    height = bottom - top
    if width <= 0 or height <= 0:
        raise FragmentContainerPlacementError("inverse mapping collapsed RectEMU")
    result = RectEmu(left, top, width, height)
    try:
        _validate_rect(result, "destination_local_bounds")
    except Exception as exc:
        raise FragmentContainerPlacementError(str(exc)) from exc
    return result


def _forward_boundary_unbounded(rect: RectEmu, edge: AuthoredGroupEdgeV1) -> RectEmu:
    group = edge.bounds_in_parent
    local = edge.local_coordinate_space

    def map_x(local_edge: int) -> int:
        delta = _round_signed_ratio_v1(
            local_edge * group.width,
            local.width,
            "fragment.forward.x",
        )
        return _checked_int(group.x + delta, "fragment.forward.page_x")

    def map_y(local_edge: int) -> int:
        delta = _round_signed_ratio_v1(
            local_edge * group.height,
            local.height,
            "fragment.forward.y",
        )
        return _checked_int(group.y + delta, "fragment.forward.page_y")

    left = map_x(rect.x)
    right = map_x(rect.right)
    top = map_y(rect.y)
    bottom = map_y(rect.bottom)
    width = right - left
    height = bottom - top
    if width <= 0 or height <= 0:
        raise FragmentContainerPlacementError("forward mapping collapsed RectEMU")
    result = RectEmu(left, top, width, height)
    try:
        _validate_rect(result, "verified_effective_page_bounds")
    except Exception as exc:
        raise FragmentContainerPlacementError(str(exc)) from exc
    return result


def _inverse_chain_unbounded(
    rect: RectEmu,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
) -> RectEmu:
    current = rect
    for edge in ancestry:
        current = _inverse_boundary_unbounded(current, edge)
    return current


def _forward_chain_unbounded(
    rect: RectEmu,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
) -> RectEmu:
    current = rect
    for edge in reversed(ancestry):
        current = _forward_boundary_unbounded(current, edge)
    return current


def _inside_local(rect: RectEmu, local: RectEmu) -> bool:
    return (
        rect.x >= local.x
        and rect.y >= local.y
        and rect.right <= local.right
        and rect.bottom <= local.bottom
    )


def plan_fragment_container_placement_v1(
    *,
    fragment: FragmentV2,
    destination: FragmentDestinationV1,
    desired_origin_page: PointEmu,
) -> FragmentContainerPlacementPlanV1:
    _validate_fragment(fragment)
    origin = _checked_point(desired_origin_page, "desired_origin_page")

    desired = tuple(
        (
            member,
            _translate_relative(member.relative_effective_page_bounds, origin),
        )
        for member in fragment.members
    )

    if isinstance(destination, PageDestinationV1):
        page_id = _id(destination.page_id, "destination.page_id")
        members = tuple(
            FragmentPlacementMemberV1(
                member_key=member.member_key,
                desired_effective_page_bounds=page_rect,
                destination_local_bounds=page_rect,
                verified_effective_page_bounds=page_rect,
            )
            for member, page_rect in desired
        )
        return FragmentContainerPlacementPlanV1(
            destination_kind="page",
            destination_id=page_id,
            page_id=page_id,
            desired_origin_page=origin,
            members=members,
            status="planned",
        )

    if not isinstance(destination, GroupDestinationV1):
        _fail("unsupported fragment destination")
    _validate_destination_chain(destination)

    planned = []
    for member, desired_page in desired:
        try:
            local_rect = _inverse_chain_unbounded(desired_page, destination.ancestry)
            verified = _forward_chain_unbounded(local_rect, destination.ancestry)
        except FragmentContainerPlacementError as exc:
            return FragmentContainerPlacementPlanV1(
                destination_kind="group",
                destination_id=destination.group_id,
                page_id=destination.page_id,
                desired_origin_page=origin,
                members=(),
                status="not_exactly_representable",
                diagnostic=f"{member.member_key}: {exc}",
            )
        if verified != desired_page:
            return FragmentContainerPlacementPlanV1(
                destination_kind="group",
                destination_id=destination.group_id,
                page_id=destination.page_id,
                desired_origin_page=origin,
                members=(),
                status="not_exactly_representable",
                diagnostic=f"{member.member_key}: inverse/forward round-trip differs",
            )
        planned.append(
            FragmentPlacementMemberV1(
                member_key=member.member_key,
                desired_effective_page_bounds=desired_page,
                destination_local_bounds=local_rect,
                verified_effective_page_bounds=verified,
            )
        )

    local_space = destination.ancestry[-1].local_coordinate_space
    outside = [
        item.member_key
        for item in planned
        if not _inside_local(item.destination_local_bounds, local_space)
    ]
    if outside:
        return FragmentContainerPlacementPlanV1(
            destination_kind="group",
            destination_id=destination.group_id,
            page_id=destination.page_id,
            desired_origin_page=origin,
            members=tuple(planned),
            status="destination_refit_required",
            diagnostic="outside unchanged destination local envelope: " + ",".join(outside),
        )

    return FragmentContainerPlacementPlanV1(
        destination_kind="group",
        destination_id=destination.group_id,
        page_id=destination.page_id,
        desired_origin_page=origin,
        members=tuple(planned),
        status="planned",
    )
