#!/usr/bin/env python3
"""Source-neutral PubResolvedGraph -> bounded geometry Scene bridge.

This is a direct Rar factorization of the historical pub-layout
BoundedAuthoringSlice -> project_bounded -> resolve_bounded_geometry law used by
pub-viewer. It deliberately reads only the semantic fields required by that
law: page identity/size, node identity/parent/bounds/transform and Story ids.

It does not read Story text, SourceRef, byte ranges, Quill, Contents, Escher,
paint, resources or raw PUB bytes.

The context parameter is an explicit extension seam for immutable resolved
projection context (for example future master/Cmo relations). V1 graph-only
projection is exactly the empty-context path; non-empty context fails closed
until semantics are implemented.
"""

from __future__ import annotations

import copy
import hashlib
import json
import re
from typing import Any

UUID_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")

DEFAULT_ENVIRONMENT = {
    "engine_revision": "viewer-geometry-v0.1",
    "font_set_fingerprint": "fonts:not-consumed:geometry-only",
    "resource_fingerprint": "resources:not-consumed:geometry-only",
}


class ResolvedGraphSceneError(ValueError):
    pass


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def hash_id(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical_json(value)).hexdigest()


def require_uuid(value: Any, label: str) -> str:
    if not isinstance(value, str) or not UUID_RE.fullmatch(value):
        raise ResolvedGraphSceneError(f"{label} must be canonical lowercase UUID")
    return value


