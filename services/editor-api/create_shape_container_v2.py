#!/usr/bin/env python3
"""Canonical CreateShape V2 for Page or authored Group destinations."""

from __future__ import annotations

import copy

from authored_stack_v1 import append_authored_member, validate_authored_lane
from create_shape_v1 import (
    CreateShapeError,
    validate_creation_paint_v1,
    validate_rect_emu_v1,
    validate_uuid7_node_id_v1,
)
from group_child_batch_lifecycle_v1 import (
    GroupChildCandidateV1,
    plan_group_child_batch_insert_v1,
)


class CreateShapeV2Error(ValueError):
    pass


def validate_create_shape_v2_intent(command: dict) -> None:
    allowed = {
        "kind",
        "node_id",
        "page_id",
        "destination",
        "placement",
        "paint",
        "expected_order_lane",
        "insertion_policy",
    }
    if not isinstance(command, dict) or set(command) != allowed:
        raise CreateShapeV2Error("CreateShapeV2 contains non-intent/authoritative fields")
    if command.get("kind") != "create_shape_v2":
        raise CreateShapeV2Error("CreateShapeV2 kind is required")
    try:
        validate_uuid7_node_id_v1(command.get("node_id"))
        validate_creation_paint_v1(command.get("paint"))
    except CreateShapeError as exc:
        raise CreateShapeV2Error(str(exc)) from exc

    page_id = command.get("page_id")
    if not isinstance(page_id, str) or not page_id:
        raise CreateShapeV2Error("CreateShapeV2 page_id is required")

    destination = command.get("destination")
    if not isinstance(destination, dict) or set(destination) != {"kind", "id"}:
        raise CreateShapeV2Error("CreateShapeV2 destination must contain kind/id")
    if destination.get("kind") not in {"page", "group"}:
        raise CreateShapeV2Error("unsupported CreateShapeV2 destination kind")
    if not isinstance(destination.get("id"), str) or not destination["id"]:
        raise CreateShapeV2Error("CreateShapeV2 destination id is required")
    if destination["kind"] == "page" and destination["id"] != page_id:
        raise CreateShapeV2Error("Page destination must equal page_id")

    placement = command.get("placement")
    if not isinstance(placement, dict) or set(placement) != {
        "status",
        "desired_effective_page_rect",
        "destination_local_rect",
    }:
        raise CreateShapeV2Error("CreateShapeV2 placement shape is invalid")
    if placement.get("status") != "contained":
        raise CreateShapeV2Error("CreateShapeV2 accepts contained placement only")
    try:
        validate_rect_emu_v1(
            placement.get("desired_effective_page_rect"),
            "desired_effective_page_rect",
        )
        validate_rect_emu_v1(
            placement.get("destination_local_rect"),
            "destination_local_rect",
        )
    except CreateShapeError as exc:
        raise CreateShapeV2Error(str(exc)) from exc
    if destination["kind"] == "page" and (
        placement["desired_effective_page_rect"] != placement["destination_local_rect"]
    ):
        raise CreateShapeV2Error("Page placement must be identity-local")

    lane = command.get("expected_order_lane")
    if not isinstance(lane, list):
        raise CreateShapeV2Error("expected_order_lane must be list")
    try:
        validate_authored_lane(lane)
    except ValueError as exc:
        raise CreateShapeV2Error(str(exc)) from exc

    policy = command.get("insertion_policy")
    if destination["kind"] == "page":
        if policy != "append_authored_front":
            raise CreateShapeV2Error("Page CreateShapeV2 requires append_authored_front")
    else:
        if policy != "append_block_at_front":
            raise CreateShapeV2Error("Group CreateShapeV2 requires append_block_at_front")


def _validate_collision(project: dict, node_id: str) -> None:
    for registry_name in ("shapes", "text_frames", "picture_frames", "groups", "nodes"):
        registry = project.get(registry_name)
        if isinstance(registry, dict) and node_id in registry:
            raise CreateShapeV2Error("create_shape_v2_node_id_collision")


