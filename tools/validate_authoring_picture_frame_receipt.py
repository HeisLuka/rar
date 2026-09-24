#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "packages" / "protocol" / "authoring-picture-frame" / "v1" / "producer-receipt.schema.json"


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("picture-frame receipt schema validation failed\n" + detail)


def validate_semantics(receipt):
    states = receipt["states"]
    baseline = states["baseline"]
    cropped = states["after_crop"]
    undo = states["crop_undo"]
    redo = states["crop_redo"]
    replaced = states["after_replace"]
    moved = states["after_move"]
    resized = states["after_resize"]
    replay = states["replay"]

    if cropped["frame_state_hash"] != baseline["frame_state_hash"]:
        raise AssertionError("SetImageCrop changed frame geometry")
    if cropped["asset_binding_id"] != baseline["asset_binding_id"]:
        raise AssertionError("SetImageCrop changed asset identity")
    if cropped["crop_state_hash"] == baseline["crop_state_hash"]:
        raise AssertionError("SetImageCrop did not change crop state")

    if undo != baseline:
        raise AssertionError("crop undo did not restore exact pre-state")
    if redo != cropped:
        raise AssertionError("crop redo did not restore exact post-state")

    if replaced["frame_state_hash"] != cropped["frame_state_hash"]:
        raise AssertionError("ReplaceImage changed frame geometry")
    if replaced["crop_state_hash"] != cropped["crop_state_hash"]:
        raise AssertionError("ReplaceImage changed crop state")
    if replaced["asset_binding_id"] == cropped["asset_binding_id"]:
        raise AssertionError("ReplaceImage did not change asset identity")

    if moved["asset_binding_id"] != replaced["asset_binding_id"] or moved["crop_state_hash"] != replaced["crop_state_hash"]:
        raise AssertionError("MoveNode rewrote asset/crop state")
    if moved["frame_state_hash"] == replaced["frame_state_hash"]:
        raise AssertionError("MoveNode did not change frame state")

    if resized["asset_binding_id"] != moved["asset_binding_id"] or resized["crop_state_hash"] != moved["crop_state_hash"]:
        raise AssertionError("ResizeNode rewrote asset/crop state")
    if resized["frame_state_hash"] == moved["frame_state_hash"]:
        raise AssertionError("ResizeNode did not change frame state")

    if replay != resized:
        raise AssertionError("EditorProject replay did not reproduce frame/asset/crop tuple")

    outputs = receipt["output_probe"]
    if outputs["silent_uncropped_source_fallback"]:
        raise AssertionError("target silently fell back to uncropped source image")
    if outputs["silent_old_asset_fallback"]:
        raise AssertionError("target silently fell back to old source asset")

    privacy = receipt["privacy"]
    if not privacy["source_pub_unchanged"] or privacy["source_write_count"] != 0:
        raise AssertionError("native source mutation is outside this task")
    if privacy["fit_pan_synthesized"]:
        raise AssertionError("task must not invent Fit/Fill/Pan state")
    if privacy["raw_source_bytes_in_receipt"] or privacy["raw_asset_bytes_in_receipt"]:
        raise AssertionError("public receipt leaks raw bytes")

    return {
        "receipt_kind": receipt["receipt_version"],
        "fixture_kind": receipt["fixture_kind"],
        "set_crop_changes_only_crop": True,
        "crop_undo_exact": True,
        "crop_redo_exact": True,
        "replace_image_preserves_frame_crop": True,
        "move_preserves_asset_crop": True,
        "resize_preserves_asset_crop": True,
        "replay_exact": True,
        "silent_uncropped_source_fallback": False,
        "silent_old_asset_fallback": False,
        "source_pub_unchanged": True,
    }


def main():
    if len(sys.argv) != 2:
        print("usage: validate_authoring_picture_frame_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    receipt = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
    validate_schema(receipt)
    print(json.dumps(validate_semantics(receipt), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
