#!/usr/bin/env python3
import copy
import hashlib
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCENE = ROOT / "packages" / "protocol" / "scene" / "v1"
FIXTURES = SCENE / "fixtures"
TARGET = ROOT / "target" / "scene-protocol"
FORBIDDEN_KEYS = {
    "carrier", "source_ref", "byte_range", "cfb_path", "stream_name",
    "stream_path", "source_path", "filesystem_path", "raw_pub_bytes",
    "raw_bytes", "bytes", "parser_record",
}


def canonical_json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def hash_id(value):
    return "sha256:" + hashlib.sha256(canonical_json(value)).hexdigest()


def severity_rank(value):
    return {"info": 0, "warning": 1, "error": 2}[value]


def normalize(snapshot):
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
    without_id = copy.deepcopy(value)
    without_id.pop("snapshot_id", None)
    value["snapshot_id"] = hash_id(without_id)
    return value


def walk_forbidden(value, at="$"):
    if isinstance(value, dict):
        for key, item in value.items():
            if key in FORBIDDEN_KEYS:
                raise AssertionError(f"forbidden key {key!r} at {at}")
            walk_forbidden(item, f"{at}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            walk_forbidden(item, f"{at}[{index}]")


def unique(items, key, fixture):
    values = [item[key] for item in items]
    if len(values) != len(set(values)):
        raise AssertionError(f"{fixture}: duplicate {key}")


def check_refs(snapshot, fixture):
    pages = {x["page_id"] for x in snapshot["pages"]}
    nodes = {x["node_id"] for x in snapshot["nodes"]}
    stories = {x["story_id"] for x in snapshot["stories"]}
    paints = {x["paint_id"] for x in snapshot["paints"]}
    resources = {x["resource_id"] for x in snapshot["resources"]}

    unique(snapshot["pages"], "page_id", fixture)
    unique(snapshot["nodes"], "node_id", fixture)
    unique(snapshot["stories"], "story_id", fixture)
    unique(snapshot["paints"], "paint_id", fixture)
    unique(snapshot["resources"], "resource_id", fixture)
    unique(snapshot["capabilities"], "key", fixture)

    frame_ordinals = set()
    for node in snapshot["nodes"]:
        if node["page_id"] not in pages:
            raise AssertionError(f"{fixture}: node references unknown page")
        if node.get("parent_node_id") is not None and node["parent_node_id"] not in nodes:
            raise AssertionError(f"{fixture}: node references unknown parent")
        if node.get("paint_id") is not None and node["paint_id"] not in paints:
            raise AssertionError(f"{fixture}: node references unknown paint")
        if node.get("resource_id") is not None and node["resource_id"] not in resources:
            raise AssertionError(f"{fixture}: node references unknown resource")
        transform = node["transform"]
        for scalar in ("a", "b", "c", "d"):
            if not isinstance(transform[scalar], str):
                raise AssertionError(f"{fixture}: transform {scalar} must be an exact decimal string")

    if snapshot["stacking_fidelity"] == "exact":
        if any(node["z_order"] is None or node["paint_order"] is None for node in snapshot["nodes"]):
            raise AssertionError(f"{fixture}: exact stacking requires concrete z_order and paint_order")

    for frame in snapshot["story_frames"]:
        if frame["story_id"] not in stories:
            raise AssertionError(f"{fixture}: story frame references unknown Story")
        if frame["node_id"] not in nodes:
            raise AssertionError(f"{fixture}: story frame references unknown node")
        token = (frame["story_id"], frame["frame_ordinal"])
        if token in frame_ordinals:
            raise AssertionError(f"{fixture}: duplicate Story frame ordinal")
        frame_ordinals.add(token)


def main():
    TARGET.mkdir(parents=True, exist_ok=True)

    schema = json.loads((SCENE / "snapshot.schema.json").read_text(encoding="utf-8"))
    delta_schema = json.loads((SCENE / "delta.schema.json").read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    Draft202012Validator.check_schema(delta_schema)
    validator = Draft202012Validator(schema)

    fixture_paths = sorted(FIXTURES.glob("*.json"))
    if len(fixture_paths) < 4:
        raise AssertionError("expected at least four standalone golden snapshot fixtures")

    receipt = {
        "receipt_kind": "synthetic_protocol_fixture_validation",
        "protocol_version": "chaptera.scene.v1",
        "real_pub_measurement": False,
        "fixtures": [],
    }
    coverage = {
        "negative_or_offpage_emu": False,
        "story_text": False,
        "exact_resource": False,
        "group_or_table": False,
        "explicit_unsupported": False,
    }

    for path in fixture_paths:
        fixture = path.stem
        original_bytes = path.read_bytes()
        snapshot = json.loads(original_bytes.decode("utf-8"))

        normalized = normalize(snapshot)
        if canonical_json(snapshot) != canonical_json(normalized):
            raise AssertionError(f"{fixture}: golden fixture is not normalized or has wrong snapshot_id")

        errors = sorted(validator.iter_errors(snapshot), key=lambda e: list(e.path))
        if errors:
            detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
            raise AssertionError(f"{fixture}: schema errors\n{detail}")

        walk_forbidden(snapshot)
        check_refs(snapshot, fixture)

        coverage["negative_or_offpage_emu"] |= any(
            node["bounds"]["x"] < 0 or node["bounds"]["y"] < 0
            for node in snapshot["nodes"]
        )
        coverage["story_text"] |= any(story["text"] for story in snapshot["stories"])
        coverage["exact_resource"] |= any(
            resource["availability"] == "available"
            and resource.get("content_hash") is not None
            for resource in snapshot["resources"]
        )
        coverage["group_or_table"] |= any(
            node["kind"] in {"group", "table"}
            for node in snapshot["nodes"]
        )
        coverage["explicit_unsupported"] |= (
            any(cap["state"] == "unsupported" for cap in snapshot["capabilities"])
            or bool(snapshot["diagnostics"])
        )

        encoded = canonical_json(snapshot)
        (TARGET / f"{fixture}.snapshot.json").write_bytes(encoded + b"\n")
        receipt["fixtures"].append({
            "name": fixture,
            "snapshot_id": snapshot["snapshot_id"],
            "source_file_bytes": len(original_bytes),
            "canonical_bytes": len(encoded),
        })

    missing = [name for name, present in coverage.items() if not present]
    if missing:
        raise AssertionError("fixture coverage missing: " + ", ".join(missing))

    receipt["coverage"] = coverage
    (TARGET / "receipt.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"scene protocol validation failed: {exc}", file=sys.stderr)
        raise
