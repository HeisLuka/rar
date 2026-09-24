#!/usr/bin/env python3
"""Bounded Fill/Stroke inspector adapter over canonical Shape paint V1."""

from __future__ import annotations

import copy
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation

EMU_PER_POINT = 12_700


class ShapePaintInspectorError(ValueError):
    pass


@dataclass(frozen=True)
class ShapePaintInspectorSnapshotV1:
    node_id: str
    editable: bool
    reason: str | None
    fill: dict | None
    stroke: dict | None


def _srgb(r: int, g: int, b: int) -> dict:
    result = {"r": r, "g": g, "b": b}
    for channel, value in result.items():
        if not isinstance(value, int) or isinstance(value, bool) or not 0 <= value <= 255:
            raise ShapePaintInspectorError(f"{channel} must be an sRGB byte")
    return result


def points_text_to_emu_v1(value: str) -> int:
    """Exact decimal points -> integer EMU; float input is intentionally rejected."""
    if not isinstance(value, str) or not value.strip():
        raise ShapePaintInspectorError("stroke width must be non-empty decimal point text")
    try:
        points = Decimal(value.strip())
    except InvalidOperation as exc:
        raise ShapePaintInspectorError("stroke width must be valid decimal point text") from exc
    if not points.is_finite() or points <= 0:
        raise ShapePaintInspectorError("stroke width must be positive finite points")
    emu = points * EMU_PER_POINT
    if emu != emu.to_integral_value():
        raise ShapePaintInspectorError("stroke width is not exactly representable in integer EMU")
    result = int(emu)
    if result <= 0 or result > 9_007_199_254_740_991:
        raise ShapePaintInspectorError("stroke width EMU is outside safe range")
    return result


def inspect_shape_paint_v1(shape: dict) -> ShapePaintInspectorSnapshotV1:
    if not isinstance(shape, dict):
        raise ShapePaintInspectorError("selected shape must be object")
    node_id = shape.get("node_id")
    if not isinstance(node_id, str) or not node_id:
        raise ShapePaintInspectorError("selected shape node_id is required")
    if shape.get("kind") != "shape" or shape.get("shape_kind", "rectangle") != "rectangle":
        return ShapePaintInspectorSnapshotV1(node_id, False, "unsupported_shape_class", None, None)

    paint = shape.get("paint")
    if not isinstance(paint, dict):
        return ShapePaintInspectorSnapshotV1(node_id, False, "unresolved_paint", None, None)
    if paint.get("provenance") != {"kind": "author_created"}:
        return ShapePaintInspectorSnapshotV1(node_id, False, "source_or_inherited_paint", None, None)
    fill = paint.get("fill")
    stroke = paint.get("stroke")
    if not isinstance(fill, dict) or not isinstance(stroke, dict):
        return ShapePaintInspectorSnapshotV1(node_id, False, "unresolved_paint", None, None)

    return ShapePaintInspectorSnapshotV1(
        node_id=node_id,
        editable=True,
        reason=None,
        fill=copy.deepcopy(fill),
        stroke=copy.deepcopy(stroke),
    )


def build_set_fill_request_v1(
    *,
    snapshot: ShapePaintInspectorSnapshotV1,
    visible: bool,
    r: int,
    g: int,
    b: int,
    document_id: str,
    source_hash: str,
    base_revision_id: str,
    client_operation_id: str,
) -> dict | None:
    if not snapshot.editable or snapshot.fill is None:
        raise ShapePaintInspectorError(snapshot.reason or "shape paint is read-only")
    if not isinstance(visible, bool):
        raise ShapePaintInspectorError("fill visible must be boolean")
    after = {"visible": visible, "color": _srgb(r, g, b)}
    if after == snapshot.fill:
        return None
    return {
        "protocol_version": "chaptera.shape-fill-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": base_revision_id,
        "client_operation_id": client_operation_id,
        "command": {
            "kind": "set_fill",
            "node_id": snapshot.node_id,
            "expected_before": copy.deepcopy(snapshot.fill),
            "after": after,
        },
    }


def build_set_stroke_request_v1(
    *,
    snapshot: ShapePaintInspectorSnapshotV1,
    visible: bool,
    r: int,
    g: int,
    b: int,
    width_points: str,
    document_id: str,
    source_hash: str,
    base_revision_id: str,
    client_operation_id: str,
) -> dict | None:
    if not snapshot.editable or snapshot.stroke is None:
        raise ShapePaintInspectorError(snapshot.reason or "shape paint is read-only")
    if not isinstance(visible, bool):
        raise ShapePaintInspectorError("stroke visible must be boolean")
    after = {
        "visible": visible,
        "color": _srgb(r, g, b),
        "width_emu": points_text_to_emu_v1(width_points),
    }
    if after == snapshot.stroke:
        return None
    return {
        "protocol_version": "chaptera.shape-stroke-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": base_revision_id,
        "client_operation_id": client_operation_id,
        "command": {
            "kind": "set_stroke",
            "node_id": snapshot.node_id,
            "expected_before": copy.deepcopy(snapshot.stroke),
            "after": after,
        },
    }
