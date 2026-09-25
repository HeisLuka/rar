#!/usr/bin/env python3
"""Atomic multi-rectangle AuthoringFragmentSet/PasteFragmentSet V1.

One set is one semantic edit and therefore one revision/Undo unit. Input
enumeration is normalized by source NodeId; it never carries z-order meaning.
"""

from __future__ import annotations

import copy

try:
    from authoring_fragment_v1 import (
        AuthoringFragmentError,
        capture_rectangle_fragment_v1,
        canonical_paste_fragment_operation_v1,
        validate_authoring_fragment_v1,
    )
    from create_shape_v1 import validate_uuid7_node_id_v1
except ModuleNotFoundError:
    import importlib.util
    import pathlib
    import sys

    _dir = pathlib.Path(__file__).resolve().parent

    def _load(name: str, filename: str):
        path = _dir / filename
        spec = importlib.util.spec_from_file_location(name, path)
        if spec is None or spec.loader is None:
            raise ImportError(f"cannot load {filename}")
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        added = str(_dir) not in sys.path
        if added:
            sys.path.insert(0, str(_dir))
        try:
            spec.loader.exec_module(module)
        finally:
            if added:
                sys.path.remove(str(_dir))
        return module

    _fragment = _load("chaptera_authoring_fragment_set_single", "authoring_fragment_v1.py")
    _shape = _load("chaptera_authoring_fragment_set_shape", "create_shape_v1.py")
    AuthoringFragmentError = _fragment.AuthoringFragmentError
    capture_rectangle_fragment_v1 = _fragment.capture_rectangle_fragment_v1
    canonical_paste_fragment_operation_v1 = _fragment.canonical_paste_fragment_operation_v1
    validate_authoring_fragment_v1 = _fragment.validate_authoring_fragment_v1
    validate_uuid7_node_id_v1 = _shape.validate_uuid7_node_id_v1


AUTHORING_FRAGMENT_SET_SCHEMA_V1 = "chaptera.authoring-fragment-set.v1"


class AuthoringFragmentSetError(ValueError):
    pass


def capture_rectangle_fragment_set_v1(project: dict, node_ids: list[str]) -> dict:
    if (
        not isinstance(node_ids, list)
        or len(node_ids) < 2
        or any(not isinstance(node_id, str) or not node_id for node_id in node_ids)
    ):
        raise AuthoringFragmentSetError("FragmentSet requires at least two explicit NodeIds")
    normalized = sorted(node_ids)
    if len(set(normalized)) != len(normalized):
        raise AuthoringFragmentSetError("FragmentSet source NodeIds must be unique")

    shapes = project.get("shapes") if isinstance(project, dict) else None
    if not isinstance(shapes, dict):
        raise AuthoringFragmentSetError("canonical shapes registry must be object")

    source_page_id = None
    members = []
    origin_x = None
    origin_y = None
    for index, node_id in enumerate(normalized):
        shape = shapes.get(node_id)
        if not isinstance(shape, dict):
            raise AuthoringFragmentSetError("FragmentSet source shape is missing")
        page_id = shape.get("page_id")
        if (
            not isinstance(page_id, str)
            or not page_id
            or shape.get("parent_id") != page_id
        ):
            raise AuthoringFragmentSetError("FragmentSet source must be direct page-owned")
        if source_page_id is None:
            source_page_id = page_id
        elif page_id != source_page_id:
            raise AuthoringFragmentSetError("FragmentSet members must share one source page")

        try:
            fragment = capture_rectangle_fragment_v1(project, node_id)
        except AuthoringFragmentError as exc:
            raise AuthoringFragmentSetError(str(exc)) from exc
        bounds = fragment["rectangle"]["bounds"]
        origin_x = bounds["x"] if origin_x is None else min(origin_x, bounds["x"])
        origin_y = bounds["y"] if origin_y is None else min(origin_y, bounds["y"])
        members.append(
            {
                "member_id": f"member:{index}",
                "fragment": fragment,
            }
        )

    result = {
        "schema_version": AUTHORING_FRAGMENT_SET_SCHEMA_V1,
        "source_page_id": source_page_id,
        "origin": {"x": origin_x, "y": origin_y},
        "members": members,
    }
    validate_authoring_fragment_set_v1(result)
    return result


