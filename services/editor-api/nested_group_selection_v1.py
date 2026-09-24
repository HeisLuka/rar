#!/usr/bin/env python3
"""Path-capable transient authored Group selection scope.

This is a successor to the bounded one-level GroupMember identity. It does not
change ObjectSelectionTargetV1 equality/hash semantics. A nested target carries
an explicit authored Group ancestor path from top-level Group through immediate
parent. Current graph topology is supplied as an immutable caller snapshot and
validated before composition/enter/escape/reconciliation.

Broken paths fail closed to empty top-level selection; they are never shortened
to a different semantic target.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from group_member_selection_v1 import (
    TopLevelSelectionScopeV1,
    empty_top_level_scope_v1,
)
from group_transform_chain_v1 import (
    AUTHORED_GROUP_PROVENANCE_V1,
    MAX_AUTHORED_GROUP_DEPTH_V1,
)
from object_selection_target_v1 import DirectNodeSelectionV1


NestedComposeModeV1 = Literal["replace", "add", "toggle", "subtract"]


class NestedGroupSelectionError(ValueError):
    pass


@dataclass(frozen=True)
class NestedGroupSelectionTargetV1:
    page_id: str
    group_path: tuple[str, ...]
    node_id: str

    def __post_init__(self) -> None:
        _identity(self.page_id, "page_id")
        _identity(self.node_id, "node_id")
        _validate_path_identity(self.group_path)
        if self.node_id in set(self.group_path):
            raise NestedGroupSelectionError(
                "nested target node_id must not repeat an ancestor GroupId"
            )


@dataclass(frozen=True)
class NestedGroupPathEdgeV1:
    group_id: str
    parent_group_id: str | None
    children: tuple[str, ...]
    provenance: str = AUTHORED_GROUP_PROVENANCE_V1

    def __post_init__(self) -> None:
        _identity(self.group_id, "group_id")
        if self.parent_group_id is not None:
            _identity(self.parent_group_id, "parent_group_id")
        if self.provenance != AUTHORED_GROUP_PROVENANCE_V1:
            raise NestedGroupSelectionError("unsupported Group provenance")
        if not isinstance(self.children, tuple):
            raise NestedGroupSelectionError("children must be an ordered tuple")
        for index, child in enumerate(self.children):
            _identity(child, f"children[{index}]")
        if len(set(self.children)) != len(self.children):
            raise NestedGroupSelectionError("Group children must be unique")


@dataclass(frozen=True)
class NestedGroupPathSnapshotV1:
    page_id: str
    edges: tuple[NestedGroupPathEdgeV1, ...]

    def __post_init__(self) -> None:
        _identity(self.page_id, "snapshot.page_id")
        if not isinstance(self.edges, tuple) or not self.edges:
            raise NestedGroupSelectionError("path snapshot requires at least one Group edge")
        if len(self.edges) > MAX_AUTHORED_GROUP_DEPTH_V1:
            raise NestedGroupSelectionError(
                "path snapshot exceeds MAX_AUTHORED_GROUP_DEPTH_V1"
            )


@dataclass(frozen=True)
class NestedGroupSelectionScopeV1:
    page_id: str
    container_path: tuple[str, ...]
    selected: tuple[NestedGroupSelectionTargetV1, ...]
    primary: NestedGroupSelectionTargetV1 | None
    scope: str = "nested_group_path"

    def __post_init__(self) -> None:
        _identity(self.page_id, "scope.page_id")
        _validate_path_identity(self.container_path)
        if not isinstance(self.selected, tuple):
            raise NestedGroupSelectionError("selected must be a tuple")
        for index, target in enumerate(self.selected):
            if not isinstance(target, NestedGroupSelectionTargetV1):
                raise NestedGroupSelectionError(
                    f"selected[{index}] must be NestedGroupSelectionTargetV1"
                )
            if target.page_id != self.page_id:
                raise NestedGroupSelectionError("selected targets must share scope page")
            if target.group_path != self.container_path:
                raise NestedGroupSelectionError(
                    "selected targets must share exact container path"
                )
        if len(set(self.selected)) != len(self.selected):
            raise NestedGroupSelectionError("selected targets must be duplicate-free")
        if self.primary is not None:
            if self.primary not in set(self.selected):
                raise NestedGroupSelectionError("primary must be selected")
            if (
                self.primary.page_id != self.page_id
                or self.primary.group_path != self.container_path
            ):
                raise NestedGroupSelectionError(
                    "primary must share exact scope page/path"
                )


def _identity(value: str, label: str) -> None:
    if not isinstance(value, str) or not value:
        raise NestedGroupSelectionError(f"{label} is required")


def _validate_path_identity(path: tuple[str, ...]) -> None:
    if not isinstance(path, tuple) or not path:
        raise NestedGroupSelectionError("group_path must be a non-empty tuple")
    if len(path) > MAX_AUTHORED_GROUP_DEPTH_V1:
        raise NestedGroupSelectionError(
            "group_path exceeds MAX_AUTHORED_GROUP_DEPTH_V1"
        )
    for index, group_id in enumerate(path):
        _identity(group_id, f"group_path[{index}]")
    if len(set(path)) != len(path):
        raise NestedGroupSelectionError("group_path contains a cycle/repeated Group")


def validate_nested_path_snapshot_v1(
    *,
    page_id: str,
    group_path: tuple[str, ...],
    snapshot: NestedGroupPathSnapshotV1,
) -> None:
    _identity(page_id, "page_id")
    _validate_path_identity(group_path)
    if not isinstance(snapshot, NestedGroupPathSnapshotV1):
        raise NestedGroupSelectionError("NestedGroupPathSnapshotV1 is required")
    if snapshot.page_id != page_id:
        raise NestedGroupSelectionError("snapshot page differs from selection page")
    if len(snapshot.edges) != len(group_path):
        raise NestedGroupSelectionError("snapshot edge count differs from group_path")

    edge_ids = tuple(edge.group_id for edge in snapshot.edges)
    if edge_ids != group_path:
        raise NestedGroupSelectionError("snapshot GroupIds differ from group_path")
    if len(set(edge_ids)) != len(edge_ids):
        raise NestedGroupSelectionError("snapshot contains a Group cycle")

    for index, edge in enumerate(snapshot.edges):
        expected_parent = None if index == 0 else group_path[index - 1]
        if edge.parent_group_id != expected_parent:
            raise NestedGroupSelectionError(
                f"snapshot edge[{index}] parent mismatch"
            )
        if index + 1 < len(group_path):
            expected_child_group = group_path[index + 1]
            if expected_child_group not in edge.children:
                raise NestedGroupSelectionError(
                    f"snapshot edge[{index}] does not contain next Group"
                )


def _validate_scope_against_snapshot(
    scope: NestedGroupSelectionScopeV1,
    snapshot: NestedGroupPathSnapshotV1,
) -> None:
    if not isinstance(scope, NestedGroupSelectionScopeV1):
        raise NestedGroupSelectionError("NestedGroupSelectionScopeV1 is required")
    validate_nested_path_snapshot_v1(
        page_id=scope.page_id,
        group_path=scope.container_path,
        snapshot=snapshot,
    )
    direct_children = set(snapshot.edges[-1].children)
    for target in scope.selected:
        if target.node_id not in direct_children:
            raise NestedGroupSelectionError(
                "selected target is not a current direct child of container path"
            )


def _normalize(
    values: set[NestedGroupSelectionTargetV1],
) -> tuple[NestedGroupSelectionTargetV1, ...]:
    return tuple(
        sorted(
            values,
            key=lambda target: (
                target.page_id,
                target.group_path,
                target.node_id,
            ),
        )
    )


def enter_root_group_scope_v1(
    *,
    root_group: DirectNodeSelectionV1,
    snapshot: NestedGroupPathSnapshotV1,
) -> NestedGroupSelectionScopeV1:
    if not isinstance(root_group, DirectNodeSelectionV1):
        raise NestedGroupSelectionError("root_group must be DirectNodeSelectionV1")
    path = (root_group.node_id,)
    validate_nested_path_snapshot_v1(
        page_id=root_group.page_id,
        group_path=path,
        snapshot=snapshot,
    )
    return NestedGroupSelectionScopeV1(
        page_id=root_group.page_id,
        container_path=path,
        selected=(),
        primary=None,
    )


def compose_nested_group_selection_v1(
    *,
    scope: NestedGroupSelectionScopeV1,
    snapshot: NestedGroupPathSnapshotV1,
    candidates: tuple[NestedGroupSelectionTargetV1, ...],
    mode: NestedComposeModeV1,
) -> NestedGroupSelectionScopeV1:
    _validate_scope_against_snapshot(scope, snapshot)
    if mode not in {"replace", "add", "toggle", "subtract"}:
        raise NestedGroupSelectionError("unsupported nested selection mode")
    if not isinstance(candidates, tuple):
        raise NestedGroupSelectionError("candidates must be a tuple")

    direct_children = set(snapshot.edges[-1].children)
    for index, target in enumerate(candidates):
        if not isinstance(target, NestedGroupSelectionTargetV1):
            raise NestedGroupSelectionError(
                f"candidates[{index}] must be NestedGroupSelectionTargetV1"
            )
        if target.page_id != scope.page_id:
            raise NestedGroupSelectionError("candidates must share scope page")
        if target.group_path != scope.container_path:
            raise NestedGroupSelectionError(
                "candidates must share exact container path"
            )
        if target.node_id not in direct_children:
            raise NestedGroupSelectionError(
                "candidate is not a current direct child of container path"
            )
    if len(set(candidates)) != len(candidates):
        raise NestedGroupSelectionError("candidates must be duplicate-free")

    base = set(scope.selected)
    incoming = set(candidates)
    if mode == "replace":
        selected = incoming
    elif mode == "add":
        selected = base | incoming
    elif mode == "toggle":
        selected = base ^ incoming
    else:
        selected = base - incoming

    normalized = _normalize(selected)
    if mode != "replace" and scope.primary is not None and scope.primary in selected:
        primary = scope.primary
    elif len(normalized) == 1:
        primary = normalized[0]
    else:
        primary = None

    return NestedGroupSelectionScopeV1(
        page_id=scope.page_id,
        container_path=scope.container_path,
        selected=normalized,
        primary=primary,
    )


def enter_selected_child_group_v1(
    *,
    scope: NestedGroupSelectionScopeV1,
    current_snapshot: NestedGroupPathSnapshotV1,
    child_group_id: str,
    child_snapshot: NestedGroupPathSnapshotV1,
) -> NestedGroupSelectionScopeV1:
    _validate_scope_against_snapshot(scope, current_snapshot)
    _identity(child_group_id, "child_group_id")

    selected_target = NestedGroupSelectionTargetV1(
        page_id=scope.page_id,
        group_path=scope.container_path,
        node_id=child_group_id,
    )
    if selected_target not in set(scope.selected):
        raise NestedGroupSelectionError(
            "enter child Group requires that Group to be selected in current scope"
        )

    new_path = scope.container_path + (child_group_id,)
    validate_nested_path_snapshot_v1(
        page_id=scope.page_id,
        group_path=new_path,
        snapshot=child_snapshot,
    )
    return NestedGroupSelectionScopeV1(
        page_id=scope.page_id,
        container_path=new_path,
        selected=(),
        primary=None,
    )


def escape_parent_nested_v1(
    *,
    scope: NestedGroupSelectionScopeV1,
    snapshot: NestedGroupPathSnapshotV1,
) -> TopLevelSelectionScopeV1 | NestedGroupSelectionScopeV1:
    try:
        _validate_scope_against_snapshot(scope, snapshot)
    except NestedGroupSelectionError:
        return empty_top_level_scope_v1()

    if len(scope.container_path) == 1:
        root = DirectNodeSelectionV1(
            scope.page_id,
            scope.container_path[0],
        )
        return TopLevelSelectionScopeV1(
            selected=(root,),
            primary=root,
        )

    exited_group = scope.container_path[-1]
    parent_path = scope.container_path[:-1]
    exited_target = NestedGroupSelectionTargetV1(
        page_id=scope.page_id,
        group_path=parent_path,
        node_id=exited_group,
    )
    return NestedGroupSelectionScopeV1(
        page_id=scope.page_id,
        container_path=parent_path,
        selected=(exited_target,),
        primary=exited_target,
    )


def reconcile_nested_group_scope_v1(
    *,
    scope: NestedGroupSelectionScopeV1,
    snapshot: NestedGroupPathSnapshotV1,
) -> TopLevelSelectionScopeV1 | NestedGroupSelectionScopeV1:
    try:
        _validate_scope_against_snapshot(scope, snapshot)
    except NestedGroupSelectionError:
        return empty_top_level_scope_v1()
    return scope


def nested_scope_on_page_change_v1(
    scope: NestedGroupSelectionScopeV1,
) -> TopLevelSelectionScopeV1:
    if not isinstance(scope, NestedGroupSelectionScopeV1):
        raise NestedGroupSelectionError("NestedGroupSelectionScopeV1 is required")
    return empty_top_level_scope_v1()