def require_int(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ResolvedGraphSceneError(f"{label} must be an integer")
    return value


def require_rect(value: Any, label: str) -> dict[str, int]:
    if not isinstance(value, dict) or set(value) != {"x", "y", "width", "height"}:
        raise ResolvedGraphSceneError(f"{label} must be exact RectEmu")
    rect = {
        "x": require_int(value["x"], f"{label}.x"),
        "y": require_int(value["y"], f"{label}.y"),
        "width": require_int(value["width"], f"{label}.width"),
        "height": require_int(value["height"], f"{label}.height"),
    }
    if rect["width"] <= 0 or rect["height"] <= 0:
        raise ResolvedGraphSceneError(f"{label} width/height must be positive")
    return rect


def require_size(value: Any, label: str) -> dict[str, int]:
    if not isinstance(value, dict) or set(value) != {"width", "height"}:
        raise ResolvedGraphSceneError(f"{label} must be exact Size2D")
    size = {
        "width": require_int(value["width"], f"{label}.width"),
        "height": require_int(value["height"], f"{label}.height"),
    }
    if size["width"] <= 0 or size["height"] <= 0:
        raise ResolvedGraphSceneError(f"{label} width/height must be positive")
    return size


def require_transform(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {"a", "b", "c", "d", "tx", "ty"}:
        raise ResolvedGraphSceneError(f"{label} must be exact Affine2D")
    out: dict[str, Any] = {}
    for key in ("a", "b", "c", "d"):
        coefficient = value[key]
        if not isinstance(coefficient, str) or not coefficient:
            raise ResolvedGraphSceneError(f"{label}.{key} must be exact decimal string")
        out[key] = coefficient
    out["tx"] = require_int(value["tx"], f"{label}.tx")
    out["ty"] = require_int(value["ty"], f"{label}.ty")
    return out


PROJECTION_CONTEXT_SCHEMA_V1 = "chaptera.pub-projection-context.v1"


def _projection_context(context: Any) -> dict[str, Any]:
    if context is None:
        return {
            "schema_version": PROJECTION_CONTEXT_SCHEMA_V1,
            "master_relations": [],
            "cmo_relations": [],
        }
    expected = {"schema_version", "master_relations", "cmo_relations"}
    if not isinstance(context, dict) or set(context) != expected:
        raise ResolvedGraphSceneError("projection context fields mismatch")
    if context["schema_version"] != PROJECTION_CONTEXT_SCHEMA_V1:
        raise ResolvedGraphSceneError("projection context schema_version mismatch")
    for key in ("master_relations", "cmo_relations"):
        if not isinstance(context[key], list):
            raise ResolvedGraphSceneError(f"projection context {key} must be an array")
    if context["cmo_relations"]:
        raise ResolvedGraphSceneError(
            "non-empty cmo_relations is outside resolved-geometry-v1 semantics"
        )

    seen_sources: set[str] = set()
    for index, relation in enumerate(context["master_relations"]):
        if not isinstance(relation, dict) or set(relation) != {
            "source_page_id",
            "source_page_seq_num",
            "master_page_id",
            "master_page_seq_num",
        }:
            raise ResolvedGraphSceneError(
                f"master_relations[{index}] fields mismatch"
            )
        source_page_id = require_uuid(
            relation["source_page_id"],
            f"master_relations[{index}].source_page_id",
        )
        master_page_id = require_uuid(
            relation["master_page_id"],
            f"master_relations[{index}].master_page_id",
        )
        require_int(
            relation["source_page_seq_num"],
            f"master_relations[{index}].source_page_seq_num",
        )
        require_int(
            relation["master_page_seq_num"],
            f"master_relations[{index}].master_page_seq_num",
        )
        if source_page_id == master_page_id:
            raise ResolvedGraphSceneError("master relation cannot self-reference")
        if source_page_id in seen_sources:
            raise ResolvedGraphSceneError(
                f"duplicate master relation for source page {source_page_id}"
            )
        seen_sources.add(source_page_id)
    return copy.deepcopy(context)


def source_hash_from_graph(graph: dict[str, Any]) -> str:
    document = graph.get("document")
    if not isinstance(document, dict):
        raise ResolvedGraphSceneError("resolved graph document is required")
    value = document.get("source_hash")
    if not isinstance(value, str) or not SHA_RE.fullmatch(value):
        raise ResolvedGraphSceneError("resolved graph document.source_hash must be SHA-256")
    return value


def project_resolved_graph_scene(
    graph: dict[str, Any],
    *,
    context: dict[str, Any] | None = None,
    environment: dict[str, str] | None = None,
) -> dict[str, Any]:
    if not isinstance(graph, dict):
        raise ResolvedGraphSceneError("resolved graph must be an object")
    projection_context = _projection_context(context)

    document = graph.get("document")
    pages_in = graph.get("pages")
    nodes_in = graph.get("nodes")
    stories_in = graph.get("stories")
    if not isinstance(document, dict):
        raise ResolvedGraphSceneError("resolved graph document is required")
    if not isinstance(pages_in, dict):
        raise ResolvedGraphSceneError("resolved graph pages must be an object")
    if not isinstance(nodes_in, dict):
        raise ResolvedGraphSceneError("resolved graph nodes must be an object")
    if not isinstance(stories_in, dict):
        raise ResolvedGraphSceneError("resolved graph stories must be an object")

    page_order = document.get("pages")
    if not isinstance(page_order, list):
        raise ResolvedGraphSceneError("resolved graph document.pages must be an array")

    master_relations = projection_context["master_relations"]
    master_page_ids = {relation["master_page_id"] for relation in master_relations}
    source_master = {
        relation["source_page_id"]: relation["master_page_id"]
        for relation in master_relations
    }
    for source_page_id, master_page_id in source_master.items():
        if source_page_id not in pages_in:
            raise ResolvedGraphSceneError(
                f"master relation source page {source_page_id} missing from graph"
            )
        if master_page_id not in pages_in:
            raise ResolvedGraphSceneError(
                f"master relation target page {master_page_id} missing from graph"
            )

    pages: list[dict[str, Any]] = []
    referenced_pages = set()
    for index, page_id_raw in enumerate(page_order):
        page_id = require_uuid(page_id_raw, f"document.pages[{index}]")
        if page_id in referenced_pages:
            raise ResolvedGraphSceneError(f"duplicate document page {page_id}")
        referenced_pages.add(page_id)
        page = pages_in.get(page_id)
        if not isinstance(page, dict):
            raise ResolvedGraphSceneError(f"missing page {page_id}")
        if require_uuid(page.get("id"), f"pages[{page_id}].id") != page_id:
            raise ResolvedGraphSceneError(f"page key/id mismatch for {page_id}")
        if page_id not in master_page_ids:
            pages.append({
                "origin": page_id,
                "size": require_size(page.get("size"), f"pages[{page_id}].size"),
                "bleed": copy.deepcopy(page.get("bleed")),
                "margins": copy.deepcopy(page.get("margins")),
            })

    # pub-layout normalizes input order by canonical identity.
    pages.sort(key=lambda item: item["origin"])

    nodes: list[dict[str, Any]] = []
    source_nodes: dict[str, dict[str, Any]] = {}
    for node_key, node in nodes_in.items():
        require_uuid(node_key, "nodes key")
        if not isinstance(node, dict):
            raise ResolvedGraphSceneError(f"node {node_key} must be an object")
        header = node.get("header")
        if not isinstance(header, dict):
            raise ResolvedGraphSceneError(f"node {node_key}.header is required")
        node_id = require_uuid(header.get("id"), f"nodes[{node_key}].header.id")
        if node_id != node_key:
            raise ResolvedGraphSceneError(f"node key/id mismatch for {node_key}")
        parent = require_uuid(
            header.get("parent_id"),
            f"nodes[{node_key}].header.parent_id",
        )
        projected = {
            "origin": node_id,
            "parent_origin": parent,
            "bounds": require_rect(
                header.get("bounds"),
                f"nodes[{node_key}].header.bounds",
            ),
            "transform": require_transform(
                header.get("transform"),
                f"nodes[{node_key}].header.transform",
            ),
        }
        source_nodes[node_id] = projected
        if parent not in master_page_ids:
            nodes.append(projected)

    projected_master_instances: list[dict[str, Any]] = []
    for relation in master_relations:
        source_page_id = relation["source_page_id"]
        master_page_id = relation["master_page_id"]
        master_page = pages_in[master_page_id]
        children = master_page.get("children")
        if not isinstance(children, list):
            raise ResolvedGraphSceneError(
                f"master page {master_page_id}.children must be an array"
            )
        for child_index, child_raw in enumerate(children):
            child_id = require_uuid(
                child_raw,
                f"pages[{master_page_id}].children[{child_index}]",
            )
            source_node = source_nodes.get(child_id)
            if source_node is None:
                raise ResolvedGraphSceneError(
                    f"master child {child_id} missing from graph nodes"
                )
            if source_node["parent_origin"] != master_page_id:
                raise ResolvedGraphSceneError(
                    f"master child {child_id} parent does not match master page"
                )
            instance_id = hash_id({
                "projection_kind": "inherited_master",
                "origin": child_id,
                "target_page": source_page_id,
            })
            instance = {
                "origin": child_id,
                "parent_origin": source_page_id,
                "bounds": copy.deepcopy(source_node["bounds"]),
                "transform": copy.deepcopy(source_node["transform"]),
                "instance_id": instance_id,
                "projection_kind": "inherited_master",
                "source_parent_origin": master_page_id,
            }
            nodes.append(instance)
            projected_master_instances.append(instance)

    nodes.sort(
        key=lambda item: (
            item["origin"],
            item["parent_origin"],
            item.get("instance_id", ""),
        )
    )

    story_ids: list[str] = []
    for story_key, story in stories_in.items():
        require_uuid(story_key, "stories key")
        if not isinstance(story, dict):
            raise ResolvedGraphSceneError(f"story {story_key} must be an object")
        story_id = require_uuid(story.get("id"), f"stories[{story_key}].id")
        if story_id != story_key:
            raise ResolvedGraphSceneError(f"story key/id mismatch for {story_key}")
        story_ids.append(story_id)
    story_ids.sort()

    env = copy.deepcopy(environment or DEFAULT_ENVIRONMENT)
    if not isinstance(env, dict) or set(env) != {
        "engine_revision",
        "font_set_fingerprint",
        "resource_fingerprint",
    }:
        raise ResolvedGraphSceneError("layout environment fields mismatch")
    for key, value in env.items():
        if not isinstance(value, str) or not value:
            raise ResolvedGraphSceneError(f"layout environment {key} must be non-empty")

    origin_mapping = []
    for item in nodes:
        mapping = {
            "authoring_origin": item["origin"],
            "resolved_node_origin": item["origin"],
        }
        if item.get("projection_kind") == "inherited_master":
            mapping.update({
                "resolved_instance_id": item["instance_id"],
                "projection_kind": "inherited_master",
                "target_page_origin": item["parent_origin"],
                "source_parent_origin": item["source_parent_origin"],
            })
        origin_mapping.append(mapping)

    diagnostics = [
        {
            "code": "story_text_layout_not_implemented",
            "severity": "fidelity_warning",
            "origin": story_id,
            "message": "geometry-only resolver does not shape or flow story text",
        }
        for story_id in story_ids
    ]

    return {
        "environment": env,
        "surfaces": pages,
        "nodes": nodes,
        "origin_mapping": origin_mapping,
        "diagnostics": diagnostics,
    }


def apply_project_to_resolved_graph(
    baseline_graph: dict[str, Any],
    project: dict[str, Any],
) -> dict[str, Any]:
    if not isinstance(project, dict):
        raise ResolvedGraphSceneError("EditorProject must be an object")
    operations = project.get("operations")
    if not isinstance(operations, list):
        raise ResolvedGraphSceneError("EditorProject.operations must be an array")

    graph = copy.deepcopy(baseline_graph)
    nodes = graph.get("nodes")
    if not isinstance(nodes, dict):
        raise ResolvedGraphSceneError("resolved graph nodes must be an object")

    for index, operation in enumerate(operations):
        if not isinstance(operation, dict) or operation.get("kind") != "move_node":
            raise ResolvedGraphSceneError(
                f"operation[{index}] is outside bounded MoveNode projection"
            )
        if set(operation) != {"kind", "node_id", "before", "after"}:
            raise ResolvedGraphSceneError(f"operation[{index}] fields mismatch")
        node_id = require_uuid(operation["node_id"], f"operation[{index}].node_id")
        node = nodes.get(node_id)
        if not isinstance(node, dict) or not isinstance(node.get("header"), dict):
            raise ResolvedGraphSceneError(f"operation[{index}] references unknown node")
        current = require_rect(
            node["header"].get("bounds"),
            f"nodes[{node_id}].header.bounds",
        )
        before = require_rect(operation["before"], f"operation[{index}].before")
        after = require_rect(operation["after"], f"operation[{index}].after")
        if current != before:
            raise ResolvedGraphSceneError(
                f"operation[{index}] before-state does not match current resolved graph"
            )
        if (before["width"], before["height"]) != (after["width"], after["height"]):
            raise ResolvedGraphSceneError(
                f"operation[{index}] MoveNode must preserve width/height"
            )
        node["header"]["bounds"] = copy.deepcopy(after)

    return graph


def scene_geometry_hash(scene: dict[str, Any]) -> str:
    return hash_id({
        "surfaces": scene["surfaces"],
        "nodes": scene["nodes"],
    })


def scene_surface_hash(scene: dict[str, Any]) -> str:
    return hash_id(scene["surfaces"])


def scene_origin_mapping_hash(scene: dict[str, Any]) -> str:
    return hash_id(scene["origin_mapping"])


def scene_snapshot_id(scene: dict[str, Any]) -> str:
    return hash_id(scene)


def compare_viewer_and_adapter_scene(
    viewer_geometry: dict[str, Any],
    adapter_scene: dict[str, Any],
) -> dict[str, str]:
    if not isinstance(viewer_geometry, dict):
        raise ResolvedGraphSceneError("ViewerGeometryDocument must be an object")
    viewer_scene = viewer_geometry.get("scene")
    if not isinstance(viewer_scene, dict):
        raise ResolvedGraphSceneError("ViewerGeometryDocument.scene is required")

    # The reusable bridge is factored from the exact historical
    # BoundedAuthoringSlice -> project_bounded -> resolve_bounded_geometry law.
    # Baseline acceptance therefore requires exact source-neutral Scene equality,
    # not merely count/target-node parity.
    if viewer_scene != adapter_scene:
        raise ResolvedGraphSceneError(
            "real Viewer Scene differs from reusable resolved-graph adapter Scene"
        )

    return {
        "viewer_geometry_hash": scene_geometry_hash(viewer_scene),
        "adapter_geometry_hash": scene_geometry_hash(adapter_scene),
        "viewer_surface_hash": scene_surface_hash(viewer_scene),
        "adapter_surface_hash": scene_surface_hash(adapter_scene),
        "viewer_origin_mapping_hash": scene_origin_mapping_hash(viewer_scene),
        "adapter_origin_mapping_hash": scene_origin_mapping_hash(adapter_scene),
    }


def compact_scene_state(
    scene: dict[str, Any],
    *,
    node_id: str,
    page_id: str,
) -> dict[str, Any]:
    node_id = require_uuid(node_id, "target node_id")
    page_id = require_uuid(page_id, "target page_id")
    node = next((item for item in scene["nodes"] if item["origin"] == node_id), None)
    if node is None:
        raise ResolvedGraphSceneError(f"target node {node_id} missing from Scene")
    if node["parent_origin"] != page_id:
        raise ResolvedGraphSceneError(
            "bounded target node must remain directly page-owned"
        )
    return {
        "scene_snapshot_id": scene_snapshot_id(scene),
        "node_id": node_id,
        "page_id": page_id,
        "origin_node_id": node["origin"],
        "bounds": copy.deepcopy(node["bounds"]),
        "surface_page_ids_hash": hash_id(
            [surface["origin"] for surface in scene["surfaces"]]
        ),
        "origin_mapping_hash": scene_origin_mapping_hash(scene),
        "projection_input": "current_resolved_graph",
    }
