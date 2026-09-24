#!/usr/bin/env python3
"""Canonical Chaptera-only CreateShape V1.

Creates one direct page-owned ordinary rectangle in EditorProject state.
No Publisher allocator/SPID/Oid/source write or z-order lane is owned here.
"""

from __future__ import annotations

import copy
import secrets
import time
import uuid

MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU


class CreateShapeError(ValueError):
    pass


def validate_uuid7_node_id_v1(value: str) -> None:
    if not isinstance(value, str) or not value:
        raise CreateShapeError("CreateShape node_id is required")
    try:
        parsed = uuid.UUID(value)
    except (ValueError, AttributeError) as exc:
        raise CreateShapeError("CreateShape node_id must be canonical UUIDv7") from exc
    if (
        str(parsed) != value
        or parsed.version != 7
        or parsed.variant != uuid.RFC_4122
    ):
        raise CreateShapeError("CreateShape node_id must be canonical UUIDv7")


def new_uuid7_node_id_v1(*, now_ms: int | None = None, random_bits: int | None = None) -> str:
    if now_ms is None:
        now_ms = time.time_ns() // 1_000_000
    if not isinstance(now_ms, int) or isinstance(now_ms, bool) or not (0 <= now_ms < 1 << 48):
        raise CreateShapeError("UUIDv7 timestamp must fit 48 bits")

    if random_bits is None:
        random_bits = secrets.randbits(74)
    if (
        not isinstance(random_bits, int)
        or isinstance(random_bits, bool)
        or not (0 <= random_bits < 1 << 74)
    ):
        raise CreateShapeError("UUIDv7 random_bits must fit 74 bits")

    rand_a = random_bits >> 62
    rand_b = random_bits & ((1 << 62) - 1)
    value = (
        (now_ms << 80)
        | (0x7 << 76)
        | (rand_a << 64)
        | (0b10 << 62)
        | rand_b
    )
    result = str(uuid.UUID(int=value))
    validate_uuid7_node_id_v1(result)
    return result


def validate_rect_emu_v1(rect: dict, label: str = "bounds") -> None:
    if not isinstance(rect, dict) or set(rect) != {"x", "y", "width", "height"}:
        raise CreateShapeError(f"{label} must contain exactly x/y/width/height")
    for field in ("x", "y"):
        value = rect[field]
        if (
            not isinstance(value, int)
            or isinstance(value, bool)
            or value < MIN_SAFE_EMU
            or value > MAX_SAFE_EMU
        ):
            raise CreateShapeError(f"{label}.{field} must be a JavaScript-safe EMU integer")
    for field in ("width", "height"):
        value = rect[field]
        if (
            not isinstance(value, int)
            or isinstance(value, bool)
            or value <= 0
            or value > MAX_SAFE_EMU
        ):
            raise CreateShapeError(f"{label}.{field} must be a positive JavaScript-safe EMU integer")

    right = rect["x"] + rect["width"]
    bottom = rect["y"] + rect["height"]
    if (
        right < MIN_SAFE_EMU
        or right > MAX_SAFE_EMU
        or bottom < MIN_SAFE_EMU
        or bottom > MAX_SAFE_EMU
    ):
        raise CreateShapeError(f"{label} edges exceed JavaScript-safe EMU range")


def _validate_color(color: dict, label: str) -> None:
    if not isinstance(color, dict) or set(color) != {"r", "g", "b"}:
        raise CreateShapeError(f"{label} must contain exactly r/g/b")
    for channel in ("r", "g", "b"):
        value = color[channel]
        if (
            not isinstance(value, int)
            or isinstance(value, bool)
            or value < 0
            or value > 255
        ):
            raise CreateShapeError(f"{label}.{channel} must be an sRGB byte")


