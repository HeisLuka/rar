#!/usr/bin/env python3
import copy
import hashlib
import json


REPORT_VERSION = "chaptera.visreg.report.v1"
STAGE_ORDER = {
    "structure": 0,
    "layout": 1,
    "text_layout": 2,
    "render_input": 3,
    "render": 4,
    "contract": 5,
}


def canonical_json(value):
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def hash_id(value):
    return "sha256:" + hashlib.sha256(canonical_json(value)).hexdigest()


def _index(items, key):
    out = {}
    for item in items:
        item_key = key(item)
        if item_key in out:
            raise AssertionError(f"duplicate visual-regression identity: {item_key!r}")
        out[item_key] = item
    return out


def _page_for_node(scene):
    return {node["node_id"]: node["page_id"] for node in scene.get("nodes", [])}


def _append(
    diffs,
    *,
    stage,
    code,
    origin_type,
    origin_id,
    page_id=None,
    before=None,
    after=None,
):
    diffs.append(
        {
            "stage": stage,
            "code": code,
            "origin_type": origin_type,
            "origin_id": str(origin_id),
            "page_id": page_id,
            "before": copy.deepcopy(before),
            "after": copy.deepcopy(after),
        }
    )


def _compare_keyed(
    diffs,
    *,
    baseline_items,
    candidate_items,
    key,
    stage,
    origin_type,
    added_code,
    removed_code,
    changed_code,
    page_for=None,
):
    baseline = _index(baseline_items, key)
    candidate = _index(candidate_items, key)
    for item_id in sorted(set(baseline) | set(candidate), key=str):
        before = baseline.get(item_id)
        after = candidate.get(item_id)
        page_id = None
        if page_for is not None:
            page_id = page_for(item_id, before, after)
        if before is None:
            _append(
                diffs,
                stage=stage,
                code=added_code,
                origin_type=origin_type,
                origin_id=item_id,
                page_id=page_id,
                after=after,
            )
        elif after is None:
            _append(
                diffs,
                stage=stage,
                code=removed_code,
                origin_type=origin_type,
                origin_id=item_id,
                page_id=page_id,
                before=before,
            )
        elif before != after:
            _append(
                diffs,
                stage=stage,
                code=changed_code,
                origin_type=origin_type,
                origin_id=item_id,
                page_id=page_id,
                before=before,
                after=after,
            )


def _compare_pages(diffs, baseline, candidate):
    b = _index(baseline["pages"], lambda page: page["page_id"])
    c = _index(candidate["pages"], lambda page: page["page_id"])
    for page_id in sorted(set(b) | set(c)):
        before = b.get(page_id)
        after = c.get(page_id)
        if before is None:
            _append(
                diffs,
                stage="structure",
                code="page.added",
                origin_type="page",
                origin_id=page_id,
                page_id=page_id,
                after=after,
            )
            continue
        if after is None:
            _append(
                diffs,
                stage="structure",
                code="page.removed",
                origin_type="page",
                origin_id=page_id,
                page_id=page_id,
                before=before,
            )
            continue
        if before["order"] != after["order"]:
            _append(
                diffs,
                stage="structure",
                code="page.order_changed",
                origin_type="page",
                origin_id=page_id,
                page_id=page_id,
                before=before["order"],
                after=after["order"],
            )
        for field in ("width_emu", "height_emu"):
            if before[field] != after[field]:
                _append(
                    diffs,
                    stage="layout",
                    code=f"page.{field}_changed",
                    origin_type="page",
                    origin_id=page_id,
                    page_id=page_id,
                    before=before[field],
                    after=after[field],
                )


