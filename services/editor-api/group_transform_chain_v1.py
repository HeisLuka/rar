#!/usr/bin/env python3
"""Exact recursive authored-Group transform chain V1.

This module composes the already-proven one-boundary authored Group geometry
law. It deliberately owns no graph mutation, layout recursion, renderer state,
or native Publisher semantics.

Path order is outermost Group -> immediate parent Group. Forward projection
therefore applies boundaries from inner to outer. Inverse projection applies
the same boundaries from outer to inner and then forward-reprojects the
canonical local result.
"""

from __future__ import annotations

from dataclasses import dataclass

from authored_group_geometry_v1 import (
    AuthoredGroupGeometryError,
    RectEmu,
    _checked_int,
    _round_ratio_nearest_emu,
    _validate_local_rect,
    _validate_rect,
    materialize_group_child_rect_v1,
)

MAX_AUTHORED_GROUP_DEPTH_V1 = 8
AUTHORED_GROUP_PROVENANCE_V1 = "chaptera-authored-group-v1"


class GroupTransformChainError(ValueError):
    pass


@dataclass(frozen=True)
class AuthoredGroupEdgeV1:
    group_id: str
    page_id: str
    parent_group_id: str | None
    children: tuple[str, ...]
    bounds_in_parent: RectEmu
    local_coordinate_space: RectEmu
    provenance: str = AUTHORED_GROUP_PROVENANCE_V1


@dataclass(frozen=True)
class GroupTransformStepV1:
    group_id: str
    source_space: str
    target_space: str
    source_rect: RectEmu
    target_rect: RectEmu


@dataclass(frozen=True)
class GroupTransformProjectionV1:
    target_id: str
    page_id: str
    group_path: tuple[str, ...]
    canonical_local_rect: RectEmu
    effective_page_rect: RectEmu
    steps: tuple[GroupTransformStepV1, ...]


@dataclass(frozen=True)
class GroupTransformInverseV1:
    target_id: str
    page_id: str
    group_path: tuple[str, ...]
    requested_page_rect: RectEmu
    canonical_local_rect: RectEmu
    effective_page_rect: RectEmu
    inverse_steps: tuple[GroupTransformStepV1, ...]
    forward_steps: tuple[GroupTransformStepV1, ...]


@dataclass(frozen=True)
class GroupPointEmu:
    x: int
    y: int


@dataclass(frozen=True)
class GroupPointTransformV1:
    target_id: str
    page_id: str
    group_path: tuple[str, ...]
    canonical_local_point: GroupPointEmu
    effective_page_point: GroupPointEmu


def _fail(message: str) -> None:
    raise GroupTransformChainError(message)


def _require_id(value: str, label: str) -> None:
    if not isinstance(value, str) or not value:
        _fail(f"{label} is required")


def _validate_edge(edge: AuthoredGroupEdgeV1, index: int) -> None:
    if not isinstance(edge, AuthoredGroupEdgeV1):
        _fail(f"ancestry[{index}] must be AuthoredGroupEdgeV1")
    _require_id(edge.group_id, f"ancestry[{index}].group_id")
    _require_id(edge.page_id, f"ancestry[{index}].page_id")
    if edge.parent_group_id is not None:
        _require_id(edge.parent_group_id, f"ancestry[{index}].parent_group_id")
    if edge.provenance != AUTHORED_GROUP_PROVENANCE_V1:
        _fail(f"ancestry[{index}] is not admitted authored Group provenance")
    if not isinstance(edge.children, tuple):
        _fail(f"ancestry[{index}].children must be an ordered tuple")
    if not edge.children:
        _fail(f"ancestry[{index}].children must not be empty")
    for child_index, child_id in enumerate(edge.children):
        _require_id(child_id, f"ancestry[{index}].children[{child_index}]")
    if len(set(edge.children)) != len(edge.children):
        _fail(f"ancestry[{index}] has duplicate children")
    try:
        _validate_rect(edge.bounds_in_parent, f"ancestry[{index}].bounds_in_parent")
        _validate_rect(
            edge.local_coordinate_space,
            f"ancestry[{index}].local_coordinate_space",
        )
    except AuthoredGroupGeometryError as exc:
        raise GroupTransformChainError(str(exc)) from exc
    if edge.local_coordinate_space.x != 0 or edge.local_coordinate_space.y != 0:
        _fail(f"ancestry[{index}] local coordinate space origin must be zero")


