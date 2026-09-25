#!/usr/bin/env python3
"""Atomic Chaptera-only CreateTextBox V1.

Creates one page-owned TextFrame + one Chaptera-created Story in one revision.
The operation is source-neutral: it never allocates Publisher/Quill identities,
never appends a terminal CR sentinel, and normalizes optional external text
exactly once through AUTHORING-TEXT-INGRESS-01.
"""

from __future__ import annotations

import copy
import hashlib
import json

from create_shape_v1 import validate_rect_emu_v1, validate_uuid7_node_id_v1
from paragraph_lifecycle_v1 import (
    ParagraphPropertiesV1,
    ParagraphV1,
    build_story_paragraph_state_v1,
)
from story_edit_transaction_v1 import (
    build_story_edit_core_state_v1,
    story_edit_core_state_to_dict,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    build_text_format_overlay_state_v1,
)
from text_ingress_v1 import normalize_external_text_v1


class CreateTextBoxError(ValueError):
    pass


PRESET_VERSION = "chaptera.authoring-text-preset.v1"
MAX_SAFE_EMU = 9_007_199_254_740_991


def _fail(message: str) -> None:
    raise CreateTextBoxError(message)


def _is_lower_sha256(value: object) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(ch in "0123456789abcdef" for ch in value)
    )


def validate_text_preset_v1(value: dict) -> None:
    expected = {
        "preset_version",
        "font_fingerprint",
        "face_index",
        "font_size_emu",
        "paragraph_defaults",
        "character_defaults",
    }
    if not isinstance(value, dict) or set(value) != expected:
        _fail("CreateTextBox text_preset must contain exact AuthoringTextPresetV1 fields")
    if value.get("preset_version") != PRESET_VERSION:
        _fail("CreateTextBox text_preset version is unsupported")
    if not _is_lower_sha256(value.get("font_fingerprint")):
        _fail("CreateTextBox text_preset font_fingerprint must be lowercase SHA-256")
    face_index = value.get("face_index")
    if not isinstance(face_index, int) or isinstance(face_index, bool) or face_index < 0:
        _fail("CreateTextBox text_preset face_index must be non-negative integer")
    size = value.get("font_size_emu")
    if (
        not isinstance(size, int)
        or isinstance(size, bool)
        or size <= 0
        or size > MAX_SAFE_EMU
    ):
        _fail("CreateTextBox text_preset font_size_emu must be positive safe EMU")

    paragraph = value.get("paragraph_defaults")
    if not isinstance(paragraph, dict) or set(paragraph) != {
        "alignment",
        "space_before_emu",
        "space_after_emu",
    }:
        _fail("CreateTextBox paragraph_defaults shape is invalid")
    if paragraph.get("alignment") not in {"left", "center", "right"}:
        _fail("CreateTextBox paragraph alignment is unsupported")
    for key in ("space_before_emu", "space_after_emu"):
        spacing = paragraph.get(key)
        if (
            not isinstance(spacing, int)
            or isinstance(spacing, bool)
            or spacing < 0
            or spacing > MAX_SAFE_EMU
        ):
            _fail(f"CreateTextBox paragraph_defaults.{key} must be non-negative safe EMU")

    character = value.get("character_defaults")
    if character is not None:
        if not isinstance(character, dict) or set(character) != {"bold", "italic"}:
            _fail("CreateTextBox character_defaults shape is invalid")
        if not isinstance(character.get("bold"), bool) or not isinstance(
            character.get("italic"), bool
        ):
            _fail("CreateTextBox character_defaults must be boolean")


