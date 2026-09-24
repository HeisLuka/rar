#!/usr/bin/env python3
"""Pure insert-refit planner for Chaptera-authored Group V1.

Existing children already belong to the Group. Proposed children do not exist in
that membership yet; this module plans only the geometry parameterization needed
to insert them while preserving every existing child's effective rectangle.

No graph mutation, NodeId allocation, insertion order policy, source-backed
admission, shrink, or native Publisher semantics live here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from authored_group_geometry_v1 import (
    AuthoredGroupGeometryError,
    RectEmu,
    _validate_rect,
    materialize_group_child_rect_v1,
)
from authored_group_hierarchy_v1 import (
    AuthoredGroupHierarchyError,
    validate_authored_group_hierarchy_v1,
)
from group_transform_chain_v1 import (
    AUTHORED_GROUP_PROVENANCE_V1,
    AuthoredGroupEdgeV1,
    GroupTransformChainError,
    _inverse_boundary_v1,
)

PlanStatusV1 = Literal["planned", "no_envelope_change", "ancestor_refit_required"]


class GroupInsertRefitPlanError(ValueError):
    pass


@dataclass(frozen=True)
class ProposedGroupInsertChildV1:
    node_id: str
    effective_parent_rect: RectEmu


@dataclass(frozen=True)
class GroupInsertChildResultV1:
    node_id: str
    local_rect: RectEmu
    effective_parent_rect: RectEmu
    is_new: bool


@dataclass(frozen=True)
class GroupInsertRefitPlanV1:
    status: PlanStatusV1
    group_id: str
    old_group_bounds: RectEmu
    old_local_coordinate_space: RectEmu
    new_group_bounds: RectEmu
    new_local_coordinate_space: RectEmu
    existing_children: tuple[GroupInsertChildResultV1, ...]
    proposed_children: tuple[GroupInsertChildResultV1, ...]
    ancestor_candidate_rect: RectEmu


def _fail(message: str) -> None:
    raise GroupInsertRefitPlanError(message)


def _rect_from_mapping(value, label: str) -> RectEmu:
    if not isinstance(value, dict) or set(value) != {"x", "y", "width", "height"}:
        _fail(f"{label} must be an exact rect")
    rect = RectEmu(value["x"], value["y"], value["width"], value["height"])
    try:
        _validate_rect(rect, label)
    except AuthoredGroupGeometryError as exc:
        raise GroupInsertRefitPlanError(str(exc)) from exc
    return rect


def _contains(outer: RectEmu, inner: RectEmu) -> bool:
    return (
        inner.x >= outer.x
        and inner.y >= outer.y
        and inner.right <= outer.right
        and inner.bottom <= outer.bottom
    )


def _union(rects: tuple[RectEmu, ...]) -> RectEmu:
    if not rects:
        _fail("cannot union empty rectangle set")
    for index, rect in enumerate(rects):
        try:
            _validate_rect(rect, f"union[{index}]")
        except AuthoredGroupGeometryError as exc:
            raise GroupInsertRefitPlanError(str(exc)) from exc
    left = min(r.x for r in rects)
    top = min(r.y for r in rects)
    right = max(r.right for r in rects)
    bottom = max(r.bottom for r in rects)
    rect = RectEmu(left, top, right - left, bottom - top)
    try:
        _validate_rect(rect, "union")
    except AuthoredGroupGeometryError as exc:
        raise GroupInsertRefitPlanError(str(exc)) from exc
    return rect


def _exact_local(
    *,
    group_id: str,
    page_id: str,
    child_id: str,
    children: tuple[str, ...],
    group_bounds: RectEmu,
    local_space: RectEmu,
    desired_parent_rect: RectEmu,
) -> RectEmu:
    edge = AuthoredGroupEdgeV1(
        group_id=group_id,
        page_id=page_id,
        parent_group_id=None,
        children=children,
        bounds_in_parent=group_bounds,
        local_coordinate_space=local_space,
        provenance=AUTHORED_GROUP_PROVENANCE_V1,
    )
    try:
        local = _inverse_boundary_v1(
            parent_rect=desired_parent_rect,
            edge=edge,
            label=f"insert_refit[{child_id}]",
        )
        projected = materialize_group_child_rect_v1(
            local_coordinate_space=local_space,
            child_local_bounds=local,
            current_group_bounds=group_bounds,
        )
    except (GroupTransformChainError, AuthoredGroupGeometryError) as exc:
        raise GroupInsertRefitPlanError(
            f"child {child_id!r} is not exactly representable: {exc}"
        ) from exc
    if projected != desired_parent_rect:
        _fail(f"child {child_id!r} is not exactly representable")
    return local


def plan_group_insert_refit_v1(
    *,
    project: dict,
    group_id: str,
    expected_group_bounds: RectEmu,
    expected_local_coordinate_space: RectEmu,
    expected_child_order: tuple[str, ...],
    proposed_children: tuple[ProposedGroupInsertChildV1, ...],
    parent_local_coordinate_space: RectEmu | None = None,
) -> GroupInsertRefitPlanV1:
    """Plan insertion geometry against one frozen authored Group snapshot."""
    if not isinstance(group_id, str) or not group_id:
        _fail("group_id is required")
    try:
        validate_authored_group_hierarchy_v1(project)
    except AuthoredGroupHierarchyError as exc:
        raise GroupInsertRefitPlanError(f"invalid authored hierarchy: {exc}") from exc

    nodes = project["nodes"]
    group = nodes.get(group_id)
    if (
        not isinstance(group, dict)
        or group.get("kind") != "group"
        or group.get("author_created") is not True
    ):
        _fail("unsupported Group provenance")

    page_id = None
    parent_id = group.get("parent_id")
    if parent_id in project["pages"]:
        page_id = parent_id
    elif parent_id in nodes:
        parent = nodes[parent_id]
        while isinstance(parent, dict):
            next_parent = parent.get("parent_id")
            if next_parent in project["pages"]:
                page_id = next_parent
                break
            parent = nodes.get(next_parent)
    if not isinstance(page_id, str) or not page_id:
        _fail("Group page ancestry is unresolved")

    group_bounds = _rect_from_mapping(group.get("bounds"), f"{group_id}.bounds")
    local_space = _rect_from_mapping(
        group.get("local_coordinate_space"),
        f"{group_id}.local_coordinate_space",
    )
    child_order = tuple(group.get("children") or ())

    if (
        group_bounds != expected_group_bounds
        or local_space != expected_local_coordinate_space
        or child_order != expected_child_order
    ):
        _fail("stale Group header/order snapshot")

    if not isinstance(proposed_children, tuple) or not proposed_children:
        _fail("proposed_children must be a non-empty ordered tuple")

    proposed_ids: list[str] = []
    for index, child in enumerate(proposed_children):
        if not isinstance(child, ProposedGroupInsertChildV1):
            _fail(f"proposed_children[{index}] must be ProposedGroupInsertChildV1")
        if not isinstance(child.node_id, str) or not child.node_id:
            _fail(f"proposed_children[{index}].node_id is required")
        if child.node_id in nodes or child.node_id in child_order:
            _fail(f"proposed child {child.node_id!r} already exists")
        try:
            _validate_rect(
                child.effective_parent_rect,
                f"proposed_children[{index}].effective_parent_rect",
            )
        except AuthoredGroupGeometryError as exc:
            raise GroupInsertRefitPlanError(str(exc)) from exc
        proposed_ids.append(child.node_id)
    if len(set(proposed_ids)) != len(proposed_ids):
        _fail("proposed child NodeIds must be unique")

    existing_effective: list[tuple[str, RectEmu, RectEmu]] = []
    for child_id in child_order:
        child = nodes.get(child_id)
        if not isinstance(child, dict) or child.get("parent_id") != group_id:
            _fail(f"stale child membership for {child_id!r}")
        local = _rect_from_mapping(child.get("bounds"), f"{child_id}.bounds")
        try:
            effective = materialize_group_child_rect_v1(
                local_coordinate_space=local_space,
                child_local_bounds=local,
                current_group_bounds=group_bounds,
            )
        except AuthoredGroupGeometryError as exc:
            raise GroupInsertRefitPlanError(str(exc)) from exc
        existing_effective.append((child_id, local, effective))

    envelope = _union(
        (group_bounds,)
        + tuple(child.effective_parent_rect for child in proposed_children)
    )
    if not _contains(envelope, group_bounds):
        _fail("insert-refit must never shrink current Group envelope")

    no_change = envelope == group_bounds
    new_local = (
        local_space
        if no_change
        else RectEmu(0, 0, envelope.width, envelope.height)
    )

    all_ids = child_order + tuple(proposed_ids)
    existing_results: list[GroupInsertChildResultV1] = []
    for child_id, old_local, effective in existing_effective:
        if no_change:
            local = old_local
        else:
            local = _exact_local(
                group_id=group_id,
                page_id=page_id,
                child_id=child_id,
                children=all_ids,
                group_bounds=envelope,
                local_space=new_local,
                desired_parent_rect=effective,
            )
        existing_results.append(
            GroupInsertChildResultV1(
                node_id=child_id,
                local_rect=local,
                effective_parent_rect=effective,
                is_new=False,
            )
        )

    proposed_results: list[GroupInsertChildResultV1] = []
    for child in proposed_children:
        local = _exact_local(
            group_id=group_id,
            page_id=page_id,
            child_id=child.node_id,
            children=all_ids,
            group_bounds=envelope,
            local_space=new_local,
            desired_parent_rect=child.effective_parent_rect,
        )
        proposed_results.append(
            GroupInsertChildResultV1(
                node_id=child.node_id,
                local_rect=local,
                effective_parent_rect=child.effective_parent_rect,
                is_new=True,
            )
        )

    status: PlanStatusV1 = "no_envelope_change" if no_change else "planned"
    if parent_local_coordinate_space is not None:
        try:
            _validate_rect(parent_local_coordinate_space, "parent_local_coordinate_space")
        except AuthoredGroupGeometryError as exc:
            raise GroupInsertRefitPlanError(str(exc)) from exc
        if (
            parent_local_coordinate_space.x != 0
            or parent_local_coordinate_space.y != 0
        ):
            _fail("parent local coordinate space origin must be zero")
        if not _contains(parent_local_coordinate_space, envelope):
            status = "ancestor_refit_required"

    return GroupInsertRefitPlanV1(
        status=status,
        group_id=group_id,
        old_group_bounds=group_bounds,
        old_local_coordinate_space=local_space,
        new_group_bounds=envelope,
        new_local_coordinate_space=new_local,
        existing_children=tuple(existing_results),
        proposed_children=tuple(proposed_results),
        ancestor_candidate_rect=envelope,
    )
