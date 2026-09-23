import copy
import hashlib
import json


def canonical_json(value):
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def hash_id(value):
    return "sha256:" + hashlib.sha256(canonical_json(value)).hexdigest()


def string_hash_id(value):
    return "sha256:" + hashlib.sha256(value.encode("utf-8")).hexdigest()


def severity_rank(value):
    return {"info": 0, "warning": 1, "error": 2}[value]


def normalize_snapshot(snapshot):
    value = copy.deepcopy(snapshot)
    value["pages"] = sorted(value["pages"], key=lambda x: (x["order"], x["page_id"]))
    page_order = {p["page_id"]: p["order"] for p in value["pages"]}
    value["nodes"] = sorted(
        value["nodes"],
        key=lambda x: (
            page_order.get(x["page_id"], 2**63 - 1),
            x["z_order"] is None,
            x["z_order"] if x["z_order"] is not None else 0,
            x["paint_order"] is None,
            x["paint_order"] if x["paint_order"] is not None else 0,
            x["node_id"],
        ),
    )
    value["stories"] = sorted(value["stories"], key=lambda x: x["story_id"])
    value["story_frames"] = sorted(
        value["story_frames"],
        key=lambda x: (x["story_id"], x["frame_ordinal"], x["node_id"]),
    )
    value["paints"] = sorted(value["paints"], key=lambda x: x["paint_id"])
    value["resources"] = sorted(value["resources"], key=lambda x: x["resource_id"])
    value["diagnostics"] = sorted(
        value["diagnostics"],
        key=lambda x: (
            severity_rank(x["severity"]),
            x["code"],
            x.get("origin_node_id") or "",
            x["message_key"],
        ),
    )
    value["capabilities"] = sorted(
        value["capabilities"],
        key=lambda x: (x["key"], x["state"], x.get("note") or ""),
    )
    value["fidelity"]["reasons"] = sorted(value["fidelity"]["reasons"])
    return value


def derive_snapshot_id(snapshot):
    value = normalize_snapshot(snapshot)
    value.pop("snapshot_id", None)
    return hash_id(value)


def finalize_snapshot(snapshot):
    value = normalize_snapshot(snapshot)
    value["snapshot_id"] = derive_snapshot_id(value)
    return normalize_snapshot(value)
