#!/usr/bin/env python3
"""Chaptera-authored Group hierarchy validator V1.

This validator owns product-only invariants for Chaptera-created Group trees.
Universal graph correctness (dangling refs, generic cycles, parent/child
consistency) stays owned by the core semantic graph validator.
"""

from __future__ import annotations

MAX_AUTHORED_GROUP_DEPTH_V1 = 8


class AuthoredGroupHierarchyError(ValueError):
    pass


def _positive_rect(value, label):
    if not isinstance(value, dict) or set(value) != {"x", "y", "width", "height"}:
        raise AuthoredGroupHierarchyError(f"{label} must be an exact rect")
    for key in ("x", "y", "width", "height"):
        item = value[key]
        if not isinstance(item, int) or isinstance(item, bool):
            raise AuthoredGroupHierarchyError(f"{label}.{key} must be integer")
    if value["width"] <= 0 or value["height"] <= 0:
        raise AuthoredGroupHierarchyError(f"{label} must have positive width/height")
    return value


def _contained(child, parent, label):
    if child["x"] < parent["x"] or child["y"] < parent["y"]:
        raise AuthoredGroupHierarchyError(f"{label} starts outside parent local space")
    if child["x"] + child["width"] > parent["x"] + parent["width"]:
        raise AuthoredGroupHierarchyError(f"{label} exceeds parent local width")
    if child["y"] + child["height"] > parent["y"] + parent["height"]:
        raise AuthoredGroupHierarchyError(f"{label} exceeds parent local height")


def validate_authored_group_hierarchy_v1(project):
    """Validate only Chaptera-authored Group product invariants.

    Expected bounded project shape:
      project["pages"][page_id]["children"] -> top-level direct page members
      project["authored_stack"][page_id] -> top-level authored lane
      project["nodes"][node_id] -> authored rectangle/group nodes

    Group nodes carry:
      kind="group", author_created=True, parent_id, bounds,
      local_coordinate_space, children=[...]
    Nested child nodes use bounds expressed in the immediate parent local space.
    """
    if not isinstance(project, dict):
        raise AuthoredGroupHierarchyError("project must be object")
    nodes = project.get("nodes")
    pages = project.get("pages")
    stacks = project.get("authored_stack")
    if not isinstance(nodes, dict) or not isinstance(pages, dict) or not isinstance(stacks, dict):
        raise AuthoredGroupHierarchyError("project nodes/pages/authored_stack are required")

    authored = {
        node_id: node
        for node_id, node in nodes.items()
        if isinstance(node, dict) and node.get("author_created") is True
    }

    # Top-level authored lane contains direct page-owned authored nodes only.
    stack_members = set()
    for page_id, lane in stacks.items():
        if page_id not in pages:
            raise AuthoredGroupHierarchyError(f"authored_stack[{page_id}] has no page")
        if not isinstance(lane, list) or len(set(lane)) != len(lane):
            raise AuthoredGroupHierarchyError(f"authored_stack[{page_id}] must be unique ordered NodeIds")
        for node_id in lane:
            node = authored.get(node_id)
            if node is None:
                raise AuthoredGroupHierarchyError(f"authored_stack member {node_id} is not authored")
            if node.get("parent_id") != page_id:
                raise AuthoredGroupHierarchyError(f"nested authored node {node_id} cannot be top-level stack member")
            if node_id in stack_members:
                raise AuthoredGroupHierarchyError(f"authored node {node_id} appears in multiple lanes")
            stack_members.add(node_id)

    # Every top-level authored node must appear exactly once in AuthoredStackV1.
    for node_id, node in authored.items():
        parent_id = node.get("parent_id")
        if parent_id in pages and node_id not in stack_members:
            raise AuthoredGroupHierarchyError(f"top-level authored node {node_id} missing from authored stack")
        if parent_id in authored and node_id in stack_members:
            raise AuthoredGroupHierarchyError(f"nested authored node {node_id} duplicated in authored stack")

    # Product invariants over authored groups only.
    visiting = set()
    visited = set()

    def walk(node_id, depth, path):
        if depth > MAX_AUTHORED_GROUP_DEPTH_V1:
            raise AuthoredGroupHierarchyError(
                f"authored group depth overflow at {'/'.join(path + [node_id])}"
            )
        if node_id in visiting:
            # Defensive fail-closed guard. Generic cycle detection remains core-owned.
            raise AuthoredGroupHierarchyError(f"authored group cycle at {'/'.join(path + [node_id])}")
        if node_id in visited:
            return

        node = authored.get(node_id)
        if node is None or node.get("kind") != "group":
            return

        visiting.add(node_id)
        _positive_rect(node.get("bounds"), f"{node_id}.bounds")
        local = _positive_rect(node.get("local_coordinate_space"), f"{node_id}.local_coordinate_space")
        if local["x"] != 0 or local["y"] != 0:
            raise AuthoredGroupHierarchyError(f"{node_id}.local_coordinate_space origin must be zero")

        children = node.get("children")
        if not isinstance(children, list) or not children:
            raise AuthoredGroupHierarchyError(f"{node_id}.children must be non-empty")
        if len(set(children)) != len(children):
            raise AuthoredGroupHierarchyError(f"{node_id}.children contains duplicates")

        for child_id in children:
            child = authored.get(child_id)
            if child is None:
                raise AuthoredGroupHierarchyError(f"{node_id} child {child_id} is not supported authored node")
            if child.get("parent_id") != node_id:
                raise AuthoredGroupHierarchyError(f"{child_id}.parent_id does not point back to {node_id}")
            child_bounds = _positive_rect(child.get("bounds"), f"{child_id}.bounds")
            _contained(child_bounds, local, child_id)
            if child.get("kind") == "group":
                walk(child_id, depth + 1, path + [node_id])

        visiting.remove(node_id)
        visited.add(node_id)

    for node_id, node in authored.items():
        if node.get("kind") == "group" and node.get("parent_id") in pages:
            walk(node_id, 1, [])

    return True