def validate_group_transform_chain_v1(
    *,
    target_id: str,
    target_page_id: str,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
) -> None:
    _require_id(target_id, "target_id")
    _require_id(target_page_id, "target_page_id")
    if not isinstance(ancestry, tuple):
        _fail("ancestry must be an ordered tuple")
    if len(ancestry) > MAX_AUTHORED_GROUP_DEPTH_V1:
        _fail(
            f"authored Group depth {len(ancestry)} exceeds "
            f"MAX_AUTHORED_GROUP_DEPTH_V1={MAX_AUTHORED_GROUP_DEPTH_V1}"
        )
    if not ancestry:
        return

    for index, edge in enumerate(ancestry):
        _validate_edge(edge, index)

    group_ids = tuple(edge.group_id for edge in ancestry)
    if len(set(group_ids)) != len(group_ids):
        _fail("authored Group ancestry contains a cycle/repeated Group")
    if target_id in set(group_ids):
        _fail("target_id cannot also be an ancestor Group")
    if any(edge.page_id != target_page_id for edge in ancestry):
        _fail("all authored Group ancestors must share the target page")

    for index, edge in enumerate(ancestry):
        expected_parent = None if index == 0 else ancestry[index - 1].group_id
        if edge.parent_group_id != expected_parent:
            _fail(
                f"ancestry[{index}] parent mismatch: expected "
                f"{expected_parent!r}, got {edge.parent_group_id!r}"
            )

        expected_child = (
            target_id
            if index == len(ancestry) - 1
            else ancestry[index + 1].group_id
        )
        if expected_child not in edge.children:
            _fail(
                f"ancestry[{index}] does not contain expected child "
                f"{expected_child!r}"
            )

        if index > 0:
            parent = ancestry[index - 1]
            try:
                _validate_local_rect(
                    edge.bounds_in_parent,
                    parent.local_coordinate_space,
                    f"ancestry[{index}].bounds_in_parent",
                )
            except AuthoredGroupGeometryError as exc:
                raise GroupTransformChainError(str(exc)) from exc


def project_group_transform_chain_v1(
    *,
    target_id: str,
    target_page_id: str,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    target_local_rect: RectEmu,
) -> GroupTransformProjectionV1:
    validate_group_transform_chain_v1(
        target_id=target_id,
        target_page_id=target_page_id,
        ancestry=ancestry,
    )
    try:
        _validate_rect(target_local_rect, "target_local_rect")
    except AuthoredGroupGeometryError as exc:
        raise GroupTransformChainError(str(exc)) from exc

    if not ancestry:
        return GroupTransformProjectionV1(
            target_id=target_id,
            page_id=target_page_id,
            group_path=(),
            canonical_local_rect=target_local_rect,
            effective_page_rect=target_local_rect,
            steps=(),
        )

    current = target_local_rect
    steps: list[GroupTransformStepV1] = []
    for reverse_index, edge in enumerate(reversed(ancestry)):
        path_index = len(ancestry) - 1 - reverse_index
        source_space = f"group:{edge.group_id}:local"
        target_space = (
            "page"
            if path_index == 0
            else f"group:{ancestry[path_index - 1].group_id}:local"
        )
        before = current
        try:
            current = materialize_group_child_rect_v1(
                local_coordinate_space=edge.local_coordinate_space,
                child_local_bounds=before,
                current_group_bounds=edge.bounds_in_parent,
            )
        except AuthoredGroupGeometryError as exc:
            raise GroupTransformChainError(str(exc)) from exc
        steps.append(
            GroupTransformStepV1(
                group_id=edge.group_id,
                source_space=source_space,
                target_space=target_space,
                source_rect=before,
                target_rect=current,
            )
        )

    return GroupTransformProjectionV1(
        target_id=target_id,
        page_id=target_page_id,
        group_path=tuple(edge.group_id for edge in ancestry),
        canonical_local_rect=target_local_rect,
        effective_page_rect=current,
        steps=tuple(steps),
    )


