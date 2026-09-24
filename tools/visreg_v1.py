import copy
import hashlib
import json

from scene_v1 import canonical_json, normalize_snapshot

REPORT_VERSION = "chaptera.visreg.v1"
RENDER_MANIFEST_VERSION = "chaptera.render-artifact.v1"
STAGE_ORDER = {
    "structural": 0,
    "geometry": 1,
    "text_layout": 2,
    "render": 3,
}


def _hash_id(value):
    return "sha256:" + hashlib.sha256(canonical_json(value)).hexdigest()


def _identity(snapshot):
    return {
        "document_id": snapshot["document_id"],
        "source_hash": snapshot["source_hash"],
        "revision_id": snapshot["revision_id"],
        "snapshot_id": snapshot["snapshot_id"],
    }


def _index(items, key):
    return {item[key]: item for item in items}


def _node_page_map(snapshot):
    return {node["node_id"]: node["page_id"] for node in snapshot["nodes"]}


def _difference(
    stage,
    kind,
    signal,
    baseline,
    candidate,
    *,
    page_id=None,
    node_id=None,
    story_id=None,
    paint_id=None,
    resource_id=None,
):
    value = {
        "stage": stage,
        "kind": kind,
        "signal": signal,
        "baseline": baseline,
        "candidate": candidate,
    }
    for key, item in (
        ("page_id", page_id),
        ("node_id", node_id),
        ("story_id", story_id),
        ("paint_id", paint_id),
        ("resource_id", resource_id),
    ):
        if item is not None:
            value[key] = item
    return value


def _sort_differences(differences):
    return sorted(
        differences,
        key=lambda item: (
            STAGE_ORDER[item["stage"]],
            item.get("page_id", ""),
            item.get("node_id", ""),
            item.get("story_id", ""),
            item.get("paint_id", ""),
            item.get("resource_id", ""),
            item["kind"],
            json.dumps(item["baseline"], sort_keys=True, ensure_ascii=False),
            json.dumps(item["candidate"], sort_keys=True, ensure_ascii=False),
        ),
    )


def _compare_keyed_collection(
    differences,
    *,
    baseline_items,
    candidate_items,
    key,
    stage,
    kind_prefix,
    signal="semantic",
    id_field=None,
):
    baseline_map = _index(baseline_items, key)
    candidate_map = _index(candidate_items, key)
    for item_id in sorted(set(baseline_map) | set(candidate_map)):
        before = baseline_map.get(item_id)
        after = candidate_map.get(item_id)
        kwargs = {id_field: item_id} if id_field else {}
        if before is None:
            differences.append(
                _difference(stage, f"{kind_prefix}_added", signal, None, after, **kwargs)
            )
        elif after is None:
            differences.append(
                _difference(stage, f"{kind_prefix}_removed", signal, before, None, **kwargs)
            )
        elif before != after:
            differences.append(
                _difference(stage, f"{kind_prefix}_changed", signal, before, after, **kwargs)
            )


def _validate_render_manifest(manifest, snapshot):
    if manifest is None:
        return None
    if manifest.get("manifest_version") != RENDER_MANIFEST_VERSION:
        raise ValueError("unsupported render manifest version")
    if manifest.get("snapshot_id") != snapshot["snapshot_id"]:
        raise ValueError("render manifest snapshot_id does not match scene snapshot")
    pages = manifest.get("pages")
    if not isinstance(pages, list):
        raise ValueError("render manifest pages must be a list")
    seen = set()
    for page in pages:
        page_id = page.get("page_id")
        artifact_sha256 = page.get("artifact_sha256")
        if not isinstance(page_id, str) or page_id in seen:
            raise ValueError("render manifest page_id must be unique strings")
        if (
            not isinstance(artifact_sha256, str)
            or len(artifact_sha256) != 64
            or any(ch not in "0123456789abcdef" for ch in artifact_sha256)
        ):
            raise ValueError("render artifact_sha256 must be lowercase SHA-256")
        seen.add(page_id)
    return {
        "manifest_version": RENDER_MANIFEST_VERSION,
        "snapshot_id": snapshot["snapshot_id"],
        "pages": sorted(copy.deepcopy(pages), key=lambda item: item["page_id"]),
    }


