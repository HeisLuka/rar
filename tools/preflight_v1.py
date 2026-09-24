#!/usr/bin/env python3
import copy
import hashlib
import json

SEVERITY = {"info": 0, "warning": 1, "error": 2}

def canonical_json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))

def hash_id(value):
    return "sha256:" + hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()

def _diag(code, severity, message_key, origin_node_id=None, detail=None):
    return {
        "code": code,
        "severity": severity,
        "message_key": message_key,
        "origin_node_id": origin_node_id,
        "detail": detail,
    }

def evaluate(scene, target_risks=None):
    target_risks = target_risks or []
    diagnostics = []

    for item in scene.get("diagnostics", []):
        diagnostics.append(_diag(
            item["code"],
            item["severity"],
            item["message_key"],
            item.get("origin_node_id"),
            "upstream_scene_diagnostic",
        ))

    nodes_by_resource = {}
    for node in scene.get("nodes", []):
        resource_id = node.get("resource_id")
        if resource_id:
            nodes_by_resource.setdefault(resource_id, []).append(node["node_id"])

    for resource in scene.get("resources", []):
        availability = resource.get("availability")
        if availability in {"missing", "blocked"}:
            origins = nodes_by_resource.get(resource["resource_id"]) or [None]
            for origin in origins:
                diagnostics.append(_diag(
                    "preflight.resource_missing",
                    "error",
                    "preflight.resource_missing",
                    origin,
                    f"{resource['resource_id']}:{availability}",
                ))
        elif availability in {"preview_only", "unsupported", "unknown"}:
            origins = nodes_by_resource.get(resource["resource_id"]) or [None]
            for origin in origins:
                diagnostics.append(_diag(
                    "preflight.resource_partial",
                    "warning",
                    "preflight.resource_partial",
                    origin,
                    f"{resource['resource_id']}:{availability}",
                ))

    for story in scene.get("stories", []):
        state = story.get("text_fidelity")
        if state in {"unsupported", "opaque"}:
            diagnostics.append(_diag(
                "preflight.story_semantics_opaque",
                "warning" if state == "opaque" else "error",
                "preflight.story_semantics_opaque",
                None,
                f"{story['story_id']}:{state}",
            ))

    for cap in scene.get("capabilities", []):
        if cap.get("state") != "supported":
            diagnostics.append(_diag(
                "preflight.capability_" + cap["state"],
                "warning",
                "preflight.capability_state",
                None,
                cap["key"],
            ))

    for risk in target_risks:
        diagnostics.append(_diag(
            risk["code"],
            risk["severity"],
            risk["message_key"],
            risk.get("origin_node_id"),
            risk.get("detail"),
        ))

    diagnostics.sort(key=lambda d: (
        SEVERITY[d["severity"]],
        d["code"],
        d.get("origin_node_id") or "",
        d["message_key"],
        d.get("detail") or "",
    ))
    counts = {level: sum(1 for d in diagnostics if d["severity"] == level) for level in SEVERITY}
    result = {
        "receipt_version": "chaptera.preflight.v1",
        "source_hash": scene["source_hash"],
        "revision_id": scene["revision_id"],
        "scene_snapshot_id": scene["snapshot_id"],
        "diagnostics": diagnostics,
        "summary": {
            "error_count": counts["error"],
            "warning_count": counts["warning"],
            "info_count": counts["info"],
            "blocking": counts["error"] > 0,
        },
    }
    result["receipt_id"] = hash_id(result)
    return result