def validate_authoring_fragment_set_v1(fragment_set: dict) -> None:
    if not isinstance(fragment_set, dict) or set(fragment_set) != {
        "schema_version",
        "source_page_id",
        "origin",
        "members",
    }:
        raise AuthoringFragmentSetError("AuthoringFragmentSetV1 fields are malformed")
    if fragment_set["schema_version"] != AUTHORING_FRAGMENT_SET_SCHEMA_V1:
        raise AuthoringFragmentSetError("unsupported AuthoringFragmentSetV1 schema")
    if not isinstance(fragment_set["source_page_id"], str) or not fragment_set["source_page_id"]:
        raise AuthoringFragmentSetError("FragmentSet source_page_id is required")

    origin = fragment_set["origin"]
    if (
        not isinstance(origin, dict)
        or set(origin) != {"x", "y"}
        or any(not isinstance(origin[key], int) or isinstance(origin[key], bool) for key in ("x", "y"))
    ):
        raise AuthoringFragmentSetError("FragmentSet origin is malformed")

    members = fragment_set["members"]
    if not isinstance(members, list) or len(members) < 2:
        raise AuthoringFragmentSetError("FragmentSet requires at least two members")

    source_ids = set()
    expected_x = None
    expected_y = None
    for index, member in enumerate(members):
        if (
            not isinstance(member, dict)
            or set(member) != {"member_id", "fragment"}
            or member.get("member_id") != f"member:{index}"
        ):
            raise AuthoringFragmentSetError("FragmentSet members are not normalized")
        try:
            validate_authoring_fragment_v1(member["fragment"])
        except AuthoringFragmentError as exc:
            raise AuthoringFragmentSetError(str(exc)) from exc
        provenance = member["fragment"]["rectangle"].get("source_provenance")
        if not isinstance(provenance, dict):
            raise AuthoringFragmentSetError("FragmentSet member source provenance is required")
        source_id = provenance.get("source_node_id")
        if source_id in source_ids:
            raise AuthoringFragmentSetError("FragmentSet source identities must be unique")
        source_ids.add(source_id)

        bounds = member["fragment"]["rectangle"]["bounds"]
        expected_x = bounds["x"] if expected_x is None else min(expected_x, bounds["x"])
        expected_y = bounds["y"] if expected_y is None else min(expected_y, bounds["y"])

    if origin != {"x": expected_x, "y": expected_y}:
        raise AuthoringFragmentSetError("FragmentSet origin does not match member bounds")


def validate_paste_fragment_set_intent_v1(command: dict) -> None:
    if not isinstance(command, dict) or set(command) != {
        "kind",
        "fragment_set",
        "identity_map",
        "destination",
        "placement",
    }:
        raise AuthoringFragmentSetError("PasteFragmentSet contains non-intent fields")
    if command.get("kind") != "paste_fragment_set":
        raise AuthoringFragmentSetError("PasteFragmentSet kind is required")
    validate_authoring_fragment_set_v1(command.get("fragment_set"))

    destination = command.get("destination")
    if (
        not isinstance(destination, dict)
        or set(destination) != {"kind", "page_id"}
        or destination.get("kind") != "page"
        or not isinstance(destination.get("page_id"), str)
        or not destination["page_id"]
    ):
        raise AuthoringFragmentSetError("PasteFragmentSet destination must be one page")

    placement = command.get("placement")
    if (
        not isinstance(placement, dict)
        or set(placement) != {"kind", "dx_emu", "dy_emu"}
        or placement.get("kind") != "translate"
    ):
        raise AuthoringFragmentSetError("PasteFragmentSet placement must be translation")
    for field in ("dx_emu", "dy_emu"):
        value = placement.get(field)
        if not isinstance(value, int) or isinstance(value, bool):
            raise AuthoringFragmentSetError("PasteFragmentSet translation must use integer EMU")

    members = command["fragment_set"]["members"]
    identity_map = command.get("identity_map")
    if not isinstance(identity_map, list) or len(identity_map) != len(members):
        raise AuthoringFragmentSetError("PasteFragmentSet identity_map must be complete")

    destination_ids = set()
    source_ids = {
        member["fragment"]["rectangle"]["source_provenance"]["source_node_id"]
        for member in members
    }
    for index, remap in enumerate(identity_map):
        if (
            not isinstance(remap, dict)
            or set(remap) != {"member_id", "destination_node_id"}
            or remap.get("member_id") != members[index]["member_id"]
        ):
            raise AuthoringFragmentSetError("PasteFragmentSet identity_map is not normalized")
        destination_node_id = remap.get("destination_node_id")
        try:
            validate_uuid7_node_id_v1(destination_node_id)
        except Exception as exc:
            raise AuthoringFragmentSetError("PasteFragmentSet destination identity must be UUIDv7") from exc
        if destination_node_id in destination_ids:
            raise AuthoringFragmentSetError("PasteFragmentSet destination identities must be unique")
        if destination_node_id in source_ids:
            raise AuthoringFragmentSetError("PasteFragmentSet cannot reuse source identity")
        destination_ids.add(destination_node_id)


