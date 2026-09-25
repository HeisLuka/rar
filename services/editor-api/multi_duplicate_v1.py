#!/usr/bin/env python3
"""Multi-selection Duplicate composed from FragmentSet + single Duplicate placement law."""

from __future__ import annotations

import copy

from authoring_fragment_set_v1 import (
    AuthoringFragmentSetError,
    apply_paste_fragment_set_v1,
    capture_rectangle_fragment_set_v1,
    canonical_paste_fragment_set_operation_v1,
)
from create_shape_v1 import CreateShapeError, validate_uuid7_node_id_v1
from duplicate_rectangle_v1 import (
    DUPLICATE_PLACEMENT_POLICY_V1,
    duplicate_placement_v1,
)


class MultiDuplicateV1Error(ValueError):
    pass


def validate_multi_duplicate_request_v1(request: dict) -> None:
    if request.get("protocol_version") != "chaptera.multi-duplicate-intent.v1":
        raise MultiDuplicateV1Error("MultiDuplicateV1 protocol_version is required")
    command = request.get("command")
    if (
        not isinstance(command, dict)
        or set(command) != {
            "kind",
            "source_node_ids",
            "primary_source_node_id",
            "identity_map",
            "placement_policy",
        }
        or command.get("kind") != "duplicate_selection_set"
    ):
        raise MultiDuplicateV1Error("MultiDuplicateV1 command is malformed")

    source_ids = command.get("source_node_ids")
    if (
        not isinstance(source_ids, list)
        or len(source_ids) < 2
        or any(not isinstance(node_id, str) or not node_id for node_id in source_ids)
    ):
        raise MultiDuplicateV1Error("MultiDuplicateV1 requires at least two source NodeIds")
    normalized = sorted(source_ids)
    if source_ids != normalized or len(set(source_ids)) != len(source_ids):
        raise MultiDuplicateV1Error(
            "MultiDuplicateV1 source_node_ids must be uniquely normalized by NodeId"
        )

    primary = command.get("primary_source_node_id")
    if not isinstance(primary, str) or primary not in source_ids:
        raise MultiDuplicateV1Error("MultiDuplicateV1 primary must be one selected source NodeId")

    identity_map = command.get("identity_map")
    if not isinstance(identity_map, list) or len(identity_map) != len(source_ids):
        raise MultiDuplicateV1Error("MultiDuplicateV1 identity_map must be complete")

    destinations = set()
    for index, entry in enumerate(identity_map):
        if (
            not isinstance(entry, dict)
            or set(entry) != {"source_node_id", "destination_node_id"}
            or entry.get("source_node_id") != source_ids[index]
        ):
            raise MultiDuplicateV1Error(
                "MultiDuplicateV1 identity_map must match normalized source order"
            )
        destination = entry.get("destination_node_id")
        try:
            validate_uuid7_node_id_v1(destination)
        except CreateShapeError as exc:
            raise MultiDuplicateV1Error(
                "MultiDuplicateV1 destination_node_id must be canonical UUIDv7"
            ) from exc
        if destination in destinations:
            raise MultiDuplicateV1Error("MultiDuplicateV1 destination identities must be unique")
        if destination in source_ids:
            raise MultiDuplicateV1Error("MultiDuplicateV1 cannot reuse source identity")
        destinations.add(destination)

    duplicate_placement_v1(command.get("placement_policy"))


def _paste_command_from_multi_duplicate(base_project: dict, command: dict) -> dict:
    try:
        fragment_set = capture_rectangle_fragment_set_v1(
            base_project,
            command["source_node_ids"],
        )
    except AuthoringFragmentSetError as exc:
        raise MultiDuplicateV1Error(str(exc)) from exc

    source_to_member = {}
    for member in fragment_set["members"]:
        source_id = member["fragment"]["rectangle"]["source_provenance"]["source_node_id"]
        source_to_member[source_id] = member["member_id"]

    first_shape = base_project["shapes"][command["source_node_ids"][0]]
    return {
        "kind": "paste_fragment_set",
        "fragment_set": fragment_set,
        "identity_map": [
            {
                "member_id": source_to_member[entry["source_node_id"]],
                "destination_node_id": entry["destination_node_id"],
            }
            for entry in command["identity_map"]
        ],
        "destination": {"kind": "page", "page_id": first_shape["page_id"]},
        "placement": duplicate_placement_v1(command["placement_policy"]),
    }


def execute_multi_duplicate_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list]:
    validate_multi_duplicate_request_v1(
        {
            "protocol_version": "chaptera.multi-duplicate-intent.v1",
            "command": copy.deepcopy(command),
        }
    )
    paste_command = _paste_command_from_multi_duplicate(base_project, command)
    try:
        operation, project, consequences = apply_paste_fragment_set_v1(
            base_project,
            paste_command,
        )
    except AuthoringFragmentSetError as exc:
        raise MultiDuplicateV1Error(str(exc)) from exc

    clone_by_source = {
        entry["source_node_id"]: entry["destination_node_id"]
        for entry in command["identity_map"]
    }
    selection = {
        "key": "selection.object_set",
        "state": "supported",
        "note": {
            "selected_node_ids": sorted(clone_by_source.values()),
            "primary_node_id": clone_by_source[command["primary_source_node_id"]],
        },
    }
    return operation, project, list(consequences) + [selection]


def validate_multi_duplicate_operation_v1(command: dict, operation: dict) -> None:
    if not isinstance(operation, dict) or operation.get("kind") != "paste_fragment_set":
        raise MultiDuplicateV1Error(
            "MultiDuplicateV1 must persist as canonical PasteFragmentSet"
        )
    expected_sources = command["source_node_ids"]
    members = operation.get("fragment_set", {}).get("members")
    if not isinstance(members, list):
        raise MultiDuplicateV1Error("MultiDuplicateV1 canonical FragmentSet is missing")
    actual_sources = [
        member["fragment"]["rectangle"]["source_provenance"]["source_node_id"]
        for member in members
    ]
    if actual_sources != expected_sources:
        raise MultiDuplicateV1Error(
            "MultiDuplicateV1 captured a different normalized source set"
        )

    member_by_source = {
        source: member["member_id"]
        for source, member in zip(actual_sources, members, strict=True)
    }
    expected_map = [
        {
            "member_id": member_by_source[entry["source_node_id"]],
            "destination_node_id": entry["destination_node_id"],
        }
        for entry in command["identity_map"]
    ]
    if operation.get("identity_map") != expected_map:
        raise MultiDuplicateV1Error("MultiDuplicateV1 persisted identity map differs")
    if operation.get("placement") != duplicate_placement_v1(command["placement_policy"]):
        raise MultiDuplicateV1Error("MultiDuplicateV1 placement differs from DuplicatePlacementV1")

    expected = canonical_paste_fragment_set_operation_v1(
        {
            "kind": "paste_fragment_set",
            "fragment_set": copy.deepcopy(operation["fragment_set"]),
            "identity_map": copy.deepcopy(operation["identity_map"]),
            "destination": copy.deepcopy(operation["destination"]),
            "placement": copy.deepcopy(operation["placement"]),
        }
    )
    if operation != expected:
        raise MultiDuplicateV1Error(
            "MultiDuplicateV1 produced non-canonical PasteFragmentSet"
        )


def default_multi_duplicate_policy_v1() -> str:
    return DUPLICATE_PLACEMENT_POLICY_V1