def validate_creation_paint_v1(paint: dict) -> None:
    if not isinstance(paint, dict) or set(paint) != {"fill", "stroke"}:
        raise CreateShapeError("CreateShape paint must explicitly contain fill/stroke")

    fill = paint["fill"]
    if not isinstance(fill, dict) or set(fill) != {"visible", "color"}:
        raise CreateShapeError("CreateShape paint.fill must contain visible/color")
    if not isinstance(fill["visible"], bool):
        raise CreateShapeError("CreateShape paint.fill.visible must be boolean")
    _validate_color(fill["color"], "CreateShape paint.fill.color")

    stroke = paint["stroke"]
    if not isinstance(stroke, dict) or set(stroke) != {"visible", "color", "width_emu"}:
        raise CreateShapeError("CreateShape paint.stroke must contain visible/color/width_emu")
    if not isinstance(stroke["visible"], bool):
        raise CreateShapeError("CreateShape paint.stroke.visible must be boolean")
    _validate_color(stroke["color"], "CreateShape paint.stroke.color")
    width = stroke["width_emu"]
    if (
        not isinstance(width, int)
        or isinstance(width, bool)
        or width <= 0
        or width > MAX_SAFE_EMU
    ):
        raise CreateShapeError("CreateShape paint.stroke.width_emu must be positive safe EMU")


def validate_create_shape_intent_v1(command: dict) -> None:
    if not isinstance(command, dict) or set(command) != {
        "kind",
        "node_id",
        "page_id",
        "bounds",
        "paint",
    }:
        raise CreateShapeError("CreateShape contains non-intent/authoritative fields")
    if command.get("kind") != "create_shape":
        raise CreateShapeError("CreateShape kind is required")
    validate_uuid7_node_id_v1(command.get("node_id"))
    page_id = command.get("page_id")
    if not isinstance(page_id, str) or not page_id:
        raise CreateShapeError("CreateShape page_id is required")
    validate_rect_emu_v1(command.get("bounds"))
    validate_creation_paint_v1(command.get("paint"))


def apply_create_shape_v1(base_project: dict, command: dict) -> tuple[dict, dict, list]:
    validate_create_shape_intent_v1(command)
    project = copy.deepcopy(base_project)

    pages = project.get("pages")
    if not isinstance(pages, dict) or command["page_id"] not in pages:
        raise CreateShapeError("invalid_create_shape_page")
    page = pages[command["page_id"]]
    if isinstance(page, dict) and page.get("authoring_enabled") is False:
        raise CreateShapeError("create_shape_page_not_authorable")

    node_id = command["node_id"]
    for registry_name in ("shapes", "text_frames", "picture_frames", "groups", "nodes"):
        registry = project.get(registry_name)
        if isinstance(registry, dict) and node_id in registry:
            raise CreateShapeError("create_shape_node_id_collision")

    shapes = project.setdefault("shapes", {})
    if not isinstance(shapes, dict):
        raise CreateShapeError("canonical shapes registry must be object")

    canonical_paint = {
        "fill": copy.deepcopy(command["paint"]["fill"]),
        "stroke": copy.deepcopy(command["paint"]["stroke"]),
        "provenance": {"kind": "author_created"},
    }
    entity = {
        "node_id": node_id,
        "kind": "shape",
        "shape_kind": "rectangle",
        "page_id": command["page_id"],
        "parent_id": command["page_id"],
        "bounds": copy.deepcopy(command["bounds"]),
        "transform": {"kind": "identity"},
        "paint": canonical_paint,
        "provenance": {"kind": "author_created"},
    }
    shapes[node_id] = entity

    operation = {
        "kind": "create_shape",
        "node_id": node_id,
        "page_id": command["page_id"],
        "shape_kind": "rectangle",
        "bounds": copy.deepcopy(command["bounds"]),
        "transform": {"kind": "identity"},
        "paint": copy.deepcopy(canonical_paint),
        "provenance": {"kind": "author_created"},
    }
    operations = project.get("operations")
    if not isinstance(operations, list):
        raise CreateShapeError("canonical project operations must be list")
    operations.append(copy.deepcopy(operation))

    consequences = [
        {"key": "shape.created", "state": "supported", "note": None},
        {"key": "layout.scene", "state": "invalidated", "note": None},
        {"key": "editable_export", "state": "invalidated", "note": None},
    ]
    return operation, project, consequences
