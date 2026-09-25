#!/usr/bin/env python3
"""Canonical SetPageExtentV1 authoring operation.

This is Chaptera KeepObjectsFixed semantics only:
- mutate one explicit canonical page extent;
- preserve every object geometry/transform and Story byte-for-byte;
- never infer Publisher paper/page-tracking morph behavior.
"""

from __future__ import annotations

import copy


MAX_SAFE_EMU = 9_007_199_254_740_991


class PageExtentV1Error(ValueError):
    pass


def _extent(value: object, label: str) -> dict:
    if (
        not isinstance(value, dict)
        or set(value) != {"width_emu", "height_emu"}
        or any(
            not isinstance(value.get(field), int)
            or isinstance(value.get(field), bool)
            or value[field] <= 0
            or value[field] > MAX_SAFE_EMU
            for field in ("width_emu", "height_emu")
        )
    ):
        raise PageExtentV1Error(
            f"{label} must contain positive JavaScript-safe integer EMU width/height"
        )
    return {"width_emu": value["width_emu"], "height_emu": value["height_emu"]}



def validate_page_extent_authority_v1(command: dict, authority: dict) -> None:
    """Validate caller-supplied page-role/tracking evidence.

    The intent never carries these authoritative fields. A runtime that knows
    the document projection supplies this evidence out-of-band before commit.
    """
    if not isinstance(authority, dict) or set(authority) != {
        "page_id",
        "page_role",
        "tracking_morph",
    }:
        raise PageExtentV1Error("page extent authority evidence is incomplete")
    if authority.get("page_id") != command.get("page_id"):
        raise PageExtentV1Error("page extent authority targets a different page")

    role = authority.get("page_role")
    if role not in {"customer_page", "chaptera_authored_page"}:
        raise PageExtentV1Error("page extent target role is not authorable")

    tracking = authority.get("tracking_morph")
    if tracking not in {"absent", "classified_safe"}:
        raise PageExtentV1Error("page tracking/morph state is not classified safe")


def apply_set_page_extent_v1(base_project: dict, command: dict) -> tuple[dict, dict, list]:
    if not isinstance(command, dict) or command.get("kind") != "set_page_extent":
        raise PageExtentV1Error("SetPageExtentV1 command is required")

    page_id = command.get("page_id")
    if not isinstance(page_id, str) or not page_id:
        raise PageExtentV1Error("SetPageExtentV1 page_id is required")
    if command.get("semantics") != "keep_objects_fixed":
        raise PageExtentV1Error("SetPageExtentV1 only supports KeepObjectsFixed semantics")

    expected = _extent(command.get("expected_before"), "expected_before")
    after = _extent(command.get("after"), "after")
    if expected == after:
        raise PageExtentV1Error("SetPageExtentV1 must not be a no-op")

    pages = base_project.get("pages") if isinstance(base_project, dict) else None
    if not isinstance(pages, dict):
        raise PageExtentV1Error("canonical pages registry must be object")
    page = pages.get(page_id)
    if not isinstance(page, dict):
        raise PageExtentV1Error("SetPageExtentV1 target page is missing")

    current = _extent(page.get("size"), "current page size")
    if current != expected:
        raise PageExtentV1Error("stale_page_extent")

    operations = base_project.get("operations")
    if not isinstance(operations, list):
        raise PageExtentV1Error("canonical project operations must be list")

    operation = {
        "kind": "set_page_extent",
        "page_id": page_id,
        "before": expected,
        "after": after,
        "semantics": "keep_objects_fixed",
    }

    project = copy.deepcopy(base_project)
    project["pages"][page_id]["size"] = copy.deepcopy(after)
    project["operations"].append(copy.deepcopy(operation))

    return operation, project, [
        {"key": "page.geometry", "state": "supported", "note": "keep_objects_fixed"},
        {"key": "layout.scene", "state": "invalidated", "note": None},
        {"key": "editable_export", "state": "invalidated", "note": None},
    ]
