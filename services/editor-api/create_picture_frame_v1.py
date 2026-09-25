#!/usr/bin/env python3
"""Canonical Chaptera-only CreatePictureFrame V1.

Creates one direct page-owned author-created ImageFrame over an already admitted
editor asset. The operation is source-neutral and does not invent Publisher
Fit/Fill/Pan state.
"""

from __future__ import annotations

import copy
from math import gcd
from typing import Mapping

from create_shape_v1 import validate_rect_emu_v1, validate_uuid7_node_id_v1
from editor_project_asset_registry_v1 import (
    PROJECT_SCHEMA_V1,
    EditorProjectAssetRegistryError,
    derive_asset_metadata_v1,
    normalize_project_asset_registry_v1,
)


PLACEMENT_KIND_V1 = "chaptera.full-asset-equal-aspect.v1"
ZERO_CROP_V1 = {"left": 0, "top": 0, "right": 0, "bottom": 0}


class CreatePictureFrameError(ValueError):
    pass


def _fail(message: str) -> None:
    raise CreatePictureFrameError(message)


def _sha256(value: str) -> str:
    if (
        not isinstance(value, str)
        or len(value) != 64
        or any(ch not in "0123456789abcdef" for ch in value)
    ):
        _fail("CreatePictureFrame asset_sha256 must be lowercase SHA-256")
    return value