def _preset_id(value: dict) -> str:
    validate_text_preset_v1(value)
    ordered = {
        "preset_version": value["preset_version"],
        "font_fingerprint": value["font_fingerprint"],
        "face_index": value["face_index"],
        "font_size_emu": value["font_size_emu"],
        "paragraph_defaults": {
            "alignment": value["paragraph_defaults"]["alignment"],
            "space_before_emu": value["paragraph_defaults"]["space_before_emu"],
            "space_after_emu": value["paragraph_defaults"]["space_after_emu"],
        },
    }
    if value["character_defaults"] is not None:
        ordered["character_defaults"] = {
            "bold": value["character_defaults"]["bold"],
            "italic": value["character_defaults"]["italic"],
        }
    payload = json.dumps(
        ordered,
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")
    digest = hashlib.sha256(
        b"chaptera-authoring-text-preset-id-v1\0" + payload
    ).hexdigest()
    return f"sha256:{digest}"


def authoring_text_preset_id_v1(value: dict) -> str:
    """Return the canonical identity of one validated AuthoringTextPresetV1."""
    return _preset_id(value)


def _base_format(value: dict) -> BaseCharacterFormatV1:
    character = value["character_defaults"] or {"bold": False, "italic": False}
    return BaseCharacterFormatV1(
        font_resource_id=(
            f"sha256:{value['font_fingerprint']}#face={value['face_index']}"
        ),
        font_size_emu=value["font_size_emu"],
        bold=character["bold"],
        italic=character["italic"],
        text_color_rgb="#000000",
    )


def _paragraph_properties(value: dict) -> ParagraphPropertiesV1:
    paragraph = value["paragraph_defaults"]
    return ParagraphPropertiesV1(
        (
            ("alignment", paragraph["alignment"]),
            ("space_after_emu", paragraph["space_after_emu"]),
            ("space_before_emu", paragraph["space_before_emu"]),
        )
    )


def validate_create_textbox_intent_v1(command: dict) -> None:
    expected = {
        "kind",
        "node_id",
        "story_id",
        "page_id",
        "bounds",
        "text_preset",
        "initial_text",
    }
    if not isinstance(command, dict) or set(command) != expected:
        _fail("CreateTextBox contains non-intent/authoritative fields")
    if command.get("kind") != "create_textbox":
        _fail("CreateTextBox kind is required")
    validate_uuid7_node_id_v1(command.get("node_id"))
    validate_uuid7_node_id_v1(command.get("story_id"))
    if command.get("node_id") == command.get("story_id"):
        _fail("CreateTextBox node_id and story_id must be distinct")
    page_id = command.get("page_id")
    if not isinstance(page_id, str) or not page_id:
        _fail("CreateTextBox page_id is required")
    validate_rect_emu_v1(command.get("bounds"), "CreateTextBox bounds")
    validate_text_preset_v1(command.get("text_preset"))
    initial_text = command.get("initial_text")
    if initial_text is not None and not isinstance(initial_text, str):
        _fail("CreateTextBox initial_text must be Unicode string or null")


def _story_state(story_id: str, text: str, preset: dict, preset_id: str) -> dict:
    props = _paragraph_properties(preset)
    paragraph_count = text.count("\r") + 1
    paragraphs = tuple(
        ParagraphV1(
            paragraph_id=f"{story_id}:paragraph:{index}",
            properties=props,
            provenance="chaptera_created",
        )
        for index in range(paragraph_count)
    )
    paragraph_state = build_story_paragraph_state_v1(
        story_id=story_id,
        story_text=text,
        paragraphs=paragraphs,
        protected_terminal_cr=False,
    )
    base_format = _base_format(preset)
    base_runs = (
        ()
        if not text
        else (BaseFormatRunV1(0, len(text), base_format),)
    )
    format_state = build_text_format_overlay_state_v1(
        story_id=story_id,
        base_revision_id=f"authoring-preset:{preset_id}",
        story_scalar_len=len(text),
        base_runs=base_runs,
    )
    core = build_story_edit_core_state_v1(
        story_id=story_id,
        provenance="chaptera_created",
        paragraph_state=paragraph_state,
        format_state=format_state,
        empty_story_preset_format=base_format if not text else None,
    )
    return story_edit_core_state_to_dict(core)


def apply_create_textbox_v1(base_project: dict, command: dict) -> tuple[dict, dict, list]:
    validate_create_textbox_intent_v1(command)
    project = copy.deepcopy(base_project)

    pages = project.get("pages")
    if not isinstance(pages, dict) or command["page_id"] not in pages:
        _fail("invalid_create_textbox_page")
    page = pages[command["page_id"]]
    if not isinstance(page, dict):
        _fail("canonical page entry must be object")
    if page.get("authoring_enabled") is False:
        _fail("create_textbox_page_not_authorable")
    children = page.setdefault("children", [])
    if not isinstance(children, list):
        _fail("canonical page children must be list")

    node_id = command["node_id"]
    story_id = command["story_id"]
    if node_id in children:
        _fail("create_textbox_node_id_collision")
    for registry_name in ("shapes", "text_frames", "picture_frames", "groups", "nodes"):
        registry = project.get(registry_name)
        if isinstance(registry, dict) and node_id in registry:
            _fail("create_textbox_node_id_collision")
    for registry_name in ("stories", "story_models"):
        registry = project.get(registry_name)
        if isinstance(registry, dict) and story_id in registry:
            _fail("create_textbox_story_id_collision")
    existing_frames = project.get("text_frames")
    if isinstance(existing_frames, dict) and any(
        isinstance(frame, dict) and frame.get("story_id") == story_id
        for frame in existing_frames.values()
    ):
        _fail("create_textbox_story_id_collision")

    initial_text = command["initial_text"]
    canonical_text = normalize_external_text_v1(
        "" if initial_text is None else initial_text
    ).text
    preset = copy.deepcopy(command["text_preset"])
    preset_id = _preset_id(preset)
    before_children = copy.deepcopy(children)
    after_children = before_children + [node_id]

    frame = {
        "node_id": node_id,
        "kind": "text_frame",
        "page_id": command["page_id"],
        "parent_id": command["page_id"],
        "story_id": story_id,
        "bounds": copy.deepcopy(command["bounds"]),
        "transform": {"kind": "identity"},
        "text_preset_id": preset_id,
        "provenance": {"kind": "author_created"},
    }
    text_frames = project.setdefault("text_frames", {})
    stories = project.setdefault("stories", {})
    story_models = project.setdefault("story_models", {})
    text_presets = project.setdefault("text_presets", {})
    for label, registry in (
        ("text_frames", text_frames),
        ("stories", stories),
        ("story_models", story_models),
        ("text_presets", text_presets),
    ):
        if not isinstance(registry, dict):
            _fail(f"canonical {label} registry must be object")

    text_frames[node_id] = frame
    stories[story_id] = canonical_text
    story_models[story_id] = _story_state(story_id, canonical_text, preset, preset_id)
    preset_record = {"preset_id": preset_id, "preset": preset}
    existing_preset = text_presets.get(preset_id)
    if existing_preset is not None and existing_preset != preset_record:
        _fail("create_textbox_preset_id_collision")
    text_presets[preset_id] = preset_record
    page["children"] = after_children

    operation = {
        "kind": "create_textbox",
        "node_id": node_id,
        "story_id": story_id,
        "page_id": command["page_id"],
        "parent_id": command["page_id"],
        "bounds": copy.deepcopy(command["bounds"]),
        "transform": {"kind": "identity"},
        "text_preset_id": preset_id,
        "text_preset": preset,
        "story_text": canonical_text,
        "page_children_before": before_children,
        "page_children_after": after_children,
        "provenance": {"kind": "author_created"},
    }
    operations = project.get("operations")
    if not isinstance(operations, list):
        _fail("canonical project operations must be list")
    operations.append(copy.deepcopy(operation))

    consequences = [
        {"key": "text_frame.created", "state": "supported", "note": None},
        {"key": "story.created", "state": "supported", "note": None},
        {"key": "layout.scene", "state": "invalidated", "note": None},
        {"key": "editable_export", "state": "invalidated", "note": None},
    ]
    return operation, project, consequences


def validate_create_textbox_operation_v1(command: dict, operation: dict) -> None:
    expected = {
        "kind",
        "node_id",
        "story_id",
        "page_id",
        "parent_id",
        "bounds",
        "transform",
        "text_preset_id",
        "text_preset",
        "story_text",
        "page_children_before",
        "page_children_after",
        "provenance",
    }
    if not isinstance(operation, dict) or set(operation) != expected:
        _fail("authoritative executor returned malformed CreateTextBox operation")
    if operation.get("kind") != "create_textbox":
        _fail("canonical CreateTextBox kind is invalid")
    for key in ("node_id", "story_id", "page_id", "bounds", "text_preset"):
        if operation.get(key) != command.get(key):
            _fail(f"canonical CreateTextBox {key} differs from accepted intent")
    if operation.get("parent_id") != command.get("page_id"):
        _fail("canonical CreateTextBox parent must equal accepted page")
    if operation.get("transform") != {"kind": "identity"}:
        _fail("canonical CreateTextBox transform must be identity")
    if operation.get("provenance") != {"kind": "author_created"}:
        _fail("canonical CreateTextBox provenance must be author_created")
    if operation.get("text_preset_id") != _preset_id(command["text_preset"]):
        _fail("canonical CreateTextBox preset identity mismatch")
    expected_text = normalize_external_text_v1(
        "" if command["initial_text"] is None else command["initial_text"]
    ).text
    if operation.get("story_text") != expected_text:
        _fail("canonical CreateTextBox Story text differs from one-pass ingress")
    before = operation.get("page_children_before")
    after = operation.get("page_children_after")
    if not isinstance(before, list) or after != before + [command["node_id"]]:
        _fail("canonical CreateTextBox page-child transition is invalid")
