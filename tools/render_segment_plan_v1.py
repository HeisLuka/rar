#!/usr/bin/env python3
from __future__ import annotations
import copy
import hashlib
import json

SCHEMA = "chaptera.render-segment-plan.v1"


def _canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def _hash(value):
    return "sha256:" + hashlib.sha256(_canonical(value).encode("utf-8")).hexdigest()


def _atom_index(scene):
    out = {}
    for kind, atoms in scene["primitives"].items():
        for atom in atoms:
            if atom["atom_id"] in out:
                raise ValueError("duplicate atom_id")
            out[atom["atom_id"]] = (kind, atom)
    return out


def _public_meta(kind, atom, extra):
    extra = extra or {}
    return {
        "primitive_kind": kind,
        "paint_id": atom.get("paint_id"),
        "resource_id": atom.get("resource_id") or atom.get("font_resource_id"),
        "clip_id": extra.get("clip_id"),
        "effect_group_id": extra.get("effect_group_id"),
        "isolation": bool(extra.get("isolation", False)),
        "blend_mode": extra.get("blend_mode", "source-over"),
        "diagnostic_barrier": extra.get("diagnostic_barrier"),
        "barrier_unknown": bool(extra.get("barrier_unknown", False)),
    }


def _barrier_key(meta):
    return (
        meta["clip_id"],
        meta["effect_group_id"],
        meta["isolation"],
        meta["blend_mode"],
        meta["diagnostic_barrier"],
        meta["barrier_unknown"],
    )


def _batch_key(meta):
    return (
        meta["primitive_kind"],
        meta["paint_id"],
        meta["resource_id"],
        meta["clip_id"],
        meta["effect_group_id"],
        meta["blend_mode"],
    )


def plan_page(scene, page_id, *, visible_atom_ids=None, metadata_by_atom=None):
    index = _atom_index(scene)
    metadata_by_atom = metadata_by_atom or {}
    visible = None if visible_atom_ids is None else set(visible_atom_ids)

    ordered = []
    for aid in scene["paint_seq"]:
        entry = index.get(aid)
        if entry is None:
            raise ValueError(f"paint_seq references missing atom: {aid}")
        kind, atom = entry
        if atom.get("page_id") != page_id:
            continue
        if visible is not None and aid not in visible:
            continue
        ordered.append((aid, atom, _public_meta(kind, atom, metadata_by_atom.get(aid))))

    segments = []
    current = None
    for aid, atom, meta in ordered:
        barrier = _barrier_key(meta)
        if current is None or current["barrier_key"] != barrier or meta["barrier_unknown"]:
            current = {
                "barrier_key": barrier,
                "atoms": [],
                "batches": [],
                "unknown_barrier": meta["barrier_unknown"],
            }
            segments.append(current)
        current["atoms"].append(aid)

        key = _batch_key(meta)
        if not current["batches"] or current["batches"][-1]["batch_key"] != key:
            current["batches"].append({"batch_key": key, "atom_ids": []})
        current["batches"][-1]["atom_ids"].append(aid)

    public_segments = []
    for ordinal, segment in enumerate(segments):
        dependencies = {
            "atom_ids": list(segment["atoms"]),
            "node_ids": sorted({index[aid][1]["node_id"] for aid in segment["atoms"]}),
            "paint_ids": sorted({index[aid][1].get("paint_id") for aid in segment["atoms"] if index[aid][1].get("paint_id")}),
            "resource_ids": sorted({
                index[aid][1].get("resource_id") or index[aid][1].get("font_resource_id")
                for aid in segment["atoms"]
                if index[aid][1].get("resource_id") or index[aid][1].get("font_resource_id")
            }),
            "clip_ids": sorted({metadata_by_atom.get(aid, {}).get("clip_id") for aid in segment["atoms"] if metadata_by_atom.get(aid, {}).get("clip_id")}),
            "effect_group_ids": sorted({metadata_by_atom.get(aid, {}).get("effect_group_id") for aid in segment["atoms"] if metadata_by_atom.get(aid, {}).get("effect_group_id")}),
        }
        row = {
            "ordinal": ordinal,
            "atom_ids": list(segment["atoms"]),
            "barrier_key": list(segment["barrier_key"]),
            "unknown_barrier": segment["unknown_barrier"],
            "batches": [
                {"batch_key": list(batch["batch_key"]), "atom_ids": list(batch["atom_ids"])}
                for batch in segment["batches"]
            ],
            "dependencies": dependencies,
        }
        row["fingerprint"] = _hash({"schema": SCHEMA, "page_id": page_id, "segment": row})
        public_segments.append(row)

    plan = {
        "schema": SCHEMA,
        "page_id": page_id,
        "planner_version": 1,
        "visible_atom_count": len(ordered),
        "ordered_atom_ids": [aid for aid, _, _ in ordered],
        "segments": public_segments,
        "segment_count": len(public_segments),
        "batch_count": sum(len(s["batches"]) for s in public_segments),
    }
    plan["fingerprint"] = _hash(plan)
    return plan


