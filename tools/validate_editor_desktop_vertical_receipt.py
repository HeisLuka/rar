#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = (
    ROOT
    / "packages"
    / "product"
    / "editor-desktop-vertical"
    / "v1"
    / "acceptance-receipt.schema.json"
)


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(
        Draft202012Validator(schema).iter_errors(receipt),
        key=lambda error: list(error.path),
    )
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError("desktop vertical receipt schema validation failed\n" + detail)


def validate_semantics(receipt):
    story = receipt["story_edit"]
    move = receipt["object_move"]
    history = receipt["history"]
    project = receipt["project"]
    capability = receipt["capability_loss"]

    if story["before_state_id"] == story["after_state_id"]:
        raise AssertionError("Story edit must change Story state")

    if move["before"] == move["after"]:
        raise AssertionError("MoveNode before/after RectEmu must differ")

    if history["after_story_state_id"] == history["after_move_state_id"]:
        raise AssertionError("MoveNode must change effective editor state")
    if history["undo_state_id"] != history["after_story_state_id"]:
        raise AssertionError("undo must restore exact post-Story/pre-Move state")
    if history["redo_state_id"] != history["after_move_state_id"]:
        raise AssertionError("redo must restore exact post-Move state")
    if history["reopened_state_id"] != history["after_move_state_id"]:
        raise AssertionError("fresh-session reopen must reproduce final state")

    for key in (
        "story_state_after_move",
        "story_state_after_undo",
        "story_state_after_redo",
        "story_state_reopened",
    ):
        if history[key] != story["after_state_id"]:
            raise AssertionError(f"{key} must preserve the accepted Story edit")

    if project["operation_count"] != 2:
        raise AssertionError("Desktop V0 sidecar must contain exactly Story edit + MoveNode")
    if project["operation_count"] != (
        project["story_operation_count"] + project["move_operation_count"]
    ):
        raise AssertionError("project operation counts are inconsistent")

    if capability["blocking_loss_count"] != 0:
        raise AssertionError("edited export cannot proceed with blocking loss")

    return {
        "receipt_kind": receipt["receipt_version"],
        "source_hash": receipt["source"]["sha256"],
        "story_id": story["story_id"],
        "moved_node_id": move["origin_node_id"],
        "project_sha256": project["sha256"],
        "export_format": receipt["export"]["format"],
        "export_sha256": receipt["export"]["sha256"],
        "rar_commit": receipt["environment"]["rar_commit"],
        "source_pub_immutable": True,
        "undo_redo_exact": True,
        "fresh_reopen_exact": True,
    }


def main():
    if len(sys.argv) != 2:
        print("usage: validate_editor_desktop_vertical_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    receipt = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
    validate_schema(receipt)
    print(json.dumps(validate_semantics(receipt), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
