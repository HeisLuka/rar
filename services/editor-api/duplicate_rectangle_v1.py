#!/usr/bin/env python3
"""Single authored Rectangle Duplicate composed from CaptureFragment -> PasteFragment."""

from __future__ import annotations

import copy

from authoring_fragment_v1 import (
    AuthoringFragmentError,
    apply_paste_fragment_v1,
    canonical_paste_fragment_operation_v1,
    capture_rectangle_fragment_v1,
)
from create_shape_v1 import CreateShapeError, validate_uuid7_node_id_v1


DUPLICATE_PLACEMENT_POLICY_V1 = "chaptera.duplicate-placement.10pt-down-right.v1"
DUPLICATE_OFFSET_EMU_V1 = 127_000  # Product-defined Chaptera V1 policy: 10 pt.


class DuplicateRectangleV1Error(ValueError):
    pass


def duplicate_placement_v1(policy: str) -> dict:
    if policy != DUPLICATE_PLACEMENT_POLICY_V1:
        raise DuplicateRectangleV1Error("unsupported DuplicatePlacementV1 policy")
    return {
        "kind": "translate",
        "dx_emu": DUPLICATE_OFFSET_EMU_V1,
        "dy_emu": DUPLICATE_OFFSET_EMU_V1,
    }


def validate_duplicate_rectangle_request_v1(request: dict) -> None:
    if request.get("protocol_version") != "chaptera.duplicate-rectangle-intent.v1":
        raise DuplicateRectangleV1Error("DuplicateRectangleV1 protocol_version is required")
    command = request.get("command")
    if (
        not isinstance(command, dict)
        or set(command) != {
            "kind",
            "source_node_id",
            "destination_node_id",
            "placement_policy",
        }
        or command.get("kind") != "duplicate_rectangle"
    ):
        raise DuplicateRectangleV1Error("DuplicateRectangleV1 command is malformed")
    source = command.get("source_node_id")
    destination = command.get("destination_node_id")
    if not isinstance(source, str) or not source:
        raise DuplicateRectangleV1Error("DuplicateRectangleV1 source_node_id is required")
    try:
        validate_uuid7_node_id_v1(destination)
    except CreateShapeError as exc:
        raise DuplicateRectangleV1Error(
            "DuplicateRectangleV1 destination_node_id must be canonical UUIDv7"
        ) from exc
    if source == destination:
        raise DuplicateRectangleV1Error("DuplicateRectangleV1 must allocate a new NodeId")
    duplicate_placement_v1(command.get("placement_policy"))


def _paste_command_from_duplicate(base_project: dict, command: dict) -> dict:
    try:
        fragment = capture_rectangle_fragment_v1(
            base_project,
            command["source_node_id"],
        )
    except AuthoringFragmentError as exc:
        raise DuplicateRectangleV1Error(str(exc)) from exc
    source_shape = base_project["shapes"][command["source_node_id"]]
    return {
        "kind": "paste_fragment",
        "fragment": fragment,
        "identity_map": {
            "fragment_entity_id": fragment["rectangle"]["fragment_entity_id"],
            "destination_node_id": command["destination_node_id"],
        },
        "destination": {
            "kind": "page",
            "page_id": source_shape["page_id"],
        },
        "placement": duplicate_placement_v1(command["placement_policy"]),
    }


def execute_duplicate_rectangle_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list]:
    validate_duplicate_rectangle_request_v1(
        {
            "protocol_version": "chaptera.duplicate-rectangle-intent.v1",
            "command": copy.deepcopy(command),
        }
    )
    paste_command = _paste_command_from_duplicate(base_project, command)
    try:
        operation, project, consequences = apply_paste_fragment_v1(
            base_project,
            paste_command,
        )
    except AuthoringFragmentError as exc:
        raise DuplicateRectangleV1Error(str(exc)) from exc

    selection = {
        "key": "selection.object",
        "state": "supported",
        "note": command["destination_node_id"],
    }
    return operation, project, list(consequences) + [selection]


def validate_duplicate_rectangle_operation_v1(command: dict, operation: dict) -> None:
    if not isinstance(operation, dict) or operation.get("kind") != "paste_fragment":
        raise DuplicateRectangleV1Error(
            "DuplicateRectangleV1 must persist as canonical PasteFragment"
        )
    identity = operation.get("identity_map")
    if (
        not isinstance(identity, dict)
        or identity.get("destination_node_id") != command.get("destination_node_id")
    ):
        raise DuplicateRectangleV1Error("DuplicateRectangleV1 persisted identity differs")
    fragment = operation.get("fragment")
    source = (
        fragment.get("rectangle", {})
        .get("source_provenance", {})
        .get("source_node_id")
        if isinstance(fragment, dict)
        else None
    )
    if source != command.get("source_node_id"):
        raise DuplicateRectangleV1Error("DuplicateRectangleV1 captured a different source")
    if operation.get("placement") != duplicate_placement_v1(command.get("placement_policy")):
        raise DuplicateRectangleV1Error("DuplicateRectangleV1 placement differs from V1 policy")
    expected = canonical_paste_fragment_operation_v1(
        {
            "kind": "paste_fragment",
            "fragment": copy.deepcopy(operation["fragment"]),
            "identity_map": copy.deepcopy(operation["identity_map"]),
            "destination": copy.deepcopy(operation["destination"]),
            "placement": copy.deepcopy(operation["placement"]),
        }
    )
    if operation != expected:
        raise DuplicateRectangleV1Error("DuplicateRectangleV1 produced non-canonical PasteFragment")


def paste_command_from_duplicate_operation_v1(operation: dict) -> dict:
    """Strip derived created_entity for deterministic redo/replay through PasteFragment."""
    if not isinstance(operation, dict) or operation.get("kind") != "paste_fragment":
        raise DuplicateRectangleV1Error("duplicate replay requires canonical PasteFragment")
    return {
        "kind": "paste_fragment",
        "fragment": copy.deepcopy(operation["fragment"]),
        "identity_map": copy.deepcopy(operation["identity_map"]),
        "destination": copy.deepcopy(operation["destination"]),
        "placement": copy.deepcopy(operation["placement"]),
    }
