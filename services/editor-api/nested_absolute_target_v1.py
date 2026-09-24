#!/usr/bin/env python3
"""Exact adapter from desired page rectangles to current nested Group mutations.

This planner owns no page-relative policy and performs no mutation. It converts
an external exact page-space translation target into either:
- exact current-container local MoveNode/MoveNodesV2 entries; or
- one immediate Group refit plus an optional existing ancestor refit cascade.

Only currently admitted authored rectangle/Group families are accepted in this
slice. Rich TextFrame/PictureFrame admission remains owned by its separate gate.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Literal

from authored_group_geometry_v1 import (
    AuthoredGroupGeometryError,
    RectEmu,
    _validate_local_rect,
    _validate_rect,
    materialize_group_child_rect_v1,
)
from group_ancestor_refit_v1 import (
    GroupAncestorRefitCascadePlanV1,
    GroupCascadeSnapshotV1,
    _inverse_prefix_exact,
    _map_edge_forward_unbounded,
    plan_group_ancestor_refit_cascade_v1,
)
from group_multi_member_refit_v1 import (
    GroupMultiMemberProposalV1,
    GroupMultiMemberRefitError,
    GroupMultiMemberRefitPlanV1,
    plan_group_multi_member_refit_v1,
)
from group_refit_plan_v1 import (
    GroupRefitChildResultV1,
    GroupRefitPlanError,
    GroupRefitPlanV1,
    plan_group_refit_v1,
)
from group_transform_chain_v1 import (
    AuthoredGroupEdgeV1,
    GroupTransformChainError,
    inverse_group_transform_chain_v1,
    project_group_transform_chain_v1,
    validate_group_transform_chain_v1,
)
from nested_group_selection_v1 import (
    NestedGroupPathSnapshotV1,
    NestedGroupSelectionError,
    NestedGroupSelectionScopeV1,
    validate_nested_path_snapshot_v1,
)


NestedAbsoluteFamilyV1 = Literal["rectangle", "group"]
NestedAbsoluteStatusV1 = Literal["contained_exact", "refit_exact", "rejected"]
NestedAbsoluteReasonV1 = Literal[
    "not_exactly_representable",
    "unsupported_family",
    "stale_path",
    "invalid_request",
]
NestedAbsoluteMutationRouteV1 = Literal[
    "move_node",
    "move_nodes_v2",
    "group_refit_composite",
]


class NestedAbsoluteTargetError(ValueError):
    pass


@dataclass(frozen=True)
class NestedAbsoluteTargetMemberV1:
    node_id: str
    local_rect: RectEmu
    family: str = "rectangle"
    provenance: str = "chaptera-authored"
    transform: str = "identity"


@dataclass(frozen=True)
class DesiredPageRectV1:
    node_id: str
    rect: RectEmu


@dataclass(frozen=True)
class NestedAbsoluteMoveEntryV1:
    node_id: str
    before_local: RectEmu
    after_local: RectEmu
    before_page: RectEmu
    after_page: RectEmu


@dataclass(frozen=True)
class NestedAbsoluteTargetPlanV1:
    status: NestedAbsoluteStatusV1
    page_id: str
    container_path: tuple[str, ...]
    selected_node_ids: tuple[str, ...]
    desired_page_rects: tuple[DesiredPageRectV1, ...]
    mutation_route: NestedAbsoluteMutationRouteV1 | None
    move_entries: tuple[NestedAbsoluteMoveEntryV1, ...]
    immediate_refit: GroupRefitPlanV1 | GroupMultiMemberRefitPlanV1 | None
    ancestor_cascade: GroupAncestorRefitCascadePlanV1 | None
    final_ancestry: tuple[AuthoredGroupEdgeV1, ...]
    unselected_siblings_preserved: bool
    reason: NestedAbsoluteReasonV1 | None = None
    authoring_mutations: int = 0


def _rejected(
    *,
    page_id: str,
    container_path: tuple[str, ...],
    selected_node_ids: tuple[str, ...],
    desired_page_rects: tuple[DesiredPageRectV1, ...],
    reason: NestedAbsoluteReasonV1,
) -> NestedAbsoluteTargetPlanV1:
    return NestedAbsoluteTargetPlanV1(
        status="rejected",
        page_id=page_id,
        container_path=container_path,
        selected_node_ids=selected_node_ids,
        desired_page_rects=desired_page_rects,
        mutation_route=None,
        move_entries=(),
        immediate_refit=None,
        ancestor_cascade=None,
        final_ancestry=(),
        unselected_siblings_preserved=False,
        reason=reason,
    )


def _contains(outer: RectEmu, inner: RectEmu) -> bool:
    return (
        inner.x >= outer.x
        and inner.y >= outer.y
        and inner.right <= outer.right
        and inner.bottom <= outer.bottom
    )


def _project(
    *,
    node_id: str,
    page_id: str,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    local_rect: RectEmu,
) -> RectEmu:
    try:
        return project_group_transform_chain_v1(
            target_id=node_id,
            target_page_id=page_id,
            ancestry=ancestry,
            target_local_rect=local_rect,
        ).effective_page_rect
    except GroupTransformChainError as exc:
        raise NestedAbsoluteTargetError(str(exc)) from exc


def _project_parent_rect_to_page_unbounded(
    *,
    prefix: tuple[AuthoredGroupEdgeV1, ...],
    parent_rect: RectEmu,
) -> RectEmu:
    current = parent_rect
    for edge in reversed(prefix):
        current = _map_edge_forward_unbounded(
            local_coordinate_space=edge.local_coordinate_space,
            local_rect=current,
            group_bounds=edge.bounds_in_parent,
        )
    return current


def _replace_edge(
    edge: AuthoredGroupEdgeV1,
    *,
    bounds_in_parent: RectEmu | None = None,
    local_coordinate_space: RectEmu | None = None,
) -> AuthoredGroupEdgeV1:
    return AuthoredGroupEdgeV1(
        group_id=edge.group_id,
        page_id=edge.page_id,
        parent_group_id=edge.parent_group_id,
        children=edge.children,
        bounds_in_parent=bounds_in_parent or edge.bounds_in_parent,
        local_coordinate_space=local_coordinate_space or edge.local_coordinate_space,
        provenance=edge.provenance,
    )


def _validate_inputs(
    *,
    scope: NestedGroupSelectionScopeV1,
    selection_snapshot: NestedGroupPathSnapshotV1,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    cascade_snapshots: tuple[GroupCascadeSnapshotV1, ...],
    members: tuple[NestedAbsoluteTargetMemberV1, ...],
    desired: tuple[DesiredPageRectV1, ...],
) -> tuple[
    tuple[NestedAbsoluteTargetMemberV1, ...],
    tuple[DesiredPageRectV1, ...],
]:
    if not isinstance(scope, NestedGroupSelectionScopeV1):
        raise NestedAbsoluteTargetError("NestedGroupSelectionScopeV1 is required")
    if not scope.selected:
        raise NestedAbsoluteTargetError("at least one selected direct child is required")
    try:
        validate_nested_path_snapshot_v1(
            page_id=scope.page_id,
            group_path=scope.container_path,
            snapshot=selection_snapshot,
        )
    except NestedGroupSelectionError as exc:
        raise NestedAbsoluteTargetError(str(exc)) from exc

    selected_ids = tuple(sorted(target.node_id for target in scope.selected))
    if len(set(selected_ids)) != len(selected_ids):
        raise NestedAbsoluteTargetError("selected NodeIds must be unique")

    if not isinstance(ancestry, tuple) or tuple(edge.group_id for edge in ancestry) != scope.container_path:
        raise NestedAbsoluteTargetError("transform ancestry differs from current selection path")
    if not ancestry:
        raise NestedAbsoluteTargetError("nested absolute target requires a Group path")
    if len(cascade_snapshots) != len(ancestry):
        raise NestedAbsoluteTargetError("cascade snapshot depth differs from transform path")
    for index, (edge, snapshot) in enumerate(zip(ancestry, cascade_snapshots)):
        if not isinstance(snapshot, GroupCascadeSnapshotV1) or snapshot.edge != edge:
            raise NestedAbsoluteTargetError(f"cascade snapshot[{index}] is stale")
        if selection_snapshot.edges[index].group_id != edge.group_id:
            raise NestedAbsoluteTargetError("selection/transform path mismatch")
        if selection_snapshot.edges[index].children != edge.children:
            raise NestedAbsoluteTargetError("selection membership snapshot is stale")

    direct_children = set(ancestry[-1].children)
    if any(node_id not in direct_children for node_id in selected_ids):
        raise NestedAbsoluteTargetError("selected target is not a current direct sibling")

    if not isinstance(members, tuple) or not isinstance(desired, tuple):
        raise NestedAbsoluteTargetError("members and desired targets must be tuples")
    by_member: dict[str, NestedAbsoluteTargetMemberV1] = {}
    for index, member in enumerate(members):
        if not isinstance(member, NestedAbsoluteTargetMemberV1):
            raise NestedAbsoluteTargetError(f"members[{index}] must be NestedAbsoluteTargetMemberV1")
        if member.node_id in by_member:
            raise NestedAbsoluteTargetError("member NodeIds must be unique")
        if member.family not in {"rectangle", "group"}:
            raise NestedAbsoluteTargetError("unsupported translation family")
        if member.provenance != "chaptera-authored":
            raise NestedAbsoluteTargetError("unsupported member provenance")
        if member.transform != "identity":
            raise NestedAbsoluteTargetError("unsupported member transform")
        try:
            _validate_local_rect(
                member.local_rect,
                ancestry[-1].local_coordinate_space,
                f"members[{index}].local_rect",
            )
        except AuthoredGroupGeometryError as exc:
            raise NestedAbsoluteTargetError(str(exc)) from exc
        by_member[member.node_id] = member

    if set(by_member) != set(selected_ids):
        raise NestedAbsoluteTargetError("member geometry must exactly match selected identities")

    current_children = {child.node_id: child.local_rect for child in cascade_snapshots[-1].children}
    for node_id, member in by_member.items():
        if current_children.get(node_id) != member.local_rect:
            raise NestedAbsoluteTargetError("member local geometry is stale")

    by_desired: dict[str, DesiredPageRectV1] = {}
    for index, target in enumerate(desired):
        if not isinstance(target, DesiredPageRectV1):
            raise NestedAbsoluteTargetError(f"desired[{index}] must be DesiredPageRectV1")
        if target.node_id in by_desired:
            raise NestedAbsoluteTargetError("desired target NodeIds must be unique")
        try:
            _validate_rect(target.rect, f"desired[{index}].rect")
        except AuthoredGroupGeometryError as exc:
            raise NestedAbsoluteTargetError(str(exc)) from exc
        by_desired[target.node_id] = target

    if set(by_desired) != set(selected_ids):
        raise NestedAbsoluteTargetError("desired rectangles must exactly match selected identities")

    normalized_members = tuple(by_member[node_id] for node_id in selected_ids)
    normalized_desired = tuple(by_desired[node_id] for node_id in selected_ids)

    for member, target in zip(normalized_members, normalized_desired):
        try:
            validate_group_transform_chain_v1(
                target_id=member.node_id,
                target_page_id=scope.page_id,
                ancestry=ancestry,
            )
        except GroupTransformChainError as exc:
            raise NestedAbsoluteTargetError(str(exc)) from exc
        current_page = _project(
            node_id=member.node_id,
            page_id=scope.page_id,
            ancestry=ancestry,
            local_rect=member.local_rect,
        )
        if (
            current_page.width != target.rect.width
            or current_page.height != target.rect.height
        ):
            raise NestedAbsoluteTargetError("absolute target must be translation-only in page geometry")

    return normalized_members, normalized_desired


def _final_ancestry_after_refit(
    *,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    immediate_refit: GroupRefitPlanV1 | GroupMultiMemberRefitPlanV1,
    ancestor_cascade: GroupAncestorRefitCascadePlanV1 | None,
) -> tuple[AuthoredGroupEdgeV1, ...]:
    edges = list(ancestry)
    deepest = len(edges) - 1
    edges[deepest] = _replace_edge(
        edges[deepest],
        bounds_in_parent=immediate_refit.new_group_bounds,
        local_coordinate_space=immediate_refit.new_local_coordinate_space,
    )

    if ancestor_cascade is not None:
        for patch in ancestor_cascade.patches:
            index = patch.path_index
            if index < 0 or index >= deepest:
                raise NestedAbsoluteTargetError("ancestor cascade patch index is invalid")
            edges[index] = _replace_edge(
                edges[index],
                bounds_in_parent=patch.refit.new_group_bounds,
                local_coordinate_space=patch.refit.new_local_coordinate_space,
            )
            child_id = edges[index + 1].group_id
            child = next(
                (row for row in patch.refit.children if row.node_id == child_id),
                None,
            )
            if child is None:
                raise NestedAbsoluteTargetError("ancestor cascade omitted path child")
            edges[index + 1] = _replace_edge(
                edges[index + 1],
                bounds_in_parent=child.local_rect,
            )
    return tuple(edges)


def _verify_unselected_siblings(
    *,
    page_id: str,
    original_ancestry: tuple[AuthoredGroupEdgeV1, ...],
    final_ancestry: tuple[AuthoredGroupEdgeV1, ...],
    snapshots: tuple[GroupCascadeSnapshotV1, ...],
    selected_ids: set[str],
    immediate_refit: GroupRefitPlanV1 | GroupMultiMemberRefitPlanV1,
    ancestor_cascade: GroupAncestorRefitCascadePlanV1 | None,
) -> bool:
    refit_by_group: dict[str, tuple[GroupRefitChildResultV1, ...]] = {
        original_ancestry[-1].group_id: immediate_refit.children,
    }
    if ancestor_cascade is not None:
        for patch in ancestor_cascade.patches:
            refit_by_group[original_ancestry[patch.path_index].group_id] = patch.refit.children

    deepest = len(original_ancestry) - 1
    for index, snapshot in enumerate(snapshots):
        results = refit_by_group.get(snapshot.edge.group_id)
        if results is None:
            continue
        after_by_id = {row.node_id: row.local_rect for row in results}
        path_child = original_ancestry[index + 1].group_id if index < deepest else None
        for child in snapshot.children:
            if index == deepest and child.node_id in selected_ids:
                continue
            if path_child is not None and child.node_id == path_child:
                continue
            after_local = after_by_id.get(child.node_id)
            if after_local is None:
                return False
            try:
                before_page = _project(
                    node_id=child.node_id,
                    page_id=page_id,
                    ancestry=original_ancestry[: index + 1],
                    local_rect=child.local_rect,
                )
                after_page = _project(
                    node_id=child.node_id,
                    page_id=page_id,
                    ancestry=final_ancestry[: index + 1],
                    local_rect=after_local,
                )
            except NestedAbsoluteTargetError:
                return False
            if before_page != after_page:
                return False
    return True


def plan_nested_absolute_targets_v1(
    *,
    scope: NestedGroupSelectionScopeV1,
    selection_snapshot: NestedGroupPathSnapshotV1,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    cascade_snapshots: tuple[GroupCascadeSnapshotV1, ...],
    members: tuple[NestedAbsoluteTargetMemberV1, ...],
    desired_page_rects: tuple[DesiredPageRectV1, ...],
) -> NestedAbsoluteTargetPlanV1:
    selected_ids = tuple(sorted(target.node_id for target in scope.selected)) if isinstance(scope, NestedGroupSelectionScopeV1) else ()
    normalized_desired = desired_page_rects if isinstance(desired_page_rects, tuple) else ()
    try:
        normalized_members, normalized_desired = _validate_inputs(
            scope=scope,
            selection_snapshot=selection_snapshot,
            ancestry=ancestry,
            cascade_snapshots=cascade_snapshots,
            members=members,
            desired=desired_page_rects,
        )
    except NestedAbsoluteTargetError as exc:
        text = str(exc)
        reason: NestedAbsoluteReasonV1
        if "unsupported translation family" in text or "unsupported member" in text:
            reason = "unsupported_family"
        elif "stale" in text or "mismatch" in text:
            reason = "stale_path"
        else:
            reason = "invalid_request"
        return _rejected(
            page_id=getattr(scope, "page_id", ""),
            container_path=getattr(scope, "container_path", ()),
            selected_node_ids=selected_ids,
            desired_page_rects=normalized_desired,
            reason=reason,
        )

    prefix = ancestry[:-1]
    deepest = ancestry[-1]
    desired_parent: dict[str, RectEmu] = {}
    expansion_required = False

    for target in normalized_desired:
        parent_rect = _inverse_prefix_exact(
            desired_page_rect=target.rect,
            prefix=prefix,
        )
        if parent_rect is None:
            return _rejected(
                page_id=scope.page_id,
                container_path=scope.container_path,
                selected_node_ids=selected_ids,
                desired_page_rects=normalized_desired,
                reason="not_exactly_representable",
            )
        desired_parent[target.node_id] = parent_rect
        if not _contains(deepest.bounds_in_parent, parent_rect):
            expansion_required = True

    if not expansion_required:
        entries: list[NestedAbsoluteMoveEntryV1] = []
        for member, target in zip(normalized_members, normalized_desired):
            try:
                inverse = inverse_group_transform_chain_v1(
                    target_id=member.node_id,
                    target_page_id=scope.page_id,
                    ancestry=ancestry,
                    desired_page_rect=target.rect,
                )
            except GroupTransformChainError:
                return _rejected(
                    page_id=scope.page_id,
                    container_path=scope.container_path,
                    selected_node_ids=selected_ids,
                    desired_page_rects=normalized_desired,
                    reason="not_exactly_representable",
                )
            after = inverse.canonical_local_rect
            if inverse.effective_page_rect != target.rect:
                return _rejected(
                    page_id=scope.page_id,
                    container_path=scope.container_path,
                    selected_node_ids=selected_ids,
                    desired_page_rects=normalized_desired,
                    reason="not_exactly_representable",
                )
            if after.width != member.local_rect.width or after.height != member.local_rect.height:
                return _rejected(
                    page_id=scope.page_id,
                    container_path=scope.container_path,
                    selected_node_ids=selected_ids,
                    desired_page_rects=normalized_desired,
                    reason="not_exactly_representable",
                )
            before_page = _project(
                node_id=member.node_id,
                page_id=scope.page_id,
                ancestry=ancestry,
                local_rect=member.local_rect,
            )
            entries.append(
                NestedAbsoluteMoveEntryV1(
                    node_id=member.node_id,
                    before_local=member.local_rect,
                    after_local=after,
                    before_page=before_page,
                    after_page=target.rect,
                )
            )

        route: NestedAbsoluteMutationRouteV1 = "move_node" if len(entries) == 1 else "move_nodes_v2"
        return NestedAbsoluteTargetPlanV1(
            status="contained_exact",
            page_id=scope.page_id,
            container_path=scope.container_path,
            selected_node_ids=selected_ids,
            desired_page_rects=normalized_desired,
            mutation_route=route,
            move_entries=tuple(entries),
            immediate_refit=None,
            ancestor_cascade=None,
            final_ancestry=ancestry,
            unselected_siblings_preserved=True,
        )

    parent_local = ancestry[-2].local_coordinate_space if len(ancestry) > 1 else None
    try:
        if len(normalized_members) == 1:
            member = normalized_members[0]
            immediate: GroupRefitPlanV1 | GroupMultiMemberRefitPlanV1 = plan_group_refit_v1(
                mode="expand_only_with_proposed_child_rect",
                group_bounds=deepest.bounds_in_parent,
                local_coordinate_space=deepest.local_coordinate_space,
                children=cascade_snapshots[-1].children,
                target_node_id=member.node_id,
                desired_target_parent_rect=desired_parent[member.node_id],
                parent_local_coordinate_space=parent_local,
            )
        else:
            immediate = plan_group_multi_member_refit_v1(
                group=deepest,
                children=cascade_snapshots[-1].children,
                proposals=tuple(
                    GroupMultiMemberProposalV1(
                        node_id=member.node_id,
                        desired_effective_parent_rect=desired_parent[member.node_id],
                    )
                    for member in normalized_members
                ),
                parent_local_coordinate_space=parent_local,
            )
    except (GroupRefitPlanError, GroupMultiMemberRefitError):
        return _rejected(
            page_id=scope.page_id,
            container_path=scope.container_path,
            selected_node_ids=selected_ids,
            desired_page_rects=normalized_desired,
            reason="not_exactly_representable",
        )

    if immediate.status == "no_envelope_change":
        return _rejected(
            page_id=scope.page_id,
            container_path=scope.container_path,
            selected_node_ids=selected_ids,
            desired_page_rects=normalized_desired,
            reason="not_exactly_representable",
        )

    ancestor_cascade = None
    if immediate.status == "ancestor_refit_required":
        if len(ancestry) <= 1:
            return _rejected(
                page_id=scope.page_id,
                container_path=scope.container_path,
                selected_node_ids=selected_ids,
                desired_page_rects=normalized_desired,
                reason="not_exactly_representable",
            )
        try:
            candidate_page = _project_parent_rect_to_page_unbounded(
                prefix=prefix,
                parent_rect=immediate.new_group_bounds,
            )
        except (ValueError, AuthoredGroupGeometryError):
            return _rejected(
                page_id=scope.page_id,
                container_path=scope.container_path,
                selected_node_ids=selected_ids,
                desired_page_rects=normalized_desired,
                reason="not_exactly_representable",
            )
        ancestor_cascade = plan_group_ancestor_refit_cascade_v1(
            target_id=deepest.group_id,
            page_id=scope.page_id,
            snapshots=cascade_snapshots[:-1],
            desired_page_rect=candidate_page,
        )
        if ancestor_cascade.status != "planned":
            reason = (
                "stale_path"
                if ancestor_cascade.reason == "stale_path"
                else "not_exactly_representable"
            )
            return _rejected(
                page_id=scope.page_id,
                container_path=scope.container_path,
                selected_node_ids=selected_ids,
                desired_page_rects=normalized_desired,
                reason=reason,
            )

    try:
        final_ancestry = _final_ancestry_after_refit(
            ancestry=ancestry,
            immediate_refit=immediate,
            ancestor_cascade=ancestor_cascade,
        )
    except NestedAbsoluteTargetError:
        return _rejected(
            page_id=scope.page_id,
            container_path=scope.container_path,
            selected_node_ids=selected_ids,
            desired_page_rects=normalized_desired,
            reason="stale_path",
        )

    after_by_id = {row.node_id: row.local_rect for row in immediate.children}
    entries = []
    for member, target in zip(normalized_members, normalized_desired):
        after = after_by_id.get(member.node_id)
        if after is None:
            return _rejected(
                page_id=scope.page_id,
                container_path=scope.container_path,
                selected_node_ids=selected_ids,
                desired_page_rects=normalized_desired,
                reason="stale_path",
            )
        try:
            after_page = _project(
                node_id=member.node_id,
                page_id=scope.page_id,
                ancestry=final_ancestry,
                local_rect=after,
            )
        except NestedAbsoluteTargetError:
            return _rejected(
                page_id=scope.page_id,
                container_path=scope.container_path,
                selected_node_ids=selected_ids,
                desired_page_rects=normalized_desired,
                reason="not_exactly_representable",
            )
        if after_page != target.rect:
            return _rejected(
                page_id=scope.page_id,
                container_path=scope.container_path,
                selected_node_ids=selected_ids,
                desired_page_rects=normalized_desired,
                reason="not_exactly_representable",
            )
        entries.append(
            NestedAbsoluteMoveEntryV1(
                node_id=member.node_id,
                before_local=member.local_rect,
                after_local=after,
                before_page=_project(
                    node_id=member.node_id,
                    page_id=scope.page_id,
                    ancestry=ancestry,
                    local_rect=member.local_rect,
                ),
                after_page=after_page,
            )
        )

    siblings_preserved = _verify_unselected_siblings(
        page_id=scope.page_id,
        original_ancestry=ancestry,
        final_ancestry=final_ancestry,
        snapshots=cascade_snapshots,
        selected_ids=set(selected_ids),
        immediate_refit=immediate,
        ancestor_cascade=ancestor_cascade,
    )
    if not siblings_preserved:
        return _rejected(
            page_id=scope.page_id,
            container_path=scope.container_path,
            selected_node_ids=selected_ids,
            desired_page_rects=normalized_desired,
            reason="not_exactly_representable",
        )

    return NestedAbsoluteTargetPlanV1(
        status="refit_exact",
        page_id=scope.page_id,
        container_path=scope.container_path,
        selected_node_ids=selected_ids,
        desired_page_rects=normalized_desired,
        mutation_route="group_refit_composite",
        move_entries=tuple(entries),
        immediate_refit=immediate,
        ancestor_cascade=ancestor_cascade,
        final_ancestry=final_ancestry,
        unselected_siblings_preserved=True,
    )
