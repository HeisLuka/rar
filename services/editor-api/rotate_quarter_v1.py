#!/usr/bin/env python3
"""Exact authored-shape quarter-turn mutation V1.

The bounded V1 slice rotates only Chaptera author-created, direct page-owned
ordinary rectangles. It owns no arbitrary-angle trigonometry, source-backed
shape mutation, Group transform propagation, image-inner rotation, flips, or
native PUB write.

Affine convention:
    x' = a*x + c*y + tx
    y' = b*x + d*y + ty

Positive quarter_turns are clockwise in the page coordinate system (Y grows
downward). All coefficients and translations are exact canonical decimal
strings; this slice emits only integers or half-EMU values.
"""

from __future__ import annotations

import copy
from fractions import Fraction

from create_shape_v1 import validate_rect_emu_v1


class RotateQuarterError(ValueError):
    pass


_AFFINE_FIELDS = {"a", "b", "c", "d", "tx", "ty"}
_IDENTITY_AFFINE = {
    "a": "1",
    "b": "0",
    "c": "0",
    "d": "1",
    "tx": "0",
    "ty": "0",
}
_ALLOWED_LINEAR = {
    (1, 0, 0, 1),
    (0, 1, -1, 0),
    (-1, 0, 0, -1),
    (0, -1, 1, 0),
}


def _parse_exact_half(value: object, label: str) -> Fraction:
    if not isinstance(value, str) or not value:
        raise RotateQuarterError(f"{label} must be a canonical decimal string")
    try:
        parsed = Fraction(value)
    except (ValueError, ZeroDivisionError) as exc:
        raise RotateQuarterError(f"{label} must be an exact decimal string") from exc
    if parsed.denominator not in (1, 2):
        raise RotateQuarterError(f"{label} must resolve to integer or half-EMU")
    if _format_exact_half(parsed) != value:
        raise RotateQuarterError(f"{label} is not canonical")
    return parsed


def _format_exact_half(value: Fraction) -> str:
    if value.denominator == 1:
        return str(value.numerator)
    if value.denominator != 2:
        raise RotateQuarterError("quarter-turn affine value is not half-EMU exact")
    sign = "-" if value.numerator < 0 else ""
    magnitude = abs(value.numerator)
    return f"{sign}{magnitude // 2}.5"


def validate_affine_v1(value: object, label: str = "affine") -> dict[str, str]:
    if not isinstance(value, dict) or set(value) != _AFFINE_FIELDS:
        raise RotateQuarterError(f"{label} must contain exactly a/b/c/d/tx/ty")

    parsed = {
        field: _parse_exact_half(value[field], f"{label}.{field}")
        for field in ("a", "b", "c", "d", "tx", "ty")
    }
    linear = tuple(int(parsed[field]) for field in ("a", "b", "c", "d"))
    if any(parsed[field].denominator != 1 for field in ("a", "b", "c", "d")):
        raise RotateQuarterError(f"{label} linear coefficients must be integers")
    if linear not in _ALLOWED_LINEAR:
        raise RotateQuarterError(
            f"{label} must be an admitted non-flipped quarter-turn affine"
        )
    a, b, c, d = linear
    if a * d - b * c != 1:
        raise RotateQuarterError(f"{label} flipped/reflected transforms are unsupported")

    return {field: _format_exact_half(parsed[field]) for field in _AFFINE_FIELDS}


def canonical_entity_affine_v1(transform: object) -> dict[str, str]:
    if transform == {"kind": "identity"}:
        return dict(_IDENTITY_AFFINE)
    if not isinstance(transform, dict) or transform.get("kind") != "affine":
        raise RotateQuarterError("target transform is unsupported")
    if set(transform) != {"kind", *_AFFINE_FIELDS}:
        raise RotateQuarterError("affine target transform contains unsupported fields")
    return validate_affine_v1(
        {field: transform[field] for field in _AFFINE_FIELDS},
        "target transform",
    )


def validate_rotate_node_quarter_intent_v1(command: object) -> None:
    allowed = {
        "kind",
        "node_id",
        "expected_before",
        "pivot_policy",
        "quarter_turns",
    }
    if (
        not isinstance(command, dict)
        or command.get("kind") != "rotate_node_quarter_turn"
        or set(command) != allowed
    ):
        raise RotateQuarterError(
            "RotateNodeQuarterTurn contains non-intent/authoritative fields"
        )
    node_id = command.get("node_id")
    if not isinstance(node_id, str) or not node_id:
        raise RotateQuarterError("RotateNodeQuarterTurn node_id is required")
    validate_affine_v1(command.get("expected_before"), "expected_before")
    if command.get("pivot_policy") != "authored_bounds_center":
        raise RotateQuarterError(
            "RotateNodeQuarterTurn pivot_policy must be authored_bounds_center"
        )
    turns = command.get("quarter_turns")
    if not isinstance(turns, int) or isinstance(turns, bool):
        raise RotateQuarterError("RotateNodeQuarterTurn quarter_turns must be integer")
    if turns % 4 == 0:
        raise RotateQuarterError("full-turn/no-op rotation is not a durable edit")