def canonical_paste_fragment_set_operation_v1(command: dict) -> dict:
    validate_paste_fragment_set_intent_v1(command)
    created_entities = _materialized_entities_v1(command)
    return {
        "kind": "paste_fragment_set",
        "fragment_set": copy.deepcopy(command["fragment_set"]),
        "identity_map": copy.deepcopy(command["identity_map"]),
        "destination": copy.deepcopy(command["destination"]),
        "placement": copy.deepcopy(command["placement"]),
        "created_entities": created_entities,
    }


def apply_paste_fragment_set_v1(base_project: dict, command: dict) -> tuple[dict, dict, list]:
    validate_paste_fragment_set_intent_v1(command)

    pages = base_project.get("pages")
    page_id = command["destination"]["page_id"]
    if not isinstance(pages, dict) or page_id not in pages:
        raise AuthoringFragmentSetError("invalid_paste_fragment_set_page")
    page = pages[page_id]
    if isinstance(page, dict) and page.get("authoring_enabled") is False:
        raise AuthoringFragmentSetError("paste_fragment_set_page_not_authorable")

    # Complete preflight before copying/mutating any project state.
    created_entities = _materialized_entities_v1(command)
    destination_ids = [entity["node_id"] for entity in created_entities]
    for registry_name in ("shapes", "text_frames", "picture_frames", "groups", "nodes"):
        registry = base_project.get(registry_name)
        if isinstance(registry, dict) and any(node_id in registry for node_id in destination_ids):
            raise AuthoringFragmentSetError("paste_fragment_set_node_id_collision")

    shapes = base_project.get("shapes")
    operations = base_project.get("operations")
    if not isinstance(shapes, dict):
        raise AuthoringFragmentSetError("canonical shapes registry must be object")
    if not isinstance(operations, list):
        raise AuthoringFragmentSetError("canonical project operations must be list")

    operation = canonical_paste_fragment_set_operation_v1(command)
    project = copy.deepcopy(base_project)
    for entity in created_entities:
        project["shapes"][entity["node_id"]] = copy.deepcopy(entity)
    project["operations"].append(copy.deepcopy(operation))

    return operation, project, [
        {"key": "shape.created_set", "state": "supported", "note": str(len(created_entities))},
        {"key": "layout.scene", "state": "invalidated", "note": None},
        {"key": "editable_export", "state": "invalidated", "note": None},
    ]


def _materialized_entities_v1(command: dict) -> list[dict]:
    entities = []
    for member, remap in zip(
        command["fragment_set"]["members"],
        command["identity_map"],
        strict=True,
    ):
        single_command = {
            "kind": "paste_fragment",
            "fragment": copy.deepcopy(member["fragment"]),
            "identity_map": {
                "fragment_entity_id": member["fragment"]["rectangle"]["fragment_entity_id"],
                "destination_node_id": remap["destination_node_id"],
            },
            "destination": copy.deepcopy(command["destination"]),
            "placement": copy.deepcopy(command["placement"]),
        }
        try:
            operation = canonical_paste_fragment_operation_v1(single_command)
        except AuthoringFragmentError as exc:
            raise AuthoringFragmentSetError(str(exc)) from exc
        entities.append(operation["created_entity"])
    return entities
