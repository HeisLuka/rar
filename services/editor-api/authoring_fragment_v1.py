#!/usr/bin/env python3
"""Canonical rectangle AuthoringFragment/PasteFragment V1.

Clipboard/transport is not document truth. This module captures one admitted
author-created ordinary rectangle into a payload-local fragment identity and
materializes one persisted destination UUIDv7 identity into canonical authored
state. Page.children and authored z-order are intentionally not owned here.
"""

from __future__ import annotations

import copy

from create_shape_v1 import (
    CreateShapeError,
    validate_creation_paint_v1,
    validate_rect_emu_v1,
    validate_uuid7_node_id_v1,
)

AUTHORING_FRAGMENT_SCHEMA_V1 = "chaptera.authoring-fragment.v1"
SINGLE_RECTANGLE_ENTITY_ID_V1 = "entity:0"
MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU


class AuthoringFragmentError(ValueError):
    pass


def capture_rectangle_fragment_v1(project: dict, node_id: str) -> dict:
    if not isinstance(project, dict):
        raise AuthoringFragmentError("project must be object")
    shapes = project.get("shapes")
    if not isinstance(shapes, dict):
        raise AuthoringFragmentError("canonical shapes registry must be object")
    shape = shapes.get(node_id)
    if not isinstance(shape, dict):
        raise AuthoringFragmentError("fragment source shape is missing")

    _validate_supported_source_shape(shape)
    paint = shape["paint"]
    return {
        "schema_version": AUTHORING_FRAGMENT_SCHEMA_V1,
        "rectangle": {
            "fragment_entity_id": SINGLE_RECTANGLE_ENTITY_ID_V1,
            "bounds": copy.deepcopy(shape["bounds"]),
            "fill": copy.deepcopy(paint["fill"]),
            "stroke": copy.deepcopy(paint["stroke"]),
            "source_provenance": {"source_node_id": node_id},
        },
    }


def validate_authoring_fragment_v1(fragment: dict) -> None:
    if not isinstance(fragment, dict) or set(fragment) != {"schema_version", "rectangle"}:
        raise AuthoringFragmentError("AuthoringFragmentV1 must contain schema_version/rectangle")
    if fragment.get("schema_version") != AUTHORING_FRAGMENT_SCHEMA_V1:
        raise AuthoringFragmentError("unsupported AuthoringFragmentV1 schema")

    rectangle = fragment.get("rectangle")
    required = {"fragment_entity_id", "bounds", "fill", "stroke"}
    allowed = required | {"source_provenance"}
    if not isinstance(rectangle, dict) or not required.issubset(rectangle) or not set(rectangle) <= allowed:
        raise AuthoringFragmentError("rectangle fragment is malformed")
    if rectangle.get("fragment_entity_id") != SINGLE_RECTANGLE_ENTITY_ID_V1:
        raise AuthoringFragmentError("unsupported fragment entity identity")

    validate_rect_emu_v1(rectangle.get("bounds"), "fragment bounds")
    validate_creation_paint_v1(
        {
            "fill": rectangle.get("fill"),
            "stroke": rectangle.get("stroke"),
        }
    )

    if "source_provenance" in rectangle:
        provenance = rectangle["source_provenance"]
        if (
            not isinstance(provenance, dict)
            or set(provenance) != {"source_node_id"}
            or not isinstance(provenance.get("source_node_id"), str)
            or not provenance["source_node_id"]
        ):
            raise AuthoringFragmentError("fragment source provenance is malformed")


def validate_paste_fragment_intent_v1(command: dict) -> None:
    if not isinstance(command, dict) or set(command) != {
        "kind",
        "fragment",
        "identity_map",
        "destination",
        "placement",
    }:
        raise AuthoringFragmentError("PasteFragment contains non-intent/authoritative fields")
    if command.get("kind") != "paste_fragment":
        raise AuthoringFragmentError("PasteFragment kind is required")

    validate_authoring_fragment_v1(command.get("fragment"))

    identity_map = command.get("identity_map")
    if (
        not isinstance(identity_map, dict)
        or set(identity_map) != {"fragment_entity_id", "destination_node_id"}
        or identity_map.get("fragment_entity_id") != SINGLE_RECTANGLE_ENTITY_ID_V1
    ):
        raise AuthoringFragmentError("PasteFragment identity_map is malformed")
    try:
        validate_uuid7_node_id_v1(identity_map.get("destination_node_id"))
    except CreateShapeError as exc:
        raise AuthoringFragmentError("PasteFragment destination_node_id must be canonical UUIDv7") from exc

    source_provenance = command["fragment"]["rectangle"].get("source_provenance")
    if (
        isinstance(source_provenance, dict)
        and source_provenance.get("source_node_id") == identity_map["destination_node_id"]
    ):
        raise AuthoringFragmentError("PasteFragment destination identity reuses source identity")

    destination = command.get("destination")
    if (
        not isinstance(destination, dict)
        or set(destination) != {"kind", "page_id"}
        or destination.get("kind") != "page"
        or not isinstance(destination.get("page_id"), str)
        or not destination["page_id"]
    ):
        raise AuthoringFragmentError("PasteFragment destination must be one page")

    placement = command.get("placement")
    if (
        not isinstance(placement, dict)
        or set(placement) != {"kind", "dx_emu", "dy_emu"}
        or placement.get("kind") != "translate"
    ):
        raise AuthoringFragmentError("PasteFragment placement must be exact translation")
    for field in ("dx_emu", "dy_emu"):
        value = placement.get(field)
        if (
            not isinstance(value, int)
            or isinstance(value, bool)
            or value < MIN_SAFE_EMU
            or value > MAX_SAFE_EMU
        ):
            raise AuthoringFragmentError(f"PasteFragment {field} must be JavaScript-safe EMU")

    _materialized_entity_v1(command)


