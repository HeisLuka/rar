#!/usr/bin/env python3
"""Test-only canonical graph fixture for Chaptera-created Stories."""

from __future__ import annotations

import copy

from create_textbox_v1 import authoring_text_preset_id_v1


def authoring_text_preset_v1() -> dict:
    return {
        "preset_version": "chaptera.authoring-text-preset.v1",
        "font_fingerprint": "b" * 64,
        "face_index": 0,
        "font_size_emu": 12000,
        "paragraph_defaults": {
            "alignment": "left",
            "space_before_emu": 0,
            "space_after_emu": 0,
        },
        "character_defaults": {"bold": False, "italic": False},
    }


def bind_author_created_story_graph_v1(
    project: dict,
    *,
    story_id: str,
    frame_id: str,
    page_id: str,
    preset: dict | None = None,
) -> dict:
    """Attach the minimum canonical CreateTextBox-shaped ownership graph."""
    preset = copy.deepcopy(preset or authoring_text_preset_v1())
    preset_id = authoring_text_preset_id_v1(preset)

    pages = project.setdefault("pages", {})
    page = pages.setdefault(
        page_id,
        {
            "authoring_enabled": True,
            "children": [],
        },
    )
    children = page.setdefault("children", [])
    if frame_id not in children:
        children.append(frame_id)

    text_frames = project.setdefault("text_frames", {})
    text_frames[frame_id] = {
        "node_id": frame_id,
        "kind": "text_frame",
        "page_id": page_id,
        "parent_id": page_id,
        "story_id": story_id,
        "bounds": {
            "x": 0,
            "y": 0,
            "width": 1000000,
            "height": 500000,
        },
        "transform": {"kind": "identity"},
        "text_preset_id": preset_id,
        "provenance": {"kind": "author_created"},
    }

    text_presets = project.setdefault("text_presets", {})
    text_presets[preset_id] = {
        "preset_id": preset_id,
        "preset": preset,
    }
    return project