def _positive_pixel_dimension(value: int, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        _fail(f"{label} must be a positive integer")
    return value


def reduced_ratio_v1(width: int, height: int) -> tuple[int, int]:
    width = _positive_pixel_dimension(width, "ratio width")
    height = _positive_pixel_dimension(height, "ratio height")
    divisor = gcd(width, height)
    return width // divisor, height // divisor


def validate_equal_aspect_v1(
    frame: dict,
    *,
    intrinsic_width_px: int,
    intrinsic_height_px: int,
) -> None:
    validate_rect_emu_v1(frame, "CreatePictureFrame frame")
    frame_ratio = reduced_ratio_v1(frame["width"], frame["height"])
    asset_ratio = reduced_ratio_v1(intrinsic_width_px, intrinsic_height_px)
    if frame_ratio != asset_ratio:
        _fail("CreatePictureFrame frame ratio must exactly equal intrinsic asset ratio")


def validate_create_picture_frame_intent_v1(command: dict) -> None:
    allowed = {
        "kind",
        "node_id",
        "page_id",
        "frame",
        "asset_sha256",
        "intrinsic_width_px",
        "intrinsic_height_px",
    }
    if not isinstance(command, dict) or set(command) != allowed:
        _fail("CreatePictureFrame contains non-intent/authoritative fields")
    if command.get("kind") != "create_picture_frame":
        _fail("CreatePictureFrame kind is required")
    validate_uuid7_node_id_v1(command.get("node_id"))
    page_id = command.get("page_id")
    if not isinstance(page_id, str) or not page_id:
        _fail("CreatePictureFrame page_id is required")
    _sha256(command.get("asset_sha256"))
    width_px = _positive_pixel_dimension(
        command.get("intrinsic_width_px"),
        "CreatePictureFrame intrinsic_width_px",
    )
    height_px = _positive_pixel_dimension(
        command.get("intrinsic_height_px"),
        "CreatePictureFrame intrinsic_height_px",
    )
    validate_equal_aspect_v1(
        command.get("frame"),
        intrinsic_width_px=width_px,
        intrinsic_height_px=height_px,
    )


def _find_asset_metadata_row(project: dict, asset_sha256: str) -> dict:
    try:
        normalized = normalize_project_asset_registry_v1(project)
    except EditorProjectAssetRegistryError as exc:
        raise CreatePictureFrameError(str(exc)) from exc
    if normalized.get("schema_version") != PROJECT_SCHEMA_V1:
        _fail("CreatePictureFrame requires current EditorProject asset registry schema")
    rows = normalized.get("editor_assets")
    if not isinstance(rows, list):
        _fail("EditorProject editor_assets registry is required")
    matches = [
        row for row in rows
        if isinstance(row, dict) and row.get("asset_sha256") == asset_sha256
    ]
    if len(matches) != 1:
        _fail("CreatePictureFrame requires exactly one matching editor asset metadata row")
    return copy.deepcopy(matches[0])


def validate_create_picture_frame_asset_binding_v1(
    project: dict,
    command: dict,
    *,
    asset_bytes_by_sha: Mapping[str, bytes],
) -> dict:
    """Validate exact bytes + registry metadata + intrinsic facts before mutation."""
    validate_create_picture_frame_intent_v1(command)
    asset_sha256 = command["asset_sha256"]
    row = _find_asset_metadata_row(project, asset_sha256)
    payload = asset_bytes_by_sha.get(asset_sha256)
    if not isinstance(payload, bytes):
        _fail("CreatePictureFrame exact editor asset bytes are missing")
    mime_type = row.get("mime_type")
    try:
        derived = derive_asset_metadata_v1(
            asset_bytes=payload,
            mime_type=mime_type,
            include_intrinsic=True,
        )
    except EditorProjectAssetRegistryError as exc:
        raise CreatePictureFrameError(str(exc)) from exc
    if derived.public_dict() != row:
        _fail("CreatePictureFrame editor asset metadata does not match exact bytes")
    intrinsic = derived.intrinsic
    if intrinsic is None:
        _fail("CreatePictureFrame requires validated intrinsic metadata")
    if intrinsic.orientation_class != "normal":
        _fail("CreatePictureFrame requires normal image orientation")
    if (
        intrinsic.width_px != command["intrinsic_width_px"]
        or intrinsic.height_px != command["intrinsic_height_px"]
    ):
        _fail("CreatePictureFrame supplied intrinsic dimensions differ from exact asset bytes")
    validate_equal_aspect_v1(
        command["frame"],
        intrinsic_width_px=intrinsic.width_px,
        intrinsic_height_px=intrinsic.height_px,
    )
    return row


def _validate_page_and_identity(project: dict, command: dict) -> None:
    pages = project.get("pages")
    if not isinstance(pages, dict) or command["page_id"] not in pages:
        _fail("invalid_create_picture_frame_page")
    page = pages[command["page_id"]]
    if isinstance(page, dict) and page.get("authoring_enabled") is False:
        _fail("create_picture_frame_page_not_authorable")

    node_id = command["node_id"]
    for registry_name in (
        "shapes",
        "text_frames",
        "picture_frames",
        "groups",
        "nodes",
    ):
        registry = project.get(registry_name)
        if isinstance(registry, dict) and node_id in registry:
            _fail("create_picture_frame_node_id_collision")


def canonical_picture_frame_entity_v1(command: dict) -> dict:
    return {
        "node_id": command["node_id"],
        "kind": "image_frame",
        "page_id": command["page_id"],
        "parent_id": command["page_id"],
        "frame": copy.deepcopy(command["frame"]),
        "asset": command["asset_sha256"],
        "asset_sha256": command["asset_sha256"],
        "intrinsic": {
            "width_px": command["intrinsic_width_px"],
            "height_px": command["intrinsic_height_px"],
            "orientation_class": "normal",
        },
        "transform": {"kind": "identity"},
        "visible": True,
        "opacity_milli": 1000,
        "crop": copy.deepcopy(ZERO_CROP_V1),
        "placement": {"kind": PLACEMENT_KIND_V1},
        "supported": True,
        "provenance": {"kind": "author_created"},
    }


def canonical_create_picture_frame_operation_v1(command: dict) -> dict:
    entity = canonical_picture_frame_entity_v1(command)
    return {
        "kind": "create_picture_frame",
        "node_id": entity["node_id"],
        "page_id": entity["page_id"],
        "parent_id": entity["parent_id"],
        "frame": copy.deepcopy(entity["frame"]),
        "asset_sha256": entity["asset_sha256"],
        "intrinsic": copy.deepcopy(entity["intrinsic"]),
        "transform": copy.deepcopy(entity["transform"]),
        "visible": entity["visible"],
        "opacity_milli": entity["opacity_milli"],
        "crop": copy.deepcopy(entity["crop"]),
        "placement": copy.deepcopy(entity["placement"]),
        "provenance": copy.deepcopy(entity["provenance"]),
    }


def make_create_picture_frame_executor_v1(
    asset_bytes_by_sha: Mapping[str, bytes],
):
    """Bind exact asset bytes to an authoritative RevisionKernel executor."""

    def execute(base_project: dict, command: dict):
        # All fallible authority checks happen against the base before deepcopy/mutation.
        validate_create_picture_frame_intent_v1(command)
        validate_create_picture_frame_asset_binding_v1(
            base_project,
            command,
            asset_bytes_by_sha=asset_bytes_by_sha,
        )
        _validate_page_and_identity(base_project, command)

        project = copy.deepcopy(base_project)
        picture_frames = project.setdefault("picture_frames", {})
        if not isinstance(picture_frames, dict):
            _fail("canonical picture_frames registry must be object")

        entity = canonical_picture_frame_entity_v1(command)
        picture_frames[command["node_id"]] = copy.deepcopy(entity)
        operation = canonical_create_picture_frame_operation_v1(command)

        operations = project.get("operations")
        if not isinstance(operations, list):
            _fail("canonical project operations must be list")
        operations.append(copy.deepcopy(operation))

        consequences = [
            {"key": "picture_frame.created", "state": "supported", "note": None},
            {"key": "layout.scene", "state": "invalidated", "note": None},
            {"key": "editable_export", "state": "invalidated", "note": None},
        ]
        return operation, project, consequences

    return execute


def generic_free_resize_admitted_v1(picture_frame: dict) -> bool:
    """V1 product capability fence.

    A separately admitted aspect-preserving picture-resize command may exist,
    but generic free ResizeNode is never admitted for this created-picture class.
    """
    return not (
        isinstance(picture_frame, dict)
        and picture_frame.get("kind") == "image_frame"
        and picture_frame.get("placement") == {"kind": PLACEMENT_KIND_V1}
        and picture_frame.get("provenance") == {"kind": "author_created"}
    )


def move_node_admitted_v1(picture_frame: dict) -> bool:
    return bool(
        isinstance(picture_frame, dict)
        and picture_frame.get("kind") == "image_frame"
        and picture_frame.get("provenance") == {"kind": "author_created"}
    )