def canonical_paste_fragment_operation_v1(command: dict) -> dict:
    validate_paste_fragment_intent_v1(command)
    return {
        "kind": "paste_fragment",
        "fragment": copy.deepcopy(command["fragment"]),
        "identity_map": copy.deepcopy(command["identity_map"]),
        "destination": copy.deepcopy(command["destination"]),
        "placement": copy.deepcopy(command["placement"]),
        "created_entity": _materialized_entity_v1(command),
    }


def apply_paste_fragment_v1(base_project: dict, command: dict) -> tuple[dict, dict, list]:
    validate_paste_fragment_intent_v1(command)
    project = copy.deepcopy(base_project)

    page_id = command["destination"]["page_id"]
    pages = project.get("pages")
    if not isinstance(pages, dict) or page_id not in pages:
        raise AuthoringFragmentError("invalid_paste_fragment_page")
    page = pages[page_id]
    if isinstance(page, dict) and page.get("authoring_enabled") is False:
        raise AuthoringFragmentError("paste_fragment_page_not_authorable")

    node_id = command["identity_map"]["destination_node_id"]
    for registry_name in ("shapes", "text_frames", "picture_frames", "groups", "nodes"):
        registry = project.get(registry_name)
        if isinstance(registry, dict) and node_id in registry:
            raise AuthoringFragmentError("paste_fragment_node_id_collision")

    shapes = project.setdefault("shapes", {})
    if not isinstance(shapes, dict):
        raise AuthoringFragmentError("canonical shapes registry must be object")

    operation = canonical_paste_fragment_operation_v1(command)
    shapes[node_id] = copy.deepcopy(operation["created_entity"])

    operations = project.get("operations")
    if not isinstance(operations, list):
        raise AuthoringFragmentError("canonical project operations must be list")
    operations.append(copy.deepcopy(operation))

    return operation, project, [
        {"key": "shape.created", "state": "supported", "note": "paste_fragment"},
        {"key": "layout.scene", "state": "invalidated", "note": None},
        {"key": "editable_export", "state": "invalidated", "note": None},
    ]


def _materialized_entity_v1(command: dict) -> dict:
    rectangle = command["fragment"]["rectangle"]
    bounds = rectangle["bounds"]
    dx = command["placement"]["dx_emu"]
    dy = command["placement"]["dy_emu"]

    x = bounds["x"] + dx
    y = bounds["y"] + dy
    translated = {
        "x": x,
        "y": y,
        "width": bounds["width"],
        "height": bounds["height"],
    }
    try:
        validate_rect_emu_v1(translated, "pasted bounds")
    except CreateShapeError as exc:
        raise AuthoringFragmentError("PasteFragment translated bounds are unsafe") from exc

    page_id = command["destination"]["page_id"]
    return {
        "node_id": command["identity_map"]["destination_node_id"],
        "kind": "shape",
        "shape_kind": "rectangle",
        "page_id": page_id,
        "parent_id": page_id,
        "bounds": translated,
        "transform": {"kind": "identity"},
        "paint": {
            "fill": copy.deepcopy(rectangle["fill"]),
            "stroke": copy.deepcopy(rectangle["stroke"]),
            "provenance": {"kind": "author_created"},
        },
        "provenance": {"kind": "author_created"},
    }


def _validate_supported_source_shape(shape: dict) -> None:
    if shape.get("kind") != "shape" or shape.get("shape_kind") != "rectangle":
        raise AuthoringFragmentError("unsupported fragment source shape")
    if shape.get("transform") != {"kind": "identity"}:
        raise AuthoringFragmentError("fragment V1 requires identity-transform rectangle")
    if shape.get("provenance") != {"kind": "author_created"}:
        raise AuthoringFragmentError("fragment V1 supports author-created rectangle only")
    page_id = shape.get("page_id")
    if (
        not isinstance(page_id, str)
        or not page_id
        or shape.get("parent_id") != page_id
    ):
        raise AuthoringFragmentError("fragment source must be direct page-owned")
    validate_rect_emu_v1(shape.get("bounds"), "fragment source bounds")

    paint = shape.get("paint")
    if not isinstance(paint, dict) or set(paint) != {"fill", "stroke", "provenance"}:
        raise AuthoringFragmentError("fragment source paint is malformed")
    if paint.get("provenance") != {"kind": "author_created"}:
        raise AuthoringFragmentError("fragment V1 supports author-created paint only")
    validate_creation_paint_v1(
        {
            "fill": paint.get("fill"),
            "stroke": paint.get("stroke"),
        }
    )
