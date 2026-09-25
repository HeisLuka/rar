"""Source-neutral simultaneous DeleteNodesV1 semantics."""

from __future__ import annotations

import copy
import hashlib
import json
from typing import Any


class DeleteNodesV1Error(ValueError):
    pass


def hash_id(value: Any) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return "sha256:" + hashlib.sha256(payload.encode("utf-8")).hexdigest()


def _require_safe_leaf(node_id: str, node: dict, page_id: str) -> None:
    if node.get("parent_id") != page_id:
        raise DeleteNodesV1Error(f"target {node_id} is not direct page-owned")
    if node.get("author_created") is not True:
        raise DeleteNodesV1Error(f"target {node_id} is not canonically author-created")
    provenance = node.get("provenance")
    if provenance is not None and provenance != "chaptera-authored":
        raise DeleteNodesV1Error(f"target {node_id} has non-authored provenance")
    if node.get("node_class") != "ordinary_leaf":
        raise DeleteNodesV1Error(f"target {node_id} is not an admitted ordinary leaf")
    dependencies = node.get("dependencies")
    if not isinstance(dependencies, list) or dependencies:
        raise DeleteNodesV1Error(f"target {node_id} has dependent references")
    if node.get("story_id") is not None or node.get("text_frame_id") is not None:
        raise DeleteNodesV1Error(f"target {node_id} owns Story/TextFrame state")
    if node.get("group_id") is not None:
        raise DeleteNodesV1Error(f"target {node_id} is group-owned")


def execute_delete_nodes_v1(base_project: dict, command: dict):
    """Apply one atomic base-state DeleteNodesV1 operation.

    Command entries may arrive in any enumeration order. Canonical operation
    entries are normalized by NodeId, while deletion and inverse semantics are
    bound to the exact base page child sequence.
    """
    if not isinstance(base_project, dict):
        raise DeleteNodesV1Error("base project must be an object")
    if not isinstance(command, dict) or command.get("kind") != "delete_nodes":
        raise DeleteNodesV1Error("DeleteNodesV1 command required")

    page_id = command.get("page_id")
    entries = command.get("entries")
    if not isinstance(page_id, str) or not page_id:
        raise DeleteNodesV1Error("page_id required")
    if not isinstance(entries, list) or not entries:
        raise DeleteNodesV1Error("DeleteNodesV1 requires a non-empty target set")

    pages = base_project.get("pages")
    nodes = base_project.get("nodes")
    operations = base_project.get("operations")
    if not isinstance(pages, dict) or not isinstance(nodes, dict) or not isinstance(operations, list):
        raise DeleteNodesV1Error("base project lacks canonical pages/nodes/operations registries")
    page = pages.get(page_id)
    if not isinstance(page, dict) or not isinstance(page.get("children"), list):
        raise DeleteNodesV1Error("target page is unavailable")
    base_child_order = copy.deepcopy(page["children"])
    if any(not isinstance(child, str) or not child for child in base_child_order):
        raise DeleteNodesV1Error("base page child order is malformed")
    if len(set(base_child_order)) != len(base_child_order):
        raise DeleteNodesV1Error("base page child order contains duplicates")

    requested_ids = [entry.get("node_id") if isinstance(entry, dict) else None for entry in entries]
    if any(not isinstance(node_id, str) or not node_id for node_id in requested_ids):
        raise DeleteNodesV1Error("every DeleteNodesV1 target requires node_id")
    if len(set(requested_ids)) != len(requested_ids):
        raise DeleteNodesV1Error("DeleteNodesV1 targets must be unique")

    # Preflight every member against the same untouched base state.
    canonical_entries = []
    child_positions = {node_id: index for index, node_id in enumerate(base_child_order)}
    for requested in entries:
        node_id = requested["node_id"]
        node = nodes.get(node_id)
        if not isinstance(node, dict):
            raise DeleteNodesV1Error(f"target {node_id} is missing")
        _require_safe_leaf(node_id, node, page_id)

        actual_index = child_positions.get(node_id)
        if actual_index is None:
            raise DeleteNodesV1Error(f"target {node_id} is absent from base page child order")
        expected_index = requested.get("expected_child_index")
        if expected_index != actual_index:
            raise DeleteNodesV1Error(f"stale child index for {node_id}")

        before_entity = copy.deepcopy(node)
        before_state_id = hash_id(before_entity)
        if requested.get("expected_state_id") != before_state_id:
            raise DeleteNodesV1Error(f"stale entity state for {node_id}")

        canonical_entries.append(
            {
                "node_id": node_id,
                "before_entity": before_entity,
                "before_state_id": before_state_id,
                "parent_id": page_id,
                "child_index": actual_index,
            }
        )

    canonical_entries.sort(key=lambda entry: entry["node_id"])
    target_ids = {entry["node_id"] for entry in canonical_entries}

    operation = {
        "kind": "delete_nodes",
        "page_id": page_id,
        "entries": canonical_entries,
        "base_child_order": base_child_order,
    }

    project = copy.deepcopy(base_project)
    project["nodes"] = copy.deepcopy(nodes)
    for node_id in target_ids:
        del project["nodes"][node_id]

    project["pages"] = copy.deepcopy(pages)
    project["pages"][page_id]["children"] = [
        child for child in base_child_order if child not in target_ids
    ]
    project["operations"] = list(operations) + [copy.deepcopy(operation)]

    return operation, project, [
        {
            "key": "node.delete.batch",
            "state": "supported",
            "note": "simultaneous_base_state_delete",
        },
        {"key": "layout.scene", "state": "invalidated", "note": None},
    ]


