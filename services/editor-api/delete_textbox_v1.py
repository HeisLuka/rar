#!/usr/bin/env python3
"""Canonical V1 deletion of one exclusive author-created TextBox + Story."""

from __future__ import annotations

import copy

from create_shape_v1 import validate_uuid7_node_id_v1


class DeleteTextBoxError(ValueError):
    pass


_COMMAND_KEYS = {
    "kind",
    "node_id",
    "story_id",
    "expected_frame",
    "expected_story_text",
    "expected_story_model",
    "expected_text_preset_record",
    "expected_page_children",
    "expected_page_child_index",
}


def _fail(message: str) -> None:
    raise DeleteTextBoxError(message)


def validate_delete_textbox_intent_v1(command: object) -> None:
    if not isinstance(command, dict) or set(command) != _COMMAND_KEYS:
        _fail("DeleteTextBox command fields mismatch")
    if command.get("kind") != "delete_textbox":
        _fail("DeleteTextBox kind is required")
    validate_uuid7_node_id_v1(command.get("node_id"))
    validate_uuid7_node_id_v1(command.get("story_id"))
    if command["node_id"] == command["story_id"]:
        _fail("DeleteTextBox node_id and story_id must be distinct")
    if not isinstance(command.get("expected_frame"), dict):
        _fail("DeleteTextBox expected_frame is required")
    if not isinstance(command.get("expected_story_text"), str):
        _fail("DeleteTextBox expected_story_text must be Unicode string")
    if not isinstance(command.get("expected_story_model"), dict):
        _fail("DeleteTextBox expected_story_model is required")
    if not isinstance(command.get("expected_text_preset_record"), dict):
        _fail("DeleteTextBox expected_text_preset_record is required")
    children = command.get("expected_page_children")
    index = command.get("expected_page_child_index")
    if not isinstance(children, list):
        _fail("DeleteTextBox expected_page_children must be list")
    if isinstance(index, bool) or not isinstance(index, int) or index < 0:
        _fail("DeleteTextBox expected_page_child_index must be non-negative integer")
    if index >= len(children) or children[index] != command["node_id"]:
        _fail("DeleteTextBox expected page child position does not name target")
    if children.count(command["node_id"]) != 1:
        _fail("DeleteTextBox target must occur exactly once in expected page children")


def _validate_owned_unit(project: dict, command: dict) -> tuple[str, str]:
    node_id = command["node_id"]
    story_id = command["story_id"]

    frames = project.get("text_frames")
    if not isinstance(frames, dict):
        _fail("canonical text_frames registry missing")
    frame = frames.get(node_id)
    if frame != command["expected_frame"]:
        _fail("delete_textbox_stale_frame")
    if (
        frame.get("node_id") != node_id
        or frame.get("kind") != "text_frame"
        or frame.get("story_id") != story_id
        or frame.get("provenance") != {"kind": "author_created"}
    ):
        _fail("delete_textbox_target_not_author_created")
    page_id = frame.get("page_id")
    if not isinstance(page_id, str) or not page_id or frame.get("parent_id") != page_id:
        _fail("delete_textbox_target_not_direct_page_owned")

    owners = [
        actual_id
        for actual_id, actual_frame in frames.items()
        if isinstance(actual_frame, dict) and actual_frame.get("story_id") == story_id
    ]
    if owners != [node_id]:
        _fail("delete_textbox_story_not_exclusively_owned")

    stories = project.get("stories")
    if (
        not isinstance(stories, dict)
        or story_id not in stories
        or stories[story_id] != command["expected_story_text"]
    ):
        _fail("delete_textbox_stale_story_text")

    story_models = project.get("story_models")
    model = story_models.get(story_id) if isinstance(story_models, dict) else None
    if model != command["expected_story_model"]:
        _fail("delete_textbox_stale_story_model")
    if (
        model.get("story_id") != story_id
        or model.get("provenance") != "chaptera_created"
        or model.get("paragraph_state", {}).get("story_text")
        != command["expected_story_text"]
    ):
        _fail("delete_textbox_story_model_not_author_created")
    unsupported = model.get("unsupported_anchored_semantics")
    if unsupported not in (None, [], ()):
        _fail("delete_textbox_story_has_unsupported_semantics")

    preset_id = frame.get("text_preset_id")
    if not isinstance(preset_id, str) or not preset_id:
        _fail("delete_textbox_text_preset_missing")
    presets = project.get("text_presets")
    preset_record = presets.get(preset_id) if isinstance(presets, dict) else None
    if preset_record != command["expected_text_preset_record"]:
        _fail("delete_textbox_stale_text_preset")
    if (
        preset_record.get("preset_id") != preset_id
        or not isinstance(preset_record.get("preset"), dict)
    ):
        _fail("delete_textbox_text_preset_malformed")

    pages = project.get("pages")
    page = pages.get(page_id) if isinstance(pages, dict) else None
    if not isinstance(page, dict):
        _fail("delete_textbox_page_missing")
    children = page.get("children")
    if children != command["expected_page_children"]:
        _fail("delete_textbox_stale_page_children")
    index = command["expected_page_child_index"]
    if index >= len(children) or children[index] != node_id or children.count(node_id) != 1:
        _fail("delete_textbox_page_child_position_mismatch")

    return page_id, preset_id