def _compare_nodes(diffs, baseline, candidate):
    b = _index(baseline["nodes"], lambda node: node["node_id"])
    c = _index(candidate["nodes"], lambda node: node["node_id"])
    for node_id in sorted(set(b) | set(c)):
        before = b.get(node_id)
        after = c.get(node_id)
        page_id = (after or before).get("page_id")
        if before is None:
            _append(
                diffs,
                stage="structure",
                code="node.added",
                origin_type="node",
                origin_id=node_id,
                page_id=page_id,
                after=after,
            )
            continue
        if after is None:
            _append(
                diffs,
                stage="structure",
                code="node.removed",
                origin_type="node",
                origin_id=node_id,
                page_id=page_id,
                before=before,
            )
            continue

        for field in ("page_id", "parent_node_id", "kind"):
            if before.get(field) != after.get(field):
                _append(
                    diffs,
                    stage="structure",
                    code=f"node.{field}_changed",
                    origin_type="node",
                    origin_id=node_id,
                    page_id=page_id,
                    before=before.get(field),
                    after=after.get(field),
                )

        for field in ("bounds", "transform", "z_order", "paint_order"):
            if before.get(field) != after.get(field):
                _append(
                    diffs,
                    stage="layout",
                    code=f"node.{field}_changed",
                    origin_type="node",
                    origin_id=node_id,
                    page_id=page_id,
                    before=before.get(field),
                    after=after.get(field),
                )

        for field in ("paint_id", "resource_id"):
            if before.get(field) != after.get(field):
                _append(
                    diffs,
                    stage="render_input",
                    code=f"node.{field}_changed",
                    origin_type="node",
                    origin_id=node_id,
                    page_id=page_id,
                    before=before.get(field),
                    after=after.get(field),
                )


def _compare_stories(diffs, baseline, candidate):
    b = _index(baseline["stories"], lambda story: story["story_id"])
    c = _index(candidate["stories"], lambda story: story["story_id"])
    story_pages = {}
    for scene in (baseline, candidate):
        nodes = _page_for_node(scene)
        for frame in scene["story_frames"]:
            page_id = nodes.get(frame["node_id"])
            if page_id is not None:
                story_pages.setdefault(frame["story_id"], page_id)

    for story_id in sorted(set(b) | set(c)):
        before = b.get(story_id)
        after = c.get(story_id)
        page_id = story_pages.get(story_id)
        if before is None:
            _append(
                diffs,
                stage="text_layout",
                code="story.added",
                origin_type="story",
                origin_id=story_id,
                page_id=page_id,
                after=after,
            )
            continue
        if after is None:
            _append(
                diffs,
                stage="text_layout",
                code="story.removed",
                origin_type="story",
                origin_id=story_id,
                page_id=page_id,
                before=before,
            )
            continue
        for field in ("text", "text_fidelity"):
            if before.get(field) != after.get(field):
                _append(
                    diffs,
                    stage="text_layout",
                    code=f"story.{field}_changed",
                    origin_type="story",
                    origin_id=story_id,
                    page_id=page_id,
                    before=before.get(field),
                    after=after.get(field),
                )

    _compare_keyed(
        diffs,
        baseline_items=baseline["story_frames"],
        candidate_items=candidate["story_frames"],
        key=lambda frame: (
            frame["story_id"],
            frame["frame_ordinal"],
            frame["node_id"],
        ),
        stage="text_layout",
        origin_type="story_frame",
        added_code="story_frame.added",
        removed_code="story_frame.removed",
        changed_code="story_frame.changed",
        page_for=lambda _key, before, after: _page_for_node(candidate).get(
            (after or before)["node_id"]
        )
        or _page_for_node(baseline).get((after or before)["node_id"]),
    )


def _compare_render_inputs(diffs, baseline, candidate):
    page_by_node = {**_page_for_node(baseline), **_page_for_node(candidate)}

    _compare_keyed(
        diffs,
        baseline_items=baseline["paints"],
        candidate_items=candidate["paints"],
        key=lambda paint: paint["paint_id"],
        stage="render_input",
        origin_type="paint",
        added_code="paint.added",
        removed_code="paint.removed",
        changed_code="paint.changed",
    )
    _compare_keyed(
        diffs,
        baseline_items=baseline["resources"],
        candidate_items=candidate["resources"],
        key=lambda resource: resource["resource_id"],
        stage="render_input",
        origin_type="resource",
        added_code="resource.added",
        removed_code="resource.removed",
        changed_code="resource.changed",
    )
    _compare_keyed(
        diffs,
        baseline_items=baseline["diagnostics"],
        candidate_items=candidate["diagnostics"],
        key=lambda item: (
            item["severity"],
            item["code"],
            item.get("origin_node_id"),
            item["message_key"],
        ),
        stage="contract",
        origin_type="diagnostic",
        added_code="diagnostic.added",
        removed_code="diagnostic.removed",
        changed_code="diagnostic.changed",
        page_for=lambda item_key, _before, _after: page_by_node.get(item_key[2]),
    )
    _compare_keyed(
        diffs,
        baseline_items=baseline["capabilities"],
        candidate_items=candidate["capabilities"],
        key=lambda item: item["key"],
        stage="contract",
        origin_type="capability",
        added_code="capability.added",
        removed_code="capability.removed",
        changed_code="capability.changed",
    )
    if baseline["fidelity"] != candidate["fidelity"]:
        _append(
            diffs,
            stage="contract",
            code="fidelity.changed",
            origin_type="document",
            origin_id=baseline["document_id"],
            before=baseline["fidelity"],
            after=candidate["fidelity"],
        )