def restore_delete_nodes_v1(deleted_project: dict, operation: dict) -> dict:
    """Exact inverse for one accepted DeleteNodesV1 operation."""
    if not isinstance(operation, dict) or operation.get("kind") != "delete_nodes":
        raise DeleteNodesV1Error("canonical DeleteNodesV1 operation required")
    page_id = operation.get("page_id")
    base_child_order = operation.get("base_child_order")
    entries = operation.get("entries")
    if not isinstance(page_id, str) or not isinstance(base_child_order, list) or not isinstance(entries, list):
        raise DeleteNodesV1Error("malformed DeleteNodesV1 inverse state")

    target_ids = {entry.get("node_id") for entry in entries if isinstance(entry, dict)}
    if len(target_ids) != len(entries) or None in target_ids:
        raise DeleteNodesV1Error("malformed DeleteNodesV1 inverse targets")

    current_page = deleted_project.get("pages", {}).get(page_id)
    current_nodes = deleted_project.get("nodes")
    current_operations = deleted_project.get("operations")
    if not isinstance(current_page, dict) or not isinstance(current_page.get("children"), list):
        raise DeleteNodesV1Error("deleted project page state unavailable")
    if not isinstance(current_nodes, dict) or not isinstance(current_operations, list):
        raise DeleteNodesV1Error("deleted project registries unavailable")

    expected_children = [child for child in base_child_order if child not in target_ids]
    if current_page["children"] != expected_children:
        raise DeleteNodesV1Error("deleted page child sequence is stale")
    if any(node_id in current_nodes for node_id in target_ids):
        raise DeleteNodesV1Error("deleted target unexpectedly present")

    restored = copy.deepcopy(deleted_project)
    restored["nodes"] = copy.deepcopy(current_nodes)
    for entry in entries:
        before_entity = entry.get("before_entity")
        if not isinstance(before_entity, dict):
            raise DeleteNodesV1Error("inverse before_entity missing")
        if hash_id(before_entity) != entry.get("before_state_id"):
            raise DeleteNodesV1Error("inverse before_entity hash mismatch")
        restored["nodes"][entry["node_id"]] = copy.deepcopy(before_entity)

    restored["pages"] = copy.deepcopy(deleted_project["pages"])
    restored["pages"][page_id]["children"] = copy.deepcopy(base_child_order)

    if not current_operations or current_operations[-1] != operation:
        raise DeleteNodesV1Error("DeleteNodesV1 operation is not the current tail")
    restored["operations"] = copy.deepcopy(current_operations[:-1])
    return restored