def _inverse_boundary_v1(
    *,
    parent_rect: RectEmu,
    edge: AuthoredGroupEdgeV1,
    label: str,
) -> RectEmu:
    try:
        _validate_rect(parent_rect, f"{label}.parent_rect")
    except AuthoredGroupGeometryError as exc:
        raise GroupTransformChainError(str(exc)) from exc

    group_bounds = edge.bounds_in_parent
    local_space = edge.local_coordinate_space

    if (
        parent_rect.x < group_bounds.x
        or parent_rect.y < group_bounds.y
        or parent_rect.right > group_bounds.right
        or parent_rect.bottom > group_bounds.bottom
    ):
        _fail(f"{label} requested rect lies outside Group bounds")

    rel_left = parent_rect.x - group_bounds.x
    rel_right = parent_rect.right - group_bounds.x
    rel_top = parent_rect.y - group_bounds.y
    rel_bottom = parent_rect.bottom - group_bounds.y

    def map_x(relative_edge: int) -> int:
        try:
            return _round_ratio_nearest_emu(
                _checked_int(
                    relative_edge * local_space.width,
                    f"{label}.inverse_x_numerator",
                ),
                group_bounds.width,
            )
        except AuthoredGroupGeometryError as exc:
            raise GroupTransformChainError(str(exc)) from exc

    def map_y(relative_edge: int) -> int:
        try:
            return _round_ratio_nearest_emu(
                _checked_int(
                    relative_edge * local_space.height,
                    f"{label}.inverse_y_numerator",
                ),
                group_bounds.height,
            )
        except AuthoredGroupGeometryError as exc:
            raise GroupTransformChainError(str(exc)) from exc

    left = map_x(rel_left)
    right = map_x(rel_right)
    top = map_y(rel_top)
    bottom = map_y(rel_bottom)
    width = right - left
    height = bottom - top
    if width <= 0 or height <= 0:
        _fail(f"{label} inverse mapping collapsed to non-positive RectEMU")

    local_rect = RectEmu(left, top, width, height)
    try:
        _validate_local_rect(local_rect, local_space, f"{label}.local_rect")
    except AuthoredGroupGeometryError as exc:
        raise GroupTransformChainError(str(exc)) from exc
    return local_rect


def inverse_group_transform_chain_v1(
    *,
    target_id: str,
    target_page_id: str,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    desired_page_rect: RectEmu,
) -> GroupTransformInverseV1:
    validate_group_transform_chain_v1(
        target_id=target_id,
        target_page_id=target_page_id,
        ancestry=ancestry,
    )
    try:
        _validate_rect(desired_page_rect, "desired_page_rect")
    except AuthoredGroupGeometryError as exc:
        raise GroupTransformChainError(str(exc)) from exc

    if not ancestry:
        return GroupTransformInverseV1(
            target_id=target_id,
            page_id=target_page_id,
            group_path=(),
            requested_page_rect=desired_page_rect,
            canonical_local_rect=desired_page_rect,
            effective_page_rect=desired_page_rect,
            inverse_steps=(),
            forward_steps=(),
        )

    current = desired_page_rect
    inverse_steps: list[GroupTransformStepV1] = []
    for index, edge in enumerate(ancestry):
        source_space = (
            "page"
            if index == 0
            else f"group:{ancestry[index - 1].group_id}:local"
        )
        target_space = f"group:{edge.group_id}:local"
        before = current
        current = _inverse_boundary_v1(
            parent_rect=before,
            edge=edge,
            label=f"ancestry[{index}]",
        )
        inverse_steps.append(
            GroupTransformStepV1(
                group_id=edge.group_id,
                source_space=source_space,
                target_space=target_space,
                source_rect=before,
                target_rect=current,
            )
        )

    projected = project_group_transform_chain_v1(
        target_id=target_id,
        target_page_id=target_page_id,
        ancestry=ancestry,
        target_local_rect=current,
    )
    return GroupTransformInverseV1(
        target_id=target_id,
        page_id=target_page_id,
        group_path=projected.group_path,
        requested_page_rect=desired_page_rect,
        canonical_local_rect=current,
        effective_page_rect=projected.effective_page_rect,
        inverse_steps=tuple(inverse_steps),
        forward_steps=projected.steps,
    )