def apply_create_shape_v2(base_project: dict, command: dict) -> tuple[dict, dict, list]:
    validate_create_shape_v2_intent(command)
    project = copy.deepcopy(base_project)
    page_id = command["page_id"]
    pages = project.get("pages")
    if not isinstance(pages, dict) or page_id not in pages:
        raise CreateShapeV2Error("invalid_create_shape_v2_page")
    page = pages[page_id]
    if isinstance(page, dict) and page.get("authoring_enabled") is False:
        raise CreateShapeV2Error("create_shape_v2_page_not_authorable")

    node_id = command["node_id"]
    _validate_collision(project, node_id)
    shapes = project.setdefault("shapes", {})
    if not isinstance(shapes, dict):
        raise CreateShapeV2Error("canonical shapes registry must be object")

    destination = command["destination"]
    placement = command["placement"]
    parent_id = destination["id"]
    local_bounds = copy.deepcopy(placement["destination_local_rect"])

    if destination["kind"] == "page":
        authored = project.get("authored_stacks")
        if not isinstance(authored, dict):
            raise CreateShapeV2Error("authored_stacks registry is required")
        lane = authored.get(page_id)
        if lane != command["expected_order_lane"]:
            raise CreateShapeV2Error("stale CreateShapeV2 Page authored lane")
        authored[page_id] = append_authored_member(lane, node_id)
        order_before = list(lane)
        order_after = list(authored[page_id])
    else:
        groups = project.get("groups")
        if not isinstance(groups, dict) or parent_id not in groups:
            raise CreateShapeV2Error("invalid CreateShapeV2 Group destination")
        group = groups[parent_id]
        if not isinstance(group, dict):
            raise CreateShapeV2Error("Group destination must be object")
        if group.get("provenance") != {"kind": "author_created"}:
            raise CreateShapeV2Error("source-backed Group destination is unsupported")
        if group.get("page_id") != page_id:
            raise CreateShapeV2Error("Group destination page mismatch")
        children = group.get("children")
        if children != command["expected_order_lane"]:
            raise CreateShapeV2Error("stale CreateShapeV2 Group.children lane")
        plan = plan_group_child_batch_insert_v1(
            parent_group_id=parent_id,
            expected_children=tuple(children),
            current_children=tuple(children),
            candidates=(GroupChildCandidateV1(node_id=node_id, parent_group_id=parent_id),),
            policy="append_block_at_front",
        )
        group["children"] = list(plan.after_children)
        order_before = list(plan.before_children)
        order_after = list(plan.after_children)

    canonical_paint = {
        "fill": copy.deepcopy(command["paint"]["fill"]),
        "stroke": copy.deepcopy(command["paint"]["stroke"]),
        "provenance": {"kind": "author_created"},
    }
    entity = {
        "node_id": node_id,
        "kind": "shape",
        "shape_kind": "rectangle",
        "page_id": page_id,
        "parent_id": parent_id,
        "bounds": local_bounds,
        "transform": {"kind": "identity"},
        "paint": canonical_paint,
        "provenance": {"kind": "author_created"},
    }
    shapes[node_id] = entity

    operation = {
        "kind": "create_shape_v2",
        "node_id": node_id,
        "page_id": page_id,
        "destination": copy.deepcopy(destination),
        "parent_id": parent_id,
        "shape_kind": "rectangle",
        "bounds": copy.deepcopy(local_bounds),
        "desired_effective_page_rect": copy.deepcopy(
            placement["desired_effective_page_rect"]
        ),
        "transform": {"kind": "identity"},
        "paint": copy.deepcopy(canonical_paint),
        "provenance": {"kind": "author_created"},
        "order_before": order_before,
        "order_after": order_after,
        "insertion_policy": command["insertion_policy"],
    }
    operations = project.get("operations")
    if not isinstance(operations, list):
        raise CreateShapeV2Error("canonical project operations must be list")
    operations.append(copy.deepcopy(operation))

    return operation, project, [
        {"key": "shape.created", "state": "supported", "note": None},
        {"key": "container.order", "state": "supported", "note": None},
        {"key": "layout.scene", "state": "invalidated", "note": None},
        {"key": "editable_export", "state": "invalidated", "note": None},
    ]
