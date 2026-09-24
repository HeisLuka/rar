#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

from adapt_viewer_scene_v1 import adapt_viewer_geometry
from scene_v1 import canonical_json, derive_snapshot_id
from validate_scene_protocol import canonical_json as protocol_canonical_json, normalize as protocol_normalize

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCENE = ROOT / "packages" / "protocol" / "scene" / "v1"
VIEWER_FIXTURES = SCENE / "viewer-fixtures"
TARGET = ROOT / "target" / "scene-adapter"

DOC_ID = "90000000-0000-4000-8000-000000000001"
REVISION_ID = "sha256:" + "9" * 64
FORBIDDEN_KEYS = {
    "carrier", "source_ref", "byte_range", "cfb_path", "stream_name",
    "stream_path", "source_path", "filesystem_path", "raw_pub_bytes",
    "raw_bytes", "bytes", "parser_record",
}


def assert_true(value, message):
    if not value:
        raise AssertionError(message)


def walk_forbidden(value, at="$"):
    if isinstance(value, dict):
        for key, item in value.items():
            if key in FORBIDDEN_KEYS:
                raise AssertionError(f"forbidden field {key!r} at {at}")
            walk_forbidden(item, f"{at}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            walk_forbidden(item, f"{at}[{index}]")


def load(name):
    return json.loads((VIEWER_FIXTURES / name).read_text(encoding="utf-8"))


def validate(snapshot, schema_validator, label):
    errors = sorted(schema_validator.iter_errors(snapshot), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError(f"{label}: schema errors\n{detail}")
    assert_true(snapshot["snapshot_id"] == derive_snapshot_id(snapshot), f"{label}: wrong snapshot_id")
    walk_forbidden(snapshot)
    assert_true(
        protocol_canonical_json(protocol_normalize(snapshot)) == canonical_json(snapshot),
        f"{label}: adapter canonicalization diverges from protocol validator",
    )


def capability_map(snapshot):
    return {item["key"]: item for item in snapshot["capabilities"]}


def main():
    TARGET.mkdir(parents=True, exist_ok=True)
    schema = json.loads((SCENE / "snapshot.schema.json").read_text(encoding="utf-8"))
    schema_validator = Draft202012Validator(schema)

    receipt = {
        "receipt_kind": "synthetic_viewer_to_scene_adapter_validation",
        "protocol_version": "chaptera.scene.v1",
        "real_pub_measurement": False,
        "fixtures": [],
    }

    geometry_input = load("geometry-only.json")
    geometry = adapt_viewer_geometry(geometry_input, DOC_ID, REVISION_ID)
    geometry_again = adapt_viewer_geometry(geometry_input, DOC_ID, REVISION_ID)
    assert_true(canonical_json(geometry) == canonical_json(geometry_again), "geometry-only mapping is not deterministic")
    validate(geometry, schema_validator, "geometry-only")

    assert_true(
        [page["page_id"] for page in geometry["pages"]] == [
            "10000000-0000-4000-8000-000000000001",
            "10000000-0000-4000-8000-000000000002",
        ],
        "Viewer document order was not preserved",
    )
    nodes = {node["node_id"]: node for node in geometry["nodes"]}
    child = nodes["20000000-0000-4000-8000-000000000002"]
    assert_true(child["page_id"] == "10000000-0000-4000-8000-000000000001", "child node page ancestry lost")
    assert_true(child["parent_node_id"] == "20000000-0000-4000-8000-000000000001", "child node parent lost")
    assert_true(child["bounds"]["x"] == -12700, "signed EMU changed")
    assert_true(geometry["stacking_fidelity"] == "unknown", "adapter overclaimed stacking fidelity")
    assert_true(all(node["z_order"] is None for node in geometry["nodes"]), "adapter invented z_order")
    assert_true(all(node["paint_order"] is None for node in geometry["nodes"]), "adapter invented paint_order")
    assert_true(all(node["kind"] == "unknown" for node in geometry["nodes"]), "geometry-only adapter invented node kind")
    assert_true(geometry["story_frames"] == [], "geometry-only adapter invented Story/frame binding")
    caps = capability_map(geometry)
    assert_true(caps["render.geometry"]["state"] == "supported", "geometry capability wrong")
    assert_true(caps["render.stacking"]["state"] == "unsupported", "unknown stacking not surfaced")
    assert_true(caps["render.transforms"]["state"] == "supported", "exact transform capability wrong")
    assert_true(
        nodes["20000000-0000-4000-8000-000000000003"]["transform"]["tx"] == 12700,
        "exact non-identity transform translation was not preserved",
    )
    assert_true(caps["render.story-frame"]["state"] == "unsupported", "missing Story/frame binding not surfaced")
    assert_true(geometry["fidelity"]["state"] == "partial", "geometry-only payload overclaims fidelity")

    enriched_input = load("enriched-overlays.json")
    enriched = adapt_viewer_geometry(enriched_input, DOC_ID, REVISION_ID)
    validate(enriched, schema_validator, "enriched-overlays")

    enriched_nodes = {node["node_id"]: node for node in enriched["nodes"]}
    assert_true(enriched_nodes["21000000-0000-4000-8000-000000000001"]["kind"] == "text_frame", "StoryFrame did not classify text frame")
    assert_true(enriched_nodes["21000000-0000-4000-8000-000000000002"]["kind"] == "picture_frame", "image binding did not classify picture frame")
    assert_true(enriched_nodes["21000000-0000-4000-8000-000000000003"]["kind"] == "unknown", "unproven shape kind was invented")
    assert_true(len(enriched["story_frames"]) == 1, "StoryFrame mapping lost")
    assert_true(len(enriched["paints"]) == 1, "bounded paint mapping lost")
    assert_true(enriched["paints"][0]["fill"] == {"r": 32, "g": 96, "b": 192, "a": 255}, "fill RGB mapping wrong")
    assert_true(enriched["paints"][0]["stroke"]["width_emu"] == 12700, "line width mapping wrong")
    assert_true(len(enriched["resources"]) == 1, "image resource mapping lost")
    assert_true(enriched["resources"][0]["availability"] == "unknown", "adapter overclaimed serialized image delivery")
    assert_true(enriched["resources"][0]["content_hash"] is None, "adapter invented resource hash")
    enriched_caps = capability_map(enriched)
    assert_true(enriched_caps["render.paint"]["state"] == "partial", "bounded paint capability wrong")
    assert_true(enriched_caps["resource.image"]["state"] == "partial", "image capability wrong")
    assert_true(enriched_caps["render.story-frame"]["state"] == "partial", "StoryFrame capability wrong")
    assert_true(enriched["stacking_fidelity"] == "unknown", "enriched adapter overclaimed stacking")
    assert_true(
        enriched_nodes["21000000-0000-4000-8000-000000000001"]["transform"]
        == {"a": "1", "b": "0", "c": "0", "d": "1", "tx": 0, "ty": 0},
        "exact identity transform changed",
    )
    assert_true(enriched["fidelity"]["state"] == "partial", "enriched overlay still must remain Partial")

    for name, snapshot in [("geometry-only", geometry), ("enriched-overlays", enriched)]:
        encoded = canonical_json(snapshot)
        output = TARGET / f"{name}.scene-v1.json"
        output.write_bytes(encoded + b"\n")
        receipt["fixtures"].append({
            "name": name,
            "source_hash": snapshot["source_hash"],
            "pages": len(snapshot["pages"]),
            "nodes": len(snapshot["nodes"]),
            "stories": len(snapshot["stories"]),
            "canonical_bytes": len(encoded),
            "snapshot_id": snapshot["snapshot_id"],
        })

    (TARGET / "receipt.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"scene adapter validation failed: {exc}", file=sys.stderr)
        raise