def apply_delete_textbox_v1(base_project: dict, command: dict) -> tuple[dict, dict, list]:
    validate_delete_textbox_intent_v1(command)
    project = copy.deepcopy(base_project)
    page_id, preset_id = _validate_owned_unit(project, command)

    before_children = copy.deepcopy(command["expected_page_children"])
    index = command["expected_page_child_index"]
    after_children = before_children[:index] + before_children[index + 1 :]

    frame = copy.deepcopy(project["text_frames"][command["node_id"]])
    story_text = project["stories"][command["story_id"]]
    story_model = copy.deepcopy(project["story_models"][command["story_id"]])
    preset_record = copy.deepcopy(project["text_presets"][preset_id])

    del project["text_frames"][command["node_id"]]
    del project["stories"][command["story_id"]]
    del project["story_models"][command["story_id"]]
    project["pages"][page_id]["children"] = after_children

    operation = {
        "kind": "delete_textbox",
        "node_id": command["node_id"],
        "story_id": command["story_id"],
        "page_id": page_id,
        "page_child_index": index,
        "deleted_frame": frame,
        "deleted_story_text": story_text,
        "deleted_story_model": story_model,
        "text_preset_id": preset_id,
        "text_preset_record": preset_record,
        "page_children_before": before_children,
        "page_children_after": after_children,
        "provenance": {"kind": "author_created"},
    }

    operations = project.get("operations")
    if not isinstance(operations, list):
        _fail("canonical project operations must be list")
    operations.append(copy.deepcopy(operation))

    consequences = [
        {"key": "text_frame.deleted", "state": "supported", "note": None},
        {"key": "story.deleted", "state": "supported", "note": None},
        {"key": "layout.scene", "state": "invalidated", "note": None},
        {"key": "editable_export", "state": "invalidated", "note": None},
    ]
    return operation, project, consequences


def validate_delete_textbox_operation_v1(command: dict, operation: dict) -> None:
    expected = {
        "kind",
        "node_id",
        "story_id",
        "page_id",
        "page_child_index",
        "deleted_frame",
        "deleted_story_text",
        "deleted_story_model",
        "text_preset_id",
        "text_preset_record",
        "page_children_before",
        "page_children_after",
        "provenance",
    }
    if not isinstance(operation, dict) or set(operation) != expected:
        _fail("authoritative executor returned malformed DeleteTextBox operation")
    if operation.get("kind") != "delete_textbox":
        _fail("canonical DeleteTextBox kind is invalid")
    if operation.get("node_id") != command.get("node_id"):
        _fail("canonical DeleteTextBox node_id differs from accepted intent")
    if operation.get("story_id") != command.get("story_id"):
        _fail("canonical DeleteTextBox story_id differs from accepted intent")
    frame = command["expected_frame"]
    if operation.get("page_id") != frame.get("page_id"):
        _fail("canonical DeleteTextBox page differs from expected frame")
    if operation.get("page_child_index") != command.get("expected_page_child_index"):
        _fail("canonical DeleteTextBox child index differs from precondition")
    if operation.get("deleted_frame") != frame:
        _fail("canonical DeleteTextBox frame snapshot differs from precondition")
    if operation.get("deleted_story_text") != command.get("expected_story_text"):
        _fail("canonical DeleteTextBox Story snapshot differs from precondition")
    if operation.get("deleted_story_model") != command.get("expected_story_model"):
        _fail("canonical DeleteTextBox Story model differs from precondition")
    if operation.get("text_preset_record") != command.get("expected_text_preset_record"):
        _fail("canonical DeleteTextBox preset record differs from precondition")
    if operation.get("text_preset_id") != frame.get("text_preset_id"):
        _fail("canonical DeleteTextBox preset identity differs from frame")
    before = command["expected_page_children"]
    index = command["expected_page_child_index"]
    expected_after = before[:index] + before[index + 1 :]
    if operation.get("page_children_before") != before:
        _fail("canonical DeleteTextBox page children before differ from precondition")
    if operation.get("page_children_after") != expected_after:
        _fail("canonical DeleteTextBox page children after are invalid")
    if operation.get("provenance") != {"kind": "author_created"}:
        _fail("canonical DeleteTextBox provenance must be author_created")