def plan_scene(scene, *, visible_by_page=None, metadata_by_atom=None):
    visible_by_page = visible_by_page or {}
    pages = []
    for page in sorted(scene["pages"], key=lambda p: (p["order"], p["page_id"])):
        pid = page["page_id"]
        pages.append(plan_page(
            scene,
            pid,
            visible_atom_ids=visible_by_page.get(pid),
            metadata_by_atom=metadata_by_atom,
        ))
    out = {"schema": "chaptera.render-segment-scene-plan.v1", "pages": pages}
    out["fingerprint"] = _hash(out)
    return out


def incremental_replan(previous, scene, *, changed_atom_ids, metadata_by_atom=None, visible_by_page=None, unknown_dependency=False):
    clean = plan_scene(scene, visible_by_page=visible_by_page, metadata_by_atom=metadata_by_atom)
    previous_by_page = {p["page_id"]: p for p in previous["pages"]}
    changed = set(changed_atom_ids)
    affected_pages = set()
    for page in clean["pages"]:
        if changed.intersection(page["ordered_atom_ids"]):
            affected_pages.add(page["page_id"])
    for page in previous["pages"]:
        if changed.intersection(page["ordered_atom_ids"]):
            affected_pages.add(page["page_id"])

    reused_pages = 0
    reused_segments = 0
    rebuilt_segments = 0
    for page in clean["pages"]:
        old = previous_by_page.get(page["page_id"])
        if old is not None and page["page_id"] not in affected_pages and old["fingerprint"] == page["fingerprint"]:
            reused_pages += 1
            reused_segments += page["segment_count"]
            continue
        if old is None:
            rebuilt_segments += page["segment_count"]
            continue
        if unknown_dependency:
            rebuilt_segments += page["segment_count"]
            continue
        old_fps = {segment["fingerprint"] for segment in old["segments"]}
        for segment in page["segments"]:
            if segment["fingerprint"] in old_fps:
                reused_segments += 1
            else:
                rebuilt_segments += 1

    return {
        "plan": clean,
        "rebuild_scope": "affected_pages" if unknown_dependency else "affected_segment_neighborhoods",
        "affected_pages": sorted(affected_pages),
        "reused_pages": reused_pages,
        "reused_segments": reused_segments,
        "rebuilt_segments": rebuilt_segments,
        "full_plan_equivalent": True,
    }


def instantiate_page_plan(source_page_plan, placements):
    return {
        "schema": "chaptera.render-segment-page-instances.v1",
        "source_page_id": source_page_plan["page_id"],
        "source_plan_fingerprint": source_page_plan["fingerprint"],
        "source_atom_count": len(source_page_plan["ordered_atom_ids"]),
        "cloned_atom_count": 0,
        "instances": [
            {
                "instance_id": row["instance_id"],
                "transform": copy.deepcopy(row["transform"]),
                "source_plan_fingerprint": source_page_plan["fingerprint"],
            }
            for row in placements
        ],
    }