def authored_bounds_center_v1(bounds: dict) -> dict[str, str]:
    try:
        validate_rect_emu_v1(bounds, "rotate target bounds")
    except ValueError as exc:
        raise RotateQuarterError(str(exc)) from exc
    return {
        "x": _format_exact_half(
            Fraction(bounds["x"]) + Fraction(bounds["width"], 2)
        ),
        "y": _format_exact_half(
            Fraction(bounds["y"]) + Fraction(bounds["height"], 2)
        ),
    }


def _affine_fractions(value: dict[str, str]) -> tuple[Fraction, ...]:
    return tuple(Fraction(value[field]) for field in ("a", "b", "c", "d", "tx", "ty"))


def _affine_dict(values: tuple[Fraction, ...]) -> dict[str, str]:
    return {
        field: _format_exact_half(value)
        for field, value in zip(("a", "b", "c", "d", "tx", "ty"), values)
    }


def _compose_affine(
    left: dict[str, str],
    right: dict[str, str],
) -> dict[str, str]:
    la, lb, lc, ld, ltx, lty = _affine_fractions(left)
    ra, rb, rc, rd, rtx, rty = _affine_fractions(right)
    values = (
        la * ra + lc * rb,
        lb * ra + ld * rb,
        la * rc + lc * rd,
        lb * rc + ld * rd,
        la * rtx + lc * rty + ltx,
        lb * rtx + ld * rty + lty,
    )
    result = _affine_dict(values)
    return validate_affine_v1(result, "composed affine")


def _quarter_turn_about_center(
    *,
    pivot_x: Fraction,
    pivot_y: Fraction,
    quarter_turns: int,
) -> dict[str, str]:
    turns = quarter_turns % 4
    if turns == 1:
        a, b, c, d = 0, 1, -1, 0
    elif turns == 2:
        a, b, c, d = -1, 0, 0, -1
    elif turns == 3:
        a, b, c, d = 0, -1, 1, 0
    else:
        raise RotateQuarterError("full-turn/no-op rotation is not admitted")

    a_f, b_f, c_f, d_f = map(Fraction, (a, b, c, d))
    tx = pivot_x - (a_f * pivot_x + c_f * pivot_y)
    ty = pivot_y - (b_f * pivot_x + d_f * pivot_y)
    return _affine_dict((a_f, b_f, c_f, d_f, tx, ty))


def apply_rotate_node_quarter_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list]:
    validate_rotate_node_quarter_intent_v1(command)

    shapes = base_project.get("shapes")
    if not isinstance(shapes, dict):
        raise RotateQuarterError("canonical shapes registry is required")
    node_id = command["node_id"]
    entity = shapes.get(node_id)
    if not isinstance(entity, dict):
        raise RotateQuarterError("rotate target is not an author-created shape")
    if (
        entity.get("kind") != "shape"
        or entity.get("shape_kind") != "rectangle"
        or entity.get("provenance") != {"kind": "author_created"}
    ):
        raise RotateQuarterError("rotate target is unsupported or source-backed")
    if entity.get("parent_id") != entity.get("page_id"):
        raise RotateQuarterError(
            "V1 rotation admits only direct page-owned authored rectangles"
        )
    if "source_ref" in entity:
        raise RotateQuarterError("source-backed rotate targets are unsupported")

    bounds = entity.get("bounds")
    pivot = authored_bounds_center_v1(bounds)

    before = canonical_entity_affine_v1(entity.get("transform"))
    expected_before = validate_affine_v1(
        command["expected_before"],
        "expected_before",
    )
    if before != expected_before:
        raise RotateQuarterError("rotate target transform changed since expected_before")

    pivot_x = Fraction(pivot["x"])
    pivot_y = Fraction(pivot["y"])
    canonical_turns = command["quarter_turns"] % 4
    turn = _quarter_turn_about_center(
        pivot_x=pivot_x,
        pivot_y=pivot_y,
        quarter_turns=canonical_turns,
    )
    after = _compose_affine(turn, before)
    if after == before:
        raise RotateQuarterError("canonical quarter turn produced no state change")

    resulting = copy.deepcopy(base_project)
    resulting_entity = resulting["shapes"][node_id]
    resulting_entity["transform"] = {"kind": "affine", **after}

    operation = {
        "kind": "rotate_node_quarter_turn",
        "node_id": node_id,
        "before": before,
        "after": after,
        "pivot_policy": "authored_bounds_center",
        "pivot": pivot,
        "quarter_turns": canonical_turns,
    }
    resulting.setdefault("operations", []).append(copy.deepcopy(operation))

    consequences = [
        {
            "key": "editable_output.affine_transform",
            "state": "supported",
            "note": "exact authored quarter-turn affine is materialized",
        },
        {
            "key": "native_pub_write",
            "state": "unsupported",
            "note": "source PUB remains immutable in V1",
        },
    ]
    return operation, resulting, consequences
