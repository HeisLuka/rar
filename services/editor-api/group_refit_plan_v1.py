#!/usr/bin/env python3
"""Pure authored Group envelope refit planner V1.

This module changes only the parameterization of one Group geometry envelope.
It materializes current direct-child effective rectangles with the already
proven authored Group one-boundary mapping law, chooses a new envelope, rebases
direct-child local rectangles into that envelope, then proves that every
rebased child reprojects exactly to the required effective rectangle.

No graph mutation, z-order, selection state, recursive ancestor mutation,
floating-point transform, or native Publisher semantics live here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from authored_group_geometry_v1 import (
    AuthoredGroupGeometryError,
    RectEmu,
    _checked_sub,
    _validate_local_rect,
    _validate_rect,
    materialize_group_child_rect_v1,
)


class GroupRefitPlanError(ValueError):
    pass


GroupRefitModeV1 = Literal[
    "tight_fit_current_contents",
    "expand_only_with_proposed_child_rect",
]
GroupRefitStatusV1 = Literal[
    "planned",
    "no_envelope_change",
    "ancestor_refit_required",
]


@dataclass(frozen=True)
class GroupRefitChildV1:
    node_id: str
    local_rect: RectEmu


@dataclass(frozen=True)
class GroupRefitChildResultV1:
    node_id: str
    local_rect: RectEmu
    effective_parent_rect: RectEmu


@dataclass(frozen=True)
class GroupRefitPlanV1:
    mode: GroupRefitModeV1
    status: GroupRefitStatusV1
    old_group_bounds: RectEmu
    old_local_coordinate_space: RectEmu
    new_group_bounds: RectEmu
    new_local_coordinate_space: RectEmu
    children: tuple[GroupRefitChildResultV1, ...]
    target_node_id: str | None = None


def _fail(message: str) -> None:
    raise GroupRefitPlanError(message)


def _validate_group_state(
    *,
    group_bounds: RectEmu,
    local_coordinate_space: RectEmu,
    children: tuple[GroupRefitChildV1, ...],
) -> None:
    try:
        _validate_rect(group_bounds, "group_bounds")
        _validate_rect(local_coordinate_space, "local_coordinate_space")
    except AuthoredGroupGeometryError as exc:
        raise GroupRefitPlanError(str(exc)) from exc
    if local_coordinate_space.x != 0 or local_coordinate_space.y != 0:
        _fail("V1 local coordinate space origin must be zero")
    if not isinstance(children, tuple) or not children:
        _fail("GroupRefitPlanV1 requires at least one direct child")

    node_ids: list[str] = []
    for index, child in enumerate(children):
        if not isinstance(child, GroupRefitChildV1):
            _fail(f"children[{index}] must be GroupRefitChildV1")
        if not isinstance(child.node_id, str) or not child.node_id:
            _fail(f"children[{index}].node_id is required")
        node_ids.append(child.node_id)
        try:
            _validate_local_rect(
                child.local_rect,
                local_coordinate_space,
                f"children[{index}].local_rect",
            )
        except AuthoredGroupGeometryError as exc:
            raise GroupRefitPlanError(str(exc)) from exc
    if len(set(node_ids)) != len(node_ids):
        _fail("GroupRefitPlanV1 child NodeIds must be unique")


def _materialize_children(
    *,
    group_bounds: RectEmu,
    local_coordinate_space: RectEmu,
    children: tuple[GroupRefitChildV1, ...],
) -> tuple[GroupRefitChildResultV1, ...]:
    out: list[GroupRefitChildResultV1] = []
    for child in children:
        try:
            effective = materialize_group_child_rect_v1(
                local_coordinate_space=local_coordinate_space,
                child_local_bounds=child.local_rect,
                current_group_bounds=group_bounds,
            )
        except AuthoredGroupGeometryError as exc:
            raise GroupRefitPlanError(str(exc)) from exc
        out.append(
            GroupRefitChildResultV1(
                node_id=child.node_id,
                local_rect=child.local_rect,
                effective_parent_rect=effective,
            )
        )
    return tuple(out)


def _union_rects(rects: tuple[RectEmu, ...]) -> RectEmu:
    if not rects:
        _fail("cannot union an empty rectangle set")
    for index, rect in enumerate(rects):
        try:
            _validate_rect(rect, f"union[{index}]")
        except AuthoredGroupGeometryError as exc:
            raise GroupRefitPlanError(str(exc)) from exc
    left = min(rect.x for rect in rects)
    top = min(rect.y for rect in rects)
    right = max(rect.right for rect in rects)
    bottom = max(rect.bottom for rect in rects)
    try:
        width = _checked_sub(right, left, "union.width")
        height = _checked_sub(bottom, top, "union.height")
    except AuthoredGroupGeometryError as exc:
        raise GroupRefitPlanError(str(exc)) from exc
    if width <= 0 or height <= 0:
        _fail("refit union must remain positive")
    return RectEmu(left, top, width, height)


def _contains(outer: RectEmu, inner: RectEmu) -> bool:
    return (
        inner.x >= outer.x
        and inner.y >= outer.y
        and inner.right <= outer.right
        and inner.bottom <= outer.bottom
    )


def _rebase_exact(
    *,
    envelope: RectEmu,
    required_effective: tuple[GroupRefitChildResultV1, ...],
) -> tuple[GroupRefitChildResultV1, ...]:
    new_local_space = RectEmu(0, 0, envelope.width, envelope.height)
    rebased: list[GroupRefitChildResultV1] = []

    for child in required_effective:
        effective = child.effective_parent_rect
        try:
            local = RectEmu(
                _checked_sub(effective.x, envelope.x, "rebased.local.x"),
                _checked_sub(effective.y, envelope.y, "rebased.local.y"),
                effective.width,
                effective.height,
            )
            _validate_local_rect(
                local,
                new_local_space,
                f"rebased[{child.node_id}]",
            )
            reprojected = materialize_group_child_rect_v1(
                local_coordinate_space=new_local_space,
                child_local_bounds=local,
                current_group_bounds=envelope,
            )
        except AuthoredGroupGeometryError as exc:
            raise GroupRefitPlanError(str(exc)) from exc

        if reprojected != effective:
            _fail(
                f"rebased child {child.node_id!r} is not exactly representable "
                "under proposed Group parameterization"
            )
        rebased.append(
            GroupRefitChildResultV1(
                node_id=child.node_id,
                local_rect=local,
                effective_parent_rect=effective,
            )
        )
    return tuple(rebased)


def _ancestor_status(
    *,
    envelope: RectEmu,
    parent_local_coordinate_space: RectEmu | None,
) -> GroupRefitStatusV1:
    if parent_local_coordinate_space is None:
        return "planned"
    try:
        _validate_rect(
            parent_local_coordinate_space,
            "parent_local_coordinate_space",
        )
    except AuthoredGroupGeometryError as exc:
        raise GroupRefitPlanError(str(exc)) from exc
    if (
        parent_local_coordinate_space.x != 0
        or parent_local_coordinate_space.y != 0
    ):
        _fail("parent local coordinate space origin must be zero")
    if not _contains(parent_local_coordinate_space, envelope):
        return "ancestor_refit_required"
    return "planned"


def plan_group_refit_v1(
    *,
    mode: GroupRefitModeV1,
    group_bounds: RectEmu,
    local_coordinate_space: RectEmu,
    children: tuple[GroupRefitChildV1, ...],
    target_node_id: str | None = None,
    desired_target_parent_rect: RectEmu | None = None,
    parent_local_coordinate_space: RectEmu | None = None,
) -> GroupRefitPlanV1:
    """Plan one Group envelope rebase without mutating graph state."""

    if mode not in {
        "tight_fit_current_contents",
        "expand_only_with_proposed_child_rect",
    }:
        _fail("unsupported GroupRefitPlanV1 mode")

    _validate_group_state(
        group_bounds=group_bounds,
        local_coordinate_space=local_coordinate_space,
        children=children,
    )
    materialized = _materialize_children(
        group_bounds=group_bounds,
        local_coordinate_space=local_coordinate_space,
        children=children,
    )

    if mode == "tight_fit_current_contents":
        if target_node_id is not None or desired_target_parent_rect is not None:
            _fail("tight-fit mode does not accept a target override")
        envelope = _union_rects(
            tuple(child.effective_parent_rect for child in materialized)
        )
        required_effective = materialized

    else:
        if not isinstance(target_node_id, str) or not target_node_id:
            _fail("expand-only mode requires target_node_id")
        if desired_target_parent_rect is None:
            _fail("expand-only mode requires desired_target_parent_rect")
        try:
            _validate_rect(
                desired_target_parent_rect,
                "desired_target_parent_rect",
            )
        except AuthoredGroupGeometryError as exc:
            raise GroupRefitPlanError(str(exc)) from exc

        by_id = {child.node_id: child for child in materialized}
        if target_node_id not in by_id:
            _fail("expand-only target is not a current direct child")

        if _contains(group_bounds, desired_target_parent_rect):
            return GroupRefitPlanV1(
                mode=mode,
                status="no_envelope_change",
                old_group_bounds=group_bounds,
                old_local_coordinate_space=local_coordinate_space,
                new_group_bounds=group_bounds,
                new_local_coordinate_space=local_coordinate_space,
                children=materialized,
                target_node_id=target_node_id,
            )

        envelope = _union_rects((group_bounds, desired_target_parent_rect))
        required_list: list[GroupRefitChildResultV1] = []
        for child in materialized:
            effective = (
                desired_target_parent_rect
                if child.node_id == target_node_id
                else child.effective_parent_rect
            )
            required_list.append(
                GroupRefitChildResultV1(
                    node_id=child.node_id,
                    local_rect=child.local_rect,
                    effective_parent_rect=effective,
                )
            )
        required_effective = tuple(required_list)

        if (
            envelope.x > group_bounds.x
            or envelope.y > group_bounds.y
            or envelope.right < group_bounds.right
            or envelope.bottom < group_bounds.bottom
        ):
            _fail("expand-only mode must never shrink the current Group envelope")

    rebased = _rebase_exact(
        envelope=envelope,
        required_effective=required_effective,
    )
    status = _ancestor_status(
        envelope=envelope,
        parent_local_coordinate_space=parent_local_coordinate_space,
    )

    return GroupRefitPlanV1(
        mode=mode,
        status=status,
        old_group_bounds=group_bounds,
        old_local_coordinate_space=local_coordinate_space,
        new_group_bounds=envelope,
        new_local_coordinate_space=RectEmu(
            0,
            0,
            envelope.width,
            envelope.height,
        ),
        children=rebased,
        target_node_id=target_node_id,
    )
