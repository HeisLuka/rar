#!/usr/bin/env python3
import copy
import hashlib
import json

def canonical_json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))

def hash_id(value):
    return "sha256:" + hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()

def atom_id(node_id, local_ordinal, primitive_kind):
    return f"{node_id}:{local_ordinal}:{primitive_kind}"

def compile_render_scene(source):
    pages = sorted(copy.deepcopy(source["pages"]), key=lambda p: (p["order"], p["page_id"]))
    page_order = {p["page_id"]: p["order"] for p in pages}
    nodes = sorted(
        copy.deepcopy(source["nodes"]),
        key=lambda n: (page_order[n["page_id"]], n.get("paint_order") is None, n.get("paint_order") or 0, n["node_id"]),
    )
    paints = sorted(copy.deepcopy(source.get("paints", [])), key=lambda p: p["paint_id"])
    resources = sorted(copy.deepcopy(source.get("resources", [])), key=lambda r: r["resource_id"])
    glyph_runs = sorted(
        copy.deepcopy(source.get("glyph_runs", [])),
        key=lambda g: (page_order[g["page_id"]], g["frame_node_id"], g["story_id"], g["scalar_start"], g["scalar_end"]),
    )

    transform_index = {}
    transforms = []
    def intern_transform(value):
        key = canonical_json(value)
        if key not in transform_index:
            transform_index[key] = len(transforms)
            transforms.append(copy.deepcopy(value))
        return transform_index[key]

    rects, images, glyph_atoms, atom_map, paint_seq, diagnostics = [], [], [], [], [], []
    node_atoms = {}

    for node in nodes:
        tid = intern_transform(node["transform"])
        kind = node["kind"]
        created = []
        if kind in {"shape", "unknown"}:
            atom = {
                "atom_id": atom_id(node["node_id"], 0, "rect"),
                "node_id": node["node_id"],
                "page_id": node["page_id"],
                "bounds": copy.deepcopy(node["bounds"]),
                "transform_index": tid,
                "paint_id": node.get("paint_id"),
            }
            rects.append(atom)
            created.append(atom["atom_id"])
        elif kind == "picture_frame":
            atom = {
                "atom_id": atom_id(node["node_id"], 0, "image"),
                "node_id": node["node_id"],
                "page_id": node["page_id"],
                "bounds": copy.deepcopy(node["bounds"]),
                "transform_index": tid,
                "resource_id": node.get("resource_id"),
                "paint_id": node.get("paint_id"),
            }
            images.append(atom)
            created.append(atom["atom_id"])
        elif kind == "text_frame":
            pass
        else:
            diagnostics.append({
                "code": "render.unsupported_node_kind",
                "severity": "warning",
                "origin_node_id": node["node_id"],
                "detail": kind,
            })

        if created:
            node_atoms[node["node_id"]] = created
            atom_map.append({"node_id": node["node_id"], "atoms": created})
            paint_seq.extend(created)

    glyph_ordinals = {}
    for run in glyph_runs:
        node_id = run["frame_node_id"]
        ordinal = glyph_ordinals.get(node_id, 0)
        aid = atom_id(node_id, ordinal, "glyph_run")
        glyph_ordinals[node_id] = ordinal + 1
        atom = {
            "atom_id": aid,
            "node_id": node_id,
            "page_id": run["page_id"],
            "story_id": run["story_id"],
            "frame_node_id": run["frame_node_id"],
            "scalar_start": run["scalar_start"],
            "scalar_end": run["scalar_end"],
            "font_resource_id": run["font_resource_id"],
            "glyphs": copy.deepcopy(run["glyphs"]),
            "paint_id": run.get("paint_id"),
        }
        glyph_atoms.append(atom)
        if node_id not in node_atoms:
            node_atoms[node_id] = []
            atom_map.append({"node_id": node_id, "atoms": node_atoms[node_id]})
        node_atoms[node_id].append(aid)
        paint_seq.append(aid)

    diagnostics.extend(copy.deepcopy(source.get("diagnostics", [])))
    diagnostics.sort(key=lambda d: (d["code"], d.get("origin_node_id") or "", d.get("detail") or ""))
    atom_map.sort(key=lambda x: x["node_id"])

    result = {
        "render_scene_version": "chaptera.render-scene.v1",
        "scene_revision": source["scene_revision"],
        "order_authority": source["order_authority"],
        "pages": pages,
        "tables": {
            "transforms": transforms,
            "clips": sorted(copy.deepcopy(source.get("clips", [])), key=lambda c: c["clip_id"]),
            "paints": paints,
            "resources": resources,
        },
        "primitives": {
            "rects": rects,
            "images": images,
            "glyph_runs": glyph_atoms,
        },
        "paint_seq": paint_seq,
        "atom_map": atom_map,
        "diagnostics": diagnostics,
    }
    result["render_scene_id"] = hash_id(result)
    return result
