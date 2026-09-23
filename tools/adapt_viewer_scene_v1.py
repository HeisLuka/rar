#!/usr/bin/env python3
import argparse
import json
import pathlib
import re
import sys

from scene_v1 import canonical_json, finalize_snapshot, hash_id, string_hash_id

UUID_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
HASH_ID_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
DECIMAL_RE = re.compile(r"^-?(0|[1-9][0-9]*)(\\.[0-9]*[1-9])?$")


def fail(message):
    raise ValueError(message)


def require_uuid(value, name):
    if not isinstance(value, str) or not UUID_RE.fullmatch(value):
        fail(f"{name} must be a canonical lowercase UUID")
    return value


def require_sha(value, name):
    if not isinstance(value, str) or not SHA_RE.fullmatch(value):
        fail(f"{name} must be a lowercase SHA-256 hex string")
    return value


def require_hash_id(value, name):
    if not isinstance(value, str) or not HASH_ID_RE.fullmatch(value):
        fail(f"{name} must be sha256:<lowercase hex>")
    return value


def require_int(value, name):
    if isinstance(value, bool) or not isinstance(value, int):
        fail(f"{name} must be an integer")
    return value


def rgb(value, name):
    if not isinstance(value, list) or len(value) != 3:
        fail(f"{name} must be [r,g,b]")
    out = []
    for index, component in enumerate(value):
        component = require_int(component, f"{name}[{index}]")
        if component < 0 or component > 255:
            fail(f"{name}[{index}] must be 0..255")
        out.append(component)
    return {"r": out[0], "g": out[1], "b": out[2], "a": 255}


def diagnostic(code, severity="warning", origin_node_id=None, message_key=None):
    return {
        "severity": severity,
        "code": code,
        "origin_node_id": origin_node_id,
        "message_key": message_key or code.lower(),
    }


def map_viewer_diagnostic(item):
    severity = item.get("severity")
    mapped = "info" if severity == "info" else "warning"
    code = item.get("code")
    if not isinstance(code, str) or not code:
        fail("Viewer diagnostic must carry a stable code")
    return diagnostic(
        code=code,
        severity=mapped,
        origin_node_id=None,
        message_key=code.lower(),
    )


def environment_from_viewer(scene):
    env = scene.get("environment")
    if not isinstance(env, dict):
        fail("scene.environment is required")
    engine = env.get("engine_revision")
    fonts = env.get("font_set_fingerprint")
    resources = env.get("resource_fingerprint")
    if not all(isinstance(value, str) and value for value in (engine, fonts, resources)):
        fail("scene.environment fields must be non-empty strings")
    identity_input = {
        "engine_revision": engine,
        "font_set_fingerprint": fonts,
        "resource_fingerprint": resources,
    }
    return {
        "environment_id": hash_id(identity_input),
        "engine_revision": engine,
        "font_set_fingerprint": string_hash_id(fonts),
        "resource_fingerprint": string_hash_id(resources),
    }


def map_transform(transform):
    if not isinstance(transform, dict):
        fail("resolved node transform is required")
    out = {}
    for key in ("a", "b", "c", "d"):
        value = transform.get(key)
        if not isinstance(value, str) or not DECIMAL_RE.fullmatch(value):
            fail(f"node.transform.{key} must be a normalized exact decimal string")
        out[key] = value
    out["tx"] = require_int(transform.get("tx"), "node.transform.tx")
    out["ty"] = require_int(transform.get("ty"), "node.transform.ty")
    return out


