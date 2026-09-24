#!/usr/bin/env python3
"""Exact aggregate-space multi-resize planner V1.

Multi-resize is one affine mapping of the immutable selection aggregate, never
N independent object resizes. Only axes controlled by the admitted handle are
mapped; the opposite aggregate edge/corner is fixed exactly. Unaffected axes
are preserved byte-for-byte.

Interior edges use the same deterministic nearest-EMU rule as the existing
authored Group geometry primitive. No mutation, selection state, snapping,
modifier policy, Group creation or native Publisher semantics live here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from authored_group_geometry_v1 import (
    MAX_SAFE_EMU,
    MIN_SAFE_EMU,
    AuthoredGroupGeometryError,
    RectEmu,
    _checked_int,
    _round_ratio_nearest_emu,
    _validate_rect,
)


ResizeHandleV1 = Literal["n", "s", "e", "w", "ne", "nw", "se", "sw"]


class MultiResizePlanError(ValueError):
    pass


@dataclass(frozen=True)
class MultiResizeMemberV1:
    node_id: str
    rect: RectEmu
    provenance: str = "chaptera-authored"
    transform: str = "identity"


@dataclass(frozen=True)
class MultiResizeMemberResultV1:
    node_id: str
    before: RectEmu
    after: RectEmu


@dataclass(frozen=True)
class MultiResizePlanV1:
    handle: ResizeHandleV1
    base_aggregate: RectEmu
    target_aggregate: RectEmu
    members: tuple[MultiResizeMemberResultV1, ...]


def _fail(message: str) -> None:
    raise MultiResizePlanError(message)


def _checked_rect(rect: RectEmu, label: str) -> None:
    try:
        _validate_rect(rect, label)
    except AuthoredGroupGeometryError as exc:
        raise MultiResizePlanError(str(exc)) from exc


def _union_members(members: tuple[MultiResizeMemberV1, ...]) -> RectEmu:
    left = min(member.rect.x for member in members)
    top = min(member.rect.y for member in members)
    right = max(member.rect.right for member in members)
    bottom = max(member.rect.bottom for member in members)
    width = right - left
    height = bottom - top
    if width <= 0 or height <= 0:
        _fail("member union must have positive width/height")
    rect = RectEmu(left, top, width, height)
    _checked_rect(rect, "member_union")
    return rect


def _validate_members(members: tuple[MultiResizeMemberV1, ...]) -> None:
    if not isinstance(members, tuple) or len(members) < 2:
        _fail("MultiResizePlanV1 requires at least two members")

    ids: list[str] = []
    for index, member in enumerate(members):
        if not isinstance(member, MultiResizeMemberV1):
            _fail(f"members[{index}] must be MultiResizeMemberV1")
        if not isinstance(member.node_id, str) or not member.node_id:
            _fail(f"members[{index}].node_id is required")
        if member.provenance != "chaptera-authored":
            _fail(f"members[{index}] has unsupported provenance")
        if member.transform != "identity":
            _fail(f"members[{index}] has unsupported transform")
        _checked_rect(member.rect, f"members[{index}].rect")
        ids.append(member.node_id)

    if len(set(ids)) != len(ids):
        _fail("MultiResizePlanV1 member NodeIds must be unique")


def _handle_axes(handle: ResizeHandleV1) -> tuple[bool, bool]:
    if handle not in {"n", "s", "e", "w", "ne", "nw", "se", "sw"}:
        _fail("unsupported resize handle")
    return ("e" in handle or "w" in handle, "n" in handle or "s" in handle)


def _validate_fixed_edge_law(
    *,
    handle: ResizeHandleV1,
    base: RectEmu,
    target: RectEmu,
) -> None:
    resize_x, resize_y = _handle_axes(handle)

    if resize_x:
        if "e" in handle and target.x != base.x:
            _fail("east resize must keep aggregate left edge fixed")
        if "w" in handle and target.right != base.right:
            _fail("west resize must keep aggregate right edge fixed")
    elif target.x != base.x or target.width != base.width:
        _fail("unaffected horizontal aggregate axis must remain identical")

    if resize_y:
        if "s" in handle and target.y != base.y:
            _fail("south resize must keep aggregate top edge fixed")
        if "n" in handle and target.bottom != base.bottom:
            _fail("north resize must keep aggregate bottom edge fixed")
    elif target.y != base.y or target.height != base.height:
        _fail("unaffected vertical aggregate axis must remain identical")


def _map_edge(
    *,
    edge: int,
    base_start: int,
    base_span: int,
    target_start: int,
    target_span: int,
    label: str,
) -> int:
    relative = edge - base_start
    if relative < 0 or relative > base_span:
        _fail(f"{label} lies outside base aggregate")

    if relative == 0:
        mapped = target_start
    elif relative == base_span:
        mapped = target_start + target_span
    else:
        try:
            scaled = _round_ratio_nearest_emu(
                relative * target_span,
                base_span,
            )
            mapped = target_start + scaled
            _checked_int(mapped, label)
        except AuthoredGroupGeometryError as exc:
            raise MultiResizePlanError(str(exc)) from exc

    if mapped < MIN_SAFE_EMU or mapped > MAX_SAFE_EMU:
        _fail(f"{label} is outside JavaScript-safe EMU range")
    return mapped


def _map_member(
    *,
    member: MultiResizeMemberV1,
    base: RectEmu,
    target: RectEmu,
    resize_x: bool,
    resize_y: bool,
) -> RectEmu:
    before = member.rect

    if resize_x:
        left = _map_edge(
            edge=before.x,
            base_start=base.x,
            base_span=base.width,
            target_start=target.x,
            target_span=target.width,
            label=f"{member.node_id}.left",
        )
        right = _map_edge(
            edge=before.right,
            base_start=base.x,
            base_span=base.width,
            target_start=target.x,
            target_span=target.width,
            label=f"{member.node_id}.right",
        )
    else:
        left = before.x
        right = before.right

    if resize_y:
        top = _map_edge(
            edge=before.y,
            base_start=base.y,
            base_span=base.height,
            target_start=target.y,
            target_span=target.height,
            label=f"{member.node_id}.top",
        )
        bottom = _map_edge(
            edge=before.bottom,
            base_start=base.y,
            base_span=base.height,
            target_start=target.y,
            target_span=target.height,
            label=f"{member.node_id}.bottom",
        )
    else:
        top = before.y
        bottom = before.bottom

    if right <= left or bottom <= top:
        _fail(f"mapped member {member.node_id!r} collapsed to non-positive size")

    after = RectEmu(left, top, right - left, bottom - top)
    _checked_rect(after, f"{member.node_id}.after")
    return after


def plan_multi_resize_v1(
    *,
    members: tuple[MultiResizeMemberV1, ...],
    base_aggregate: RectEmu,
    handle: ResizeHandleV1,
    target_aggregate: RectEmu,
) -> MultiResizePlanV1:
    _validate_members(members)
    _checked_rect(base_aggregate, "base_aggregate")
    _checked_rect(target_aggregate, "target_aggregate")

    actual_union = _union_members(members)
    if actual_union != base_aggregate:
        _fail("base aggregate must equal the exact member union")

    _validate_fixed_edge_law(
        handle=handle,
        base=base_aggregate,
        target=target_aggregate,
    )

    resize_x, resize_y = _handle_axes(handle)
    if resize_x and target_aggregate.width == base_aggregate.width:
        if not resize_y or target_aggregate.height == base_aggregate.height:
            _fail("resize target must change at least one affected aggregate span")
    if resize_y and target_aggregate.height == base_aggregate.height:
        if not resize_x or target_aggregate.width == base_aggregate.width:
            _fail("resize target must change at least one affected aggregate span")

    results: list[MultiResizeMemberResultV1] = []
    for member in sorted(members, key=lambda item: item.node_id):
        after = _map_member(
            member=member,
            base=base_aggregate,
            target=target_aggregate,
            resize_x=resize_x,
            resize_y=resize_y,
        )
        results.append(
            MultiResizeMemberResultV1(
                node_id=member.node_id,
                before=member.rect,
                after=after,
            )
        )

    after_union = _union_members(
        tuple(
            MultiResizeMemberV1(
                node_id=result.node_id,
                rect=result.after,
            )
            for result in results
        )
    )
    if after_union != target_aggregate:
        _fail("mapped member union does not reproduce target aggregate exactly")

    return MultiResizePlanV1(
        handle=handle,
        base_aggregate=base_aggregate,
        target_aggregate=target_aggregate,
        members=tuple(results),
    )