def compare_scenes(baseline, candidate, baseline_render=None, candidate_render=None):
    baseline = normalize_snapshot(baseline)
    candidate = normalize_snapshot(candidate)
    baseline_render = _validate_render_manifest(baseline_render, baseline)
    candidate_render = _validate_render_manifest(candidate_render, candidate)

    differences = []
    baseline_pages = _index(baseline["pages"], "page_id")
    candidate_pages = _index(candidate["pages"], "page_id")

    for page_id in sorted(set(baseline_pages) | set(candidate_pages)):
        before = baseline_pages.get(page_id)
        after = candidate_pages.get(page_id)
        if before is None:
            differences.append(
                _difference(
                    "structural",
                    "page_added",
                    "semantic",
                    None,
                    after,
                    page_id=page_id,
                )
            )
            continue
        if after is None:
            differences.append(
                _difference(
                    "structural",
                    "page_removed",
                    "semantic",
                    before,
                    None,
                    page_id=page_id,
                )
            )
            continue
        if before["order"] != after["order"]:
            differences.append(
                _difference(
                    "structural",
                    "page_order_changed",
                    "semantic",
                    before["order"],
                    after["order"],
                    page_id=page_id,
                )
            )
        before_size = {
            "width_emu": before["width_emu"],
            "height_emu": before["height_emu"],
        }
        after_size = {
            "width_emu": after["width_emu"],
            "height_emu": after["height_emu"],
        }
        if before_size != after_size:
            differences.append(
                _difference(
                    "geometry",
                    "page_size_changed",
                    "semantic",
                    before_size,
                    after_size,
                    page_id=page_id,
                )
            )

    baseline_nodes = _index(baseline["nodes"], "node_id")
    candidate_nodes = _index(candidate["nodes"], "node_id")
    for node_id in sorted(set(baseline_nodes) | set(candidate_nodes)):
        before = baseline_nodes.get(node_id)
        after = candidate_nodes.get(node_id)
        page_id = (after or before)["page_id"]
        if before is None:
            differences.append(
                _difference(
                    "structural",
                    "node_added",
                    "semantic",
                    None,
                    after,
                    page_id=page_id,
                    node_id=node_id,
                )
            )
            continue
        if after is None:
            differences.append(
                _difference(
                    "structural",
                    "node_removed",
                    "semantic",
                    before,
                    None,
                    page_id=page_id,
                    node_id=node_id,
                )
            )
            continue

        structural_before = {
            key: before.get(key)
            for key in ("page_id", "parent_node_id", "kind")
        }
        structural_after = {
            key: after.get(key)
            for key in ("page_id", "parent_node_id", "kind")
        }
        if structural_before != structural_after:
            differences.append(
                _difference(
                    "structural",
                    "node_structure_changed",
                    "semantic",
                    structural_before,
                    structural_after,
                    page_id=page_id,
                    node_id=node_id,
                )
            )

        geometry_before = {
            key: before.get(key)
            for key in ("bounds", "transform", "z_order", "paint_order")
        }
        geometry_after = {
            key: after.get(key)
            for key in ("bounds", "transform", "z_order", "paint_order")
        }
        if geometry_before != geometry_after:
            differences.append(
                _difference(
                    "geometry",
                    "node_geometry_changed",
                    "semantic",
                    geometry_before,
                    geometry_after,
                    page_id=page_id,
                    node_id=node_id,
                )
            )

        render_before = {
            key: before.get(key)
            for key in ("paint_id", "resource_id")
        }
        render_after = {
            key: after.get(key)
            for key in ("paint_id", "resource_id")
        }
        if render_before != render_after:
            differences.append(
                _difference(
                    "render",
                    "node_render_binding_changed",
                    "semantic",
                    render_before,
                    render_after,
                    page_id=page_id,
                    node_id=node_id,
                )
            )

    baseline_stories = _index(baseline["stories"], "story_id")
    candidate_stories = _index(candidate["stories"], "story_id")
    frame_page = {}
    for snapshot in (baseline, candidate):
        nodes = _index(snapshot["nodes"], "node_id")
        for frame in snapshot["story_frames"]:
            node = nodes.get(frame["node_id"])
            if node is not None:
                frame_page.setdefault(frame["story_id"], node["page_id"])

    for story_id in sorted(set(baseline_stories) | set(candidate_stories)):
        before = baseline_stories.get(story_id)
        after = candidate_stories.get(story_id)
        page_id = frame_page.get(story_id)
        if before is None:
            kind = "story_added"
        elif after is None:
            kind = "story_removed"
        elif before != after:
            kind = "story_changed"
        else:
            continue
        differences.append(
            _difference(
                "text_layout",
                kind,
                "semantic",
                before,
                after,
                page_id=page_id,
                story_id=story_id,
            )
        )

    baseline_frames = {
        (item["story_id"], item["frame_ordinal"]): item
        for item in baseline["story_frames"]
    }
    candidate_frames = {
        (item["story_id"], item["frame_ordinal"]): item
        for item in candidate["story_frames"]
    }
    for key in sorted(set(baseline_frames) | set(candidate_frames)):
        before = baseline_frames.get(key)
        after = candidate_frames.get(key)
        if before == after:
            continue
        node_id = (after or before)["node_id"]
        page_id = (candidate_nodes.get(node_id) or baseline_nodes.get(node_id) or {}).get("page_id")
        differences.append(
            _difference(
                "text_layout",
                "story_frame_changed",
                "semantic",
                before,
                after,
                page_id=page_id,
                node_id=node_id,
                story_id=key[0],
            )
        )

    _compare_keyed_collection(
        differences,
        baseline_items=baseline["paints"],
        candidate_items=candidate["paints"],
        key="paint_id",
        stage="render",
        kind_prefix="paint",
        id_field="paint_id",
    )
    _compare_keyed_collection(
        differences,
        baseline_items=baseline["resources"],
        candidate_items=candidate["resources"],
        key="resource_id",
        stage="render",
        kind_prefix="resource",
        id_field="resource_id",
    )

    baseline_node_pages = _node_page_map(baseline)
    candidate_node_pages = _node_page_map(candidate)
    baseline_diagnostics = {
        (
            item["severity"],
            item["code"],
            item.get("origin_node_id"),
            item["message_key"],
        ): item
        for item in baseline["diagnostics"]
    }
    candidate_diagnostics = {
        (
            item["severity"],
            item["code"],
            item.get("origin_node_id"),
            item["message_key"],
        ): item
        for item in candidate["diagnostics"]
    }
    for token in sorted(
        set(baseline_diagnostics) | set(candidate_diagnostics),
        key=lambda item: tuple("" if part is None else str(part) for part in item),
    ):
        before = baseline_diagnostics.get(token)
        after = candidate_diagnostics.get(token)
        node_id = token[2]
        page_id = candidate_node_pages.get(node_id) or baseline_node_pages.get(node_id)
        differences.append(
            _difference(
                "text_layout",
                "diagnostic_added" if before is None else "diagnostic_removed",
                "semantic",
                before,
                after,
                page_id=page_id,
                node_id=node_id,
            )
        )

    if baseline_render is not None or candidate_render is not None:
        if baseline_render is None or candidate_render is None:
            differences.append(
                _difference(
                    "render",
                    "render_manifest_presence_changed",
                    "artifact",
                    baseline_render,
                    candidate_render,
                )
            )
        else:
            baseline_render_pages = _index(baseline_render["pages"], "page_id")
            candidate_render_pages = _index(candidate_render["pages"], "page_id")
            for page_id in sorted(set(baseline_render_pages) | set(candidate_render_pages)):
                before = baseline_render_pages.get(page_id)
                after = candidate_render_pages.get(page_id)
                if before == after:
                    continue
                differences.append(
                    _difference(
                        "render",
                        "render_artifact_changed",
                        "artifact",
                        before,
                        after,
                        page_id=page_id,
                    )
                )

    differences = _sort_differences(differences)
    stage_counts = {stage: 0 for stage in STAGE_ORDER}
    for item in differences:
        stage_counts[item["stage"]] += 1

    semantic_count = sum(item["signal"] == "semantic" for item in differences)
    artifact_count = sum(item["signal"] == "artifact" for item in differences)

    if not differences:
        classification = "equal"
    elif semantic_count == 0:
        classification = "render_only"
    elif artifact_count:
        classification = "mixed"
    else:
        classification = "semantic_regression"

    report = {
        "report_version": REPORT_VERSION,
        "baseline": _identity(baseline),
        "candidate": _identity(candidate),
        "summary": {
            "classification": classification,
            "difference_count": len(differences),
            "semantic_difference_count": semantic_count,
            "artifact_difference_count": artifact_count,
            "stage_counts": stage_counts,
        },
        "differences": differences,
    }
    report["report_id"] = _hash_id(report)
    return report