def _round_signed_ratio_v1(
    numerator: int,
    denominator: int,
    label: str,
) -> int:
    if not isinstance(numerator, int) or isinstance(numerator, bool):
        _fail(f"{label}.numerator must be an integer")
    try:
        sign = -1 if numerator < 0 else 1
        magnitude = _round_ratio_nearest_emu(abs(numerator), denominator)
        return _checked_int(sign * magnitude, label)
    except AuthoredGroupGeometryError as exc:
        raise GroupTransformChainError(str(exc)) from exc


def _validate_point_v1(point: GroupPointEmu, label: str) -> None:
    if not isinstance(point, GroupPointEmu):
        _fail(f"{label} must be GroupPointEmu")
    try:
        _checked_int(point.x, f"{label}.x")
        _checked_int(point.y, f"{label}.y")
    except AuthoredGroupGeometryError as exc:
        raise GroupTransformChainError(str(exc)) from exc


def _project_point_boundary_v1(
    *,
    local_point: GroupPointEmu,
    edge: AuthoredGroupEdgeV1,
    label: str,
) -> GroupPointEmu:
    _validate_point_v1(local_point, f"{label}.local_point")
    x = edge.bounds_in_parent.x + _round_signed_ratio_v1(
        local_point.x * edge.bounds_in_parent.width,
        edge.local_coordinate_space.width,
        f"{label}.x",
    )
    y = edge.bounds_in_parent.y + _round_signed_ratio_v1(
        local_point.y * edge.bounds_in_parent.height,
        edge.local_coordinate_space.height,
        f"{label}.y",
    )
    try:
        x = _checked_int(x, f"{label}.projected.x")
        y = _checked_int(y, f"{label}.projected.y")
    except AuthoredGroupGeometryError as exc:
        raise GroupTransformChainError(str(exc)) from exc
    return GroupPointEmu(x, y)


def _inverse_point_boundary_v1(
    *,
    parent_point: GroupPointEmu,
    edge: AuthoredGroupEdgeV1,
    label: str,
) -> GroupPointEmu:
    _validate_point_v1(parent_point, f"{label}.parent_point")
    x = _round_signed_ratio_v1(
        (parent_point.x - edge.bounds_in_parent.x)
        * edge.local_coordinate_space.width,
        edge.bounds_in_parent.width,
        f"{label}.x",
    )
    y = _round_signed_ratio_v1(
        (parent_point.y - edge.bounds_in_parent.y)
        * edge.local_coordinate_space.height,
        edge.bounds_in_parent.height,
        f"{label}.y",
    )
    return GroupPointEmu(x, y)


def project_group_transform_point_v1(
    *,
    target_id: str,
    target_page_id: str,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    local_point: GroupPointEmu,
) -> GroupPointTransformV1:
    validate_group_transform_chain_v1(
        target_id=target_id,
        target_page_id=target_page_id,
        ancestry=ancestry,
    )
    _validate_point_v1(local_point, "local_point")
    current = local_point
    for reverse_index, edge in enumerate(reversed(ancestry)):
        current = _project_point_boundary_v1(
            local_point=current,
            edge=edge,
            label=f"ancestry[{len(ancestry) - 1 - reverse_index}]",
        )
    return GroupPointTransformV1(
        target_id=target_id,
        page_id=target_page_id,
        group_path=tuple(edge.group_id for edge in ancestry),
        canonical_local_point=local_point,
        effective_page_point=current,
    )


def inverse_group_transform_point_v1(
    *,
    target_id: str,
    target_page_id: str,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    page_point: GroupPointEmu,
) -> GroupPointTransformV1:
    validate_group_transform_chain_v1(
        target_id=target_id,
        target_page_id=target_page_id,
        ancestry=ancestry,
    )
    _validate_point_v1(page_point, "page_point")
    current = page_point
    for index, edge in enumerate(ancestry):
        current = _inverse_point_boundary_v1(
            parent_point=current,
            edge=edge,
            label=f"ancestry[{index}]",
        )

    projected = project_group_transform_point_v1(
        target_id=target_id,
        target_page_id=target_page_id,
        ancestry=ancestry,
        local_point=current,
    )
    if projected.effective_page_point != page_point:
        _fail("page point is not exactly representable through GroupTransformChainV1")

    return GroupPointTransformV1(
        target_id=target_id,
        page_id=target_page_id,
        group_path=projected.group_path,
        canonical_local_point=current,
        effective_page_point=projected.effective_page_point,
    )
