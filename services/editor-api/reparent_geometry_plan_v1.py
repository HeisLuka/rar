#!/usr/bin/env python3
"""Pure geometry and placement plan for authored-node reparenting V1.

Reparent preserves visible page-space geometry, not old container-local numbers:

    source-local -> exact page RectEMU -> destination-local

Group conversion delegates to GroupTransformChainV1. Destination membership is
not mutated by this planner; a synthetic geometry-only terminal probe is used
solely so the existing transform-chain validator/inverse can evaluate the
destination container path without pretending the node is already a member.

No mutation, history, selection, automatic Group refit, cloning, or native
Publisher reparent semantics live here.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from authored_group_geometry_v1 import (
    AuthoredGroupGeometryError,
    RectEmu,
    _validate_local_rect,
    _validate_rect,
)
from group_transform_chain_v1 import (
    AUTHORED_GROUP_PROVENANCE_V1,
    AuthoredGroupEdgeV1,
    GroupTransformChainError,
    inverse_group_transform_chain_v1,
    project_group_transform_chain_v1,
    validate_group_transform_chain_v1,
)


ContainerKindV1 = Literal["page", "group"]
ReparentStatusV1 = Literal["planned", "rejected"]
ReparentRejectReasonV1 = Literal[
    "same_parent",
    "cycle",
    "cross_page",
    "stale_membership",
    "unsupported",
    "destination_out_of_bounds",
    "not_exactly_representable",
]


@dataclass(frozen=True)
class ReparentContainerV1:
    kind: ContainerKindV1
    container_id: str
    page_id: str
    children: tuple[str, ...]
    ancestry: tuple[AuthoredGroupEdgeV1, ...] = ()


@dataclass(frozen=True)
class ReparentRemovalV1:
    container_id: str
    children_before: tuple[str, ...]
    source_index: int
    previous_node_id: str | None
    next_node_id: str | None


@dataclass(frozen=True)
class ReparentInsertionV1:
    container_id: str
    children_before: tuple[str, ...]
    insert_index: int
    anchor_node_id: str | None


@dataclass(frozen=True)
class ReparentGeometryReceiptV1:
    source_local_rect: RectEmu
    source_effective_page_rect: RectEmu
    destination_local_rect: RectEmu
    destination_effective_page_rect: RectEmu


@dataclass(frozen=True)
class ReparentGeometryPlanV1:
    status: ReparentStatusV1
    node_id: str
    page_id: str
    source: ReparentContainerV1
    destination: ReparentContainerV1
    removal: ReparentRemovalV1 | None
    insertion: ReparentInsertionV1 | None
    geometry: ReparentGeometryReceiptV1 | None
    reason: ReparentRejectReasonV1 | None = None


def _reject(
    *,
    node_id: str,
    page_id: str,
    source: ReparentContainerV1,
    destination: ReparentContainerV1,
    reason: ReparentRejectReasonV1,
) -> ReparentGeometryPlanV1:
    return ReparentGeometryPlanV1(
        status="rejected",
        node_id=node_id,
        page_id=page_id,
        source=source,
        destination=destination,
        removal=None,
        insertion=None,
        geometry=None,
        reason=reason,
    )


def _basic_container_shape(container: ReparentContainerV1) -> bool:
    if not isinstance(container, ReparentContainerV1):
        return False
    if container.kind not in {"page", "group"}:
        return False
    if not isinstance(container.container_id, str) or not container.container_id:
        return False
    if not isinstance(container.page_id, str) or not container.page_id:
        return False
    if not isinstance(container.children, tuple):
        return False
    if any(not isinstance(node_id, str) or not node_id for node_id in container.children):
        return False
    if len(set(container.children)) != len(container.children):
        return False
    if not isinstance(container.ancestry, tuple):
        return False
    if container.kind == "page":
        return not container.ancestry
    if not container.ancestry:
        return False
    if container.ancestry[-1].group_id != container.container_id:
        return False
    if container.ancestry[-1].children != container.children:
        return False
    if any(
        edge.page_id != container.page_id
        or edge.provenance != AUTHORED_GROUP_PROVENANCE_V1
        for edge in container.ancestry
    ):
        return False
    return True


def _probe_id(
    *,
    node_id: str,
    destination: ReparentContainerV1,
) -> str:
    occupied = set(destination.children)
    occupied.update(edge.group_id for edge in destination.ancestry)
    base = f"__chaptera_reparent_probe_v1__:{node_id}"
    candidate = base
    suffix = 0
    while candidate in occupied:
        suffix += 1
        candidate = f"{base}:{suffix}"
    return candidate


def _destination_probe_ancestry(
    *,
    node_id: str,
    destination: ReparentContainerV1,
) -> tuple[AuthoredGroupEdgeV1, ...]:
    probe = _probe_id(node_id=node_id, destination=destination)
    edges = list(destination.ancestry)
    edges[-1] = replace(
        edges[-1],
        children=edges[-1].children + (probe,),
    )
    return tuple(edges)


def _validate_source_path(
    *,
    node_id: str,
    source: ReparentContainerV1,
) -> ReparentRejectReasonV1 | None:
    if source.children.count(node_id) != 1:
        return "stale_membership"
    if source.kind == "page":
        return None
    try:
        validate_group_transform_chain_v1(
            target_id=node_id,
            target_page_id=source.page_id,
            ancestry=source.ancestry,
        )
    except GroupTransformChainError as exc:
        text = str(exc)
        if "provenance" in text or "depth" in text or "local coordinate" in text:
            return "unsupported"
        return "stale_membership"
    return None


def _validate_destination_path(
    *,
    node_id: str,
    destination: ReparentContainerV1,
) -> tuple[tuple[AuthoredGroupEdgeV1, ...] | None, ReparentRejectReasonV1 | None]:
    if node_id in destination.children:
        return None, "stale_membership"
    if destination.kind == "page":
        return (), None
    ancestry = _destination_probe_ancestry(
        node_id=node_id,
        destination=destination,
    )
    probe = ancestry[-1].children[-1]
    try:
        validate_group_transform_chain_v1(
            target_id=probe,
            target_page_id=destination.page_id,
            ancestry=ancestry,
        )
    except GroupTransformChainError as exc:
        text = str(exc)
        if "provenance" in text or "depth" in text or "local coordinate" in text:
            return None, "unsupported"
        return None, "stale_membership"
    return ancestry, None


def plan_reparent_geometry_v1(
    *,
    node_id: str,
    node_authority: str,
    source: ReparentContainerV1,
    destination: ReparentContainerV1,
    source_local_rect: RectEmu,
    destination_insert_index: int,
    destination_anchor_node_id: str | None,
    node_descendant_ids: tuple[str, ...] = (),
) -> ReparentGeometryPlanV1:
    page_id = source.page_id if isinstance(source, ReparentContainerV1) else ""

    if (
        not isinstance(node_id, str)
        or not node_id
        or node_authority != "chaptera-authored"
        or not _basic_container_shape(source)
        or not _basic_container_shape(destination)
        or not isinstance(node_descendant_ids, tuple)
        or any(not isinstance(item, str) or not item for item in node_descendant_ids)
        or len(set(node_descendant_ids)) != len(node_descendant_ids)
    ):
        return _reject(
            node_id=node_id,
            page_id=page_id,
            source=source,
            destination=destination,
            reason="unsupported",
        )

    if source.page_id != destination.page_id:
        return _reject(
            node_id=node_id,
            page_id=source.page_id,
            source=source,
            destination=destination,
            reason="cross_page",
        )
    page_id = source.page_id

    if source.kind == destination.kind and source.container_id == destination.container_id:
        return _reject(
            node_id=node_id,
            page_id=page_id,
            source=source,
            destination=destination,
            reason="same_parent",
        )

    if destination.kind == "group" and (
        destination.container_id == node_id
        or destination.container_id in set(node_descendant_ids)
    ):
        return _reject(
            node_id=node_id,
            page_id=page_id,
            source=source,
            destination=destination,
            reason="cycle",
        )

    source_reason = _validate_source_path(node_id=node_id, source=source)
    if source_reason is not None:
        return _reject(
            node_id=node_id,
            page_id=page_id,
            source=source,
            destination=destination,
            reason=source_reason,
        )

    destination_ancestry, destination_reason = _validate_destination_path(
        node_id=node_id,
        destination=destination,
    )
    if destination_reason is not None:
        return _reject(
            node_id=node_id,
            page_id=page_id,
            source=source,
            destination=destination,
            reason=destination_reason,
        )

    if (
        not isinstance(destination_insert_index, int)
        or isinstance(destination_insert_index, bool)
        or destination_insert_index < 0
        or destination_insert_index > len(destination.children)
    ):
        return _reject(
            node_id=node_id,
            page_id=page_id,
            source=source,
            destination=destination,
            reason="stale_membership",
        )
    expected_anchor = (
        destination.children[destination_insert_index]
        if destination_insert_index < len(destination.children)
        else None
    )
    if destination_anchor_node_id != expected_anchor:
        return _reject(
            node_id=node_id,
            page_id=page_id,
            source=source,
            destination=destination,
            reason="stale_membership",
        )

    try:
        _validate_rect(source_local_rect, "source_local_rect")
    except AuthoredGroupGeometryError:
        return _reject(
            node_id=node_id,
            page_id=page_id,
            source=source,
            destination=destination,
            reason="unsupported",
        )

    try:
        if source.kind == "page":
            source_page_rect = source_local_rect
        else:
            source_projection = project_group_transform_chain_v1(
                target_id=node_id,
                target_page_id=page_id,
                ancestry=source.ancestry,
                target_local_rect=source_local_rect,
            )
            source_page_rect = source_projection.effective_page_rect
    except GroupTransformChainError:
        return _reject(
            node_id=node_id,
            page_id=page_id,
            source=source,
            destination=destination,
            reason="stale_membership",
        )

    if destination.kind == "page":
        destination_local_rect = source_page_rect
        destination_page_rect = source_page_rect
    else:
        assert destination_ancestry is not None
        probe = destination_ancestry[-1].children[-1]
        try:
            inverse = inverse_group_transform_chain_v1(
                target_id=probe,
                target_page_id=page_id,
                ancestry=destination_ancestry,
                desired_page_rect=source_page_rect,
            )
        except GroupTransformChainError as exc:
            if "outside Group bounds" in str(exc):
                reason: ReparentRejectReasonV1 = "destination_out_of_bounds"
            else:
                reason = "not_exactly_representable"
            return _reject(
                node_id=node_id,
                page_id=page_id,
                source=source,
                destination=destination,
                reason=reason,
            )

        destination_local_rect = inverse.canonical_local_rect
        destination_page_rect = inverse.effective_page_rect
        if destination_page_rect != source_page_rect:
            return _reject(
                node_id=node_id,
                page_id=page_id,
                source=source,
                destination=destination,
                reason="not_exactly_representable",
            )
        try:
            _validate_local_rect(
                destination_local_rect,
                destination.ancestry[-1].local_coordinate_space,
                "destination_local_rect",
            )
        except AuthoredGroupGeometryError:
            return _reject(
                node_id=node_id,
                page_id=page_id,
                source=source,
                destination=destination,
                reason="destination_out_of_bounds",
            )

    source_index = source.children.index(node_id)
    removal = ReparentRemovalV1(
        container_id=source.container_id,
        children_before=source.children,
        source_index=source_index,
        previous_node_id=source.children[source_index - 1] if source_index > 0 else None,
        next_node_id=(
            source.children[source_index + 1]
            if source_index + 1 < len(source.children)
            else None
        ),
    )
    insertion = ReparentInsertionV1(
        container_id=destination.container_id,
        children_before=destination.children,
        insert_index=destination_insert_index,
        anchor_node_id=destination_anchor_node_id,
    )
    geometry = ReparentGeometryReceiptV1(
        source_local_rect=source_local_rect,
        source_effective_page_rect=source_page_rect,
        destination_local_rect=destination_local_rect,
        destination_effective_page_rect=destination_page_rect,
    )
    return ReparentGeometryPlanV1(
        status="planned",
        node_id=node_id,
        page_id=page_id,
        source=source,
        destination=destination,
        removal=removal,
        insertion=insertion,
        geometry=geometry,
    )