def adapt_viewer_geometry(viewer, document_id, revision_id):
    require_uuid(document_id, "document_id")
    require_hash_id(revision_id, "revision_id")

    if not isinstance(viewer, dict):
        fail("ViewerGeometryDocument must be a JSON object")
    document = viewer.get("document")
    scene = viewer.get("scene")
    if not isinstance(document, dict) or not isinstance(scene, dict):
        fail("ViewerGeometryDocument requires document and scene objects")

    source = document.get("source")
    if not isinstance(source, dict):
        fail("document.source is required")
    source_hash = require_sha(source.get("source_hash"), "document.source.source_hash")

    viewer_pages = document.get("pages")
    surfaces = scene.get("surfaces")
    raw_nodes = scene.get("nodes")
    stories_in = document.get("stories")
    if not isinstance(viewer_pages, list) or not isinstance(surfaces, list):
        fail("document.pages and scene.surfaces must be arrays")
    if not isinstance(raw_nodes, list) or not isinstance(stories_in, list):
        fail("scene.nodes and document.stories must be arrays")

    pages_by_id = {}
    pages = []
    seen_orders = set()
    for page in viewer_pages:
        page_id = require_uuid(page.get("id"), "document.pages[].id")
        index = require_int(page.get("index"), "document.pages[].index")
        if index <= 0:
            fail("Viewer page index is one-based and must be positive")
        order = index - 1
        if page_id in pages_by_id:
            fail(f"duplicate Viewer page id {page_id}")
        if order in seen_orders:
            fail(f"duplicate Viewer page order {order}")
        seen_orders.add(order)
        out = {
            "page_id": page_id,
            "order": order,
            "width_emu": require_int(page.get("width_emu"), "document.pages[].width_emu"),
            "height_emu": require_int(page.get("height_emu"), "document.pages[].height_emu"),
        }
        if out["width_emu"] <= 0 or out["height_emu"] <= 0:
            fail("page width/height must be positive")
        pages_by_id[page_id] = out
        pages.append(out)

    surfaces_by_id = {}
    for surface in surfaces:
        page_id = require_uuid(surface.get("origin"), "scene.surfaces[].origin")
        if page_id in surfaces_by_id:
            fail(f"duplicate scene surface {page_id}")
        size = surface.get("size")
        if not isinstance(size, dict):
            fail("scene surface size is required")
        width = require_int(size.get("width"), "scene.surfaces[].size.width")
        height = require_int(size.get("height"), "scene.surfaces[].size.height")
        surfaces_by_id[page_id] = (width, height)

    if set(surfaces_by_id) != set(pages_by_id):
        fail("Viewer pages and scene surfaces must have identical canonical page ids")
    for page_id, page in pages_by_id.items():
        if surfaces_by_id[page_id] != (page["width_emu"], page["height_emu"]):
            fail(f"surface/page size mismatch for {page_id}")

    node_inputs = {}
    for item in raw_nodes:
        node_id = require_uuid(item.get("origin"), "scene.nodes[].origin")
        if node_id in node_inputs:
            fail(f"duplicate resolved node {node_id}")
        parent = require_uuid(item.get("parent_origin"), "scene.nodes[].parent_origin")
        bounds = item.get("bounds")
        if not isinstance(bounds, dict):
            fail("resolved node bounds are required")
        mapped_bounds = {
            "x": require_int(bounds.get("x"), "node.bounds.x"),
            "y": require_int(bounds.get("y"), "node.bounds.y"),
            "width": require_int(bounds.get("width"), "node.bounds.width"),
            "height": require_int(bounds.get("height"), "node.bounds.height"),
        }
        if mapped_bounds["width"] <= 0 or mapped_bounds["height"] <= 0:
            fail("resolved node width/height must be positive")
        node_inputs[node_id] = {
            "node_id": node_id,
            "parent_origin": parent,
            "bounds": mapped_bounds,
            "transform": map_transform(item.get("transform")),
        }

    page_cache = {}

    def resolve_page(node_id, trail=()):
        if node_id in page_cache:
            return page_cache[node_id]
        if node_id in trail:
            fail("resolved node parent cycle: " + " -> ".join(trail + (node_id,)))
        item = node_inputs[node_id]
        parent = item["parent_origin"]
        if parent in pages_by_id:
            page_cache[node_id] = parent
            return parent
        if parent in node_inputs:
            page_id = resolve_page(parent, trail + (node_id,))
            page_cache[node_id] = page_id
            return page_id
        fail(f"node {node_id} parent {parent} resolves to neither page nor node")

    story_map = {}
    stories = []
    for item in stories_in:
        story_id = require_uuid(item.get("id"), "document.stories[].id")
        if story_id in story_map:
            fail(f"duplicate Viewer Story {story_id}")
        text = item.get("text")
        if not isinstance(text, str):
            fail("Viewer Story text must be a string")
        story_map[story_id] = text
        stories.append({
            "story_id": story_id,
            "text": text,
            "text_fidelity": "partial",
        })

    node_kind = {node_id: "unknown" for node_id in node_inputs}
    node_resource = {node_id: None for node_id in node_inputs}
    node_paint = {node_id: None for node_id in node_inputs}

    story_frames = []
    seen_frame_ordinals = set()
    for item in viewer.get("story_frames", []):
        story_id = require_uuid(item.get("story_id"), "story_frames[].story_id")
        node_id = require_uuid(item.get("frame_id"), "story_frames[].frame_id")
        ordinal = require_int(item.get("ordinal"), "story_frames[].ordinal")
        if story_id not in story_map:
            fail(f"story frame references unknown Story {story_id}")
        if node_id not in node_inputs:
            fail(f"story frame references unknown node {node_id}")
        if ordinal < 0:
            fail("story frame ordinal must be non-negative")
        key = (story_id, ordinal)
        if key in seen_frame_ordinals:
            fail(f"duplicate story frame ordinal {story_id}:{ordinal}")
        seen_frame_ordinals.add(key)
        if node_kind[node_id] not in {"unknown", "text_frame"}:
            fail(f"node {node_id} has conflicting text/image semantic bindings")
        node_kind[node_id] = "text_frame"
        story_frames.append({
            "story_id": story_id,
            "frame_ordinal": ordinal,
            "node_id": node_id,
        })

    resources = []
    resource_ids = set()
    for item in viewer.get("images", []):
        resource_id = require_uuid(item.get("resource_id"), "images[].resource_id")
        if resource_id in resource_ids:
            fail(f"duplicate image resource {resource_id}")
        resource_ids.add(resource_id)
        mime = item.get("mime")
        if not isinstance(mime, str) or not mime:
            fail("serialized image descriptor must carry MIME")
        content_hash = item.get("content_hash")
        byte_len = item.get("byte_len")
        fetch_handle = item.get("fetch_handle")
        if content_hash is not None:
            require_sha(content_hash, "images[].content_hash")
        if byte_len is not None:
            byte_len = require_int(byte_len, "images[].byte_len")
            if byte_len < 0:
                fail("images[].byte_len must be non-negative")
        if fetch_handle is not None and (not isinstance(fetch_handle, str) or not fetch_handle):
            fail("images[].fetch_handle must be a non-empty opaque string")
        availability = "available" if content_hash is not None and fetch_handle is not None else "unknown"
        resources.append({
            "resource_id": resource_id,
            "kind": "image",
            "mime": mime,
            "content_hash": content_hash,
            "byte_len": byte_len,
            "availability": availability,
            "fetch_handle": fetch_handle,
        })
        node_ids = item.get("node_ids", [])
        if not isinstance(node_ids, list):
            fail("images[].node_ids must be an array")
        for node_id in node_ids:
            require_uuid(node_id, "images[].node_ids[]")
            if node_id not in node_inputs:
                fail(f"image resource references unknown node {node_id}")
            if node_kind[node_id] not in {"unknown", "picture_frame"}:
                fail(f"node {node_id} has conflicting text/image semantic bindings")
            if node_resource[node_id] not in {None, resource_id}:
                fail(f"node {node_id} has multiple image resources")
            node_kind[node_id] = "picture_frame"
            node_resource[node_id] = resource_id

    paints = []
    for item in viewer.get("paints", []):
        node_id = require_uuid(item.get("node_id"), "paints[].node_id")
        if node_id not in node_inputs:
            fail(f"paint references unknown node {node_id}")
        fill = item.get("solid_fill_rgb")
        line = item.get("solid_line")
        mapped_fill = rgb(fill, "solid_fill_rgb") if fill is not None else None
        mapped_stroke = None
        if line is not None:
            if not isinstance(line, dict):
                fail("solid_line must be an object")
            width = require_int(line.get("width_emu"), "solid_line.width_emu")
            if width < 0:
                fail("solid_line.width_emu must be non-negative")
            mapped_stroke = {
                "color": rgb(line.get("rgb"), "solid_line.rgb"),
                "width_emu": width,
            }
        if mapped_fill is None and mapped_stroke is None:
            continue
        paint_id = f"paint.{node_id}"
        if node_paint[node_id] is not None:
            fail(f"duplicate paint binding for node {node_id}")
        node_paint[node_id] = paint_id
        paints.append({
            "paint_id": paint_id,
            "fill": mapped_fill,
            "stroke": mapped_stroke,
        })

    nodes = []
    for node_id, item in node_inputs.items():
        parent = item["parent_origin"]
        page_id = resolve_page(node_id)
        parent_node_id = parent if parent in node_inputs else None
        nodes.append({
            "node_id": node_id,
            "page_id": page_id,
            "parent_node_id": parent_node_id,
            "kind": node_kind[node_id],
            "bounds": item["bounds"],
            "transform": item["transform"],
            "z_order": None,
            "paint_order": None,
            "paint_id": node_paint[node_id],
            "resource_id": node_resource[node_id],
        })

    diagnostics = []
    for item in document.get("diagnostics", []):
        diagnostics.append(map_viewer_diagnostic(item))
    node_ids = set(node_inputs)
    for item in scene.get("diagnostics", []):
        code = item.get("code")
        if not isinstance(code, str) or not code:
            fail("scene diagnostic must carry a stable code")
        origin = item.get("origin")
        origin_node = origin if isinstance(origin, str) and origin in node_ids else None
        diagnostics.append(diagnostic(
            code=code,
            severity="warning",
            origin_node_id=origin_node,
            message_key=code.lower(),
        ))

    reasons = []
    capabilities = []

    if nodes:
        diagnostics.append(diagnostic(
            "SCENE.STACKING.UNKNOWN",
            message_key="scene.stacking.unknown",
        ))
        reasons.append("stacking_order_unavailable")
        capabilities.append({
            "key": "render.geometry",
            "state": "supported",
            "note": None,
        })
        capabilities.append({
            "key": "render.stacking",
            "state": "unsupported",
            "note": "The bounded Viewer producer does not expose authored stacking order.",
        })

    unknown_nodes = [node_id for node_id, kind in node_kind.items() if kind == "unknown"]
    if unknown_nodes:
        diagnostics.append(diagnostic(
            "SCENE.NODE_KIND.PARTIAL",
            message_key="scene.node_kind.partial",
        ))
        reasons.append("node_kind_partial")
        capabilities.append({
            "key": "node.kind",
            "state": "partial",
            "note": "Only source-neutral proven text/image bindings classify nodes in V1.",
        })
    elif nodes:
        capabilities.append({"key": "node.kind", "state": "supported", "note": None})

    if nodes:
        capabilities.append({
            "key": "render.transforms",
            "state": "supported",
            "note": "Exact source-free affine transform coefficients and EMU translation are preserved.",
        })

    if stories:
        capabilities.append({
            "key": "render.text",
            "state": "partial",
            "note": "Unicode Story text is carried; browser typography/layout is not authoritative.",
        })
        if story_frames:
            capabilities.append({
                "key": "render.story-frame",
                "state": "partial",
                "note": "Source-neutral Story/frame topology is available; text shaping/flow remains server-authoritative.",
            })
        else:
            diagnostics.append(diagnostic(
                "SCENE.STORY_FRAME.UNBOUND",
                message_key="scene.story_frame.unbound",
            ))
            reasons.append("story_frame_binding_unavailable")
            capabilities.append({
                "key": "render.story-frame",
                "state": "unsupported",
                "note": "This Viewer payload does not expose source-neutral Story/frame bindings.",
            })

    if paints:
        reasons.append("paint_projection_partial")
        capabilities.append({
            "key": "render.paint",
            "state": "partial",
            "note": "Only explicit source-neutral solid fill/line state is carried.",
        })
    elif nodes:
        reasons.append("paint_projection_unavailable")
        capabilities.append({
            "key": "render.paint",
            "state": "unsupported",
            "note": "This Viewer payload does not expose source-neutral paint state.",
        })

    if resources:
        reasons.append("image_delivery_partial")
        capabilities.append({
            "key": "resource.image",
            "state": "partial",
            "note": "Image identity/bindings may be present; bytes are never embedded in the scene JSON.",
        })

    if any(d["severity"] == "warning" for d in diagnostics):
        reasons.append("viewer_fidelity_warnings")

    fidelity_state = "partial" if reasons or diagnostics else "supported"

    layout_environment = environment_from_viewer(scene)
    snapshot = {
        "protocol_version": "chaptera.scene.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "revision_id": revision_id,
        "snapshot_id": "sha256:" + "0" * 64,
        "layout_environment": layout_environment,
        "stacking_fidelity": "unknown",
        "pages": pages,
        "nodes": nodes,
        "stories": stories,
        "story_frames": story_frames,
        "paints": paints,
        "resources": resources,
        "diagnostics": diagnostics,
        "capabilities": capabilities,
        "fidelity": {
            "state": fidelity_state,
            "reasons": sorted(set(reasons)),
        },
    }
    return finalize_snapshot(snapshot)


def main():
    parser = argparse.ArgumentParser(
        description="Map source-free ViewerGeometryDocument JSON to BrowserSceneSnapshotV1"
    )
    parser.add_argument("input", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--document-id", required=True)
    parser.add_argument("--revision-id", required=True)
    args = parser.parse_args()

    viewer = json.loads(args.input.read_text(encoding="utf-8"))
    snapshot = adapt_viewer_geometry(viewer, args.document_id, args.revision_id)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(canonical_json(snapshot) + b"\n")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"viewer scene adapter failed: {exc}", file=sys.stderr)
        raise