def _render_index(evidence):
    evidence = evidence or {"pages": []}
    return _index(evidence.get("pages", []), lambda page: page["page_id"])


def _compare_render_evidence(diffs, baseline_evidence, candidate_evidence):
    b = _render_index(baseline_evidence)
    c = _render_index(candidate_evidence)
    for page_id in sorted(set(b) | set(c)):
        before = b.get(page_id)
        after = c.get(page_id)
        if before is None or after is None:
            _append(
                diffs,
                stage="render",
                code="render.page_added" if before is None else "render.page_removed",
                origin_type="page",
                origin_id=page_id,
                page_id=page_id,
                before=before,
                after=after,
            )
            continue
        if before.get("artifact_sha256") != after.get("artifact_sha256"):
            _append(
                diffs,
                stage="render",
                code="render.page_artifact_changed",
                origin_type="page",
                origin_id=page_id,
                page_id=page_id,
                before=before.get("artifact_sha256"),
                after=after.get("artifact_sha256"),
            )

        def region_key(region):
            bbox = tuple(region.get("bbox_px") or [])
            return (region.get("origin_node_id") or "", bbox)

        br = _index(before.get("regions", []), region_key)
        cr = _index(after.get("regions", []), region_key)
        for region_id in sorted(set(br) | set(cr), key=str):
            old = br.get(region_id)
            new = cr.get(region_id)
            if old == new:
                continue
            origin_node_id = (new or old).get("origin_node_id")
            _append(
                diffs,
                stage="render",
                code="render.region_changed",
                origin_type="node" if origin_node_id else "render_region",
                origin_id=origin_node_id or json.dumps(region_id[1]),
                page_id=page_id,
                before=old,
                after=new,
            )


def compare_scenes(
    baseline,
    candidate,
    *,
    baseline_render=None,
    candidate_render=None,
):
    for field in ("protocol_version", "document_id", "source_hash"):
        if baseline.get(field) != candidate.get(field):
            raise AssertionError(f"VISREG inputs are not comparable: {field} differs")
    if baseline.get("layout_environment") != candidate.get("layout_environment"):
        raise AssertionError("VISREG inputs require the same Layout Environment")

    diffs = []
    _compare_pages(diffs, baseline, candidate)
    _compare_nodes(diffs, baseline, candidate)
    _compare_stories(diffs, baseline, candidate)
    _compare_render_inputs(diffs, baseline, candidate)
    _compare_render_evidence(diffs, baseline_render, candidate_render)

    diffs.sort(
        key=lambda item: (
            STAGE_ORDER[item["stage"]],
            item.get("page_id") or "",
            item["origin_type"],
            item["origin_id"],
            item["code"],
            canonical_json(item["before"]),
            canonical_json(item["after"]),
        )
    )
    counts = {stage: 0 for stage in STAGE_ORDER}
    for item in diffs:
        counts[item["stage"]] += 1

    non_render_count = sum(
        count for stage, count in counts.items() if stage != "render"
    )
    report = {
        "report_version": REPORT_VERSION,
        "document_id": baseline["document_id"],
        "source_hash": baseline["source_hash"],
        "baseline_snapshot_id": baseline["snapshot_id"],
        "candidate_snapshot_id": candidate["snapshot_id"],
        "layout_environment_id": baseline["layout_environment"]["environment_id"],
        "summary": {
            "difference_count": len(diffs),
            "stage_counts": counts,
            "semantic_or_layout_change": non_render_count > 0,
            "render_only": bool(diffs) and non_render_count == 0,
            "equivalent": not diffs,
        },
        "differences": diffs,
    }
    report["report_hash"] = hash_id(report)
    return report
