#!/usr/bin/env python3
"""Desktop active-Story exact-literal Find/Replace UI controller V1.

This module is deliberately a thin product consumer. It owns panel state and
request lowering only; canonical search, programmatic selection/reveal, and
mutation remain owned by TextFindSnapshotV1, TextProgrammaticJumpV1, and
StoryFindReplaceV1.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from story_edit_domain_v1 import StoryEditDomainV1
from text_find_snapshot_v1 import (
    TextFindExtentV1,
    TextFindSnapshotV1,
    build_text_find_snapshot_v1,
    find_next_v1,
    find_previous_v1,
)
from text_programmatic_jump_v1 import TextProgrammaticJumpRequestV1


PanelFocusV1 = Literal["story", "find_field", "replace_field", "modal"]


class EditorTextFindReplaceUIError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class EditorTextFindReplacePanelV1:
    protocol_version: Literal["chaptera.editor-text-find-replace-panel.v1"]
    document_id: str
    story_id: str
    revision_id: str
    find_text: str
    replacement_text: str
    snapshot: TextFindSnapshotV1 | None
    current_ordinal: int | None
    wrap_navigation: bool
    panel_focus: PanelFocusV1
    composition_active: bool
    status: Literal["ready", "empty_query", "stale", "unsupported"]
    status_reason: str | None

    @property
    def result_count(self) -> int:
        return 0 if self.snapshot is None else len(self.snapshot.matches)

    @property
    def replace_enabled(self) -> bool:
        return (
            self.snapshot is not None
            and bool(self.snapshot.matches)
            and self.status == "ready"
            and not self.composition_active
        )


def _fail(code: str, message: str) -> None:
    raise EditorTextFindReplaceUIError(code, message)


def open_editor_text_find_replace_panel_v1(
    *,
    document_id: str,
    revision_id: str,
    story_id: str,
    story_text: str,
    domain: StoryEditDomainV1,
    find_text: str = "",
    replacement_text: str = "",
    wrap_navigation: bool = True,
) -> EditorTextFindReplacePanelV1:
    for label, value in (
        ("document_id", document_id),
        ("revision_id", revision_id),
        ("story_id", story_id),
    ):
        if not isinstance(value, str) or not value:
            _fail("invalid_panel_context", f"{label} is required")
    if domain.story_id != story_id:
        _fail("invalid_panel_context", "Story edit domain targets a different Story")

    panel = EditorTextFindReplacePanelV1(
        protocol_version="chaptera.editor-text-find-replace-panel.v1",
        document_id=document_id,
        story_id=story_id,
        revision_id=revision_id,
        find_text=find_text,
        replacement_text=replacement_text,
        snapshot=None,
        current_ordinal=None,
        wrap_navigation=bool(wrap_navigation),
        panel_focus="find_field",
        composition_active=False,
        status="empty_query" if find_text == "" else "ready",
        status_reason=None,
    )
    if find_text == "":
        return panel
    return refresh_editor_text_find_snapshot_v1(
        panel=panel,
        revision_id=revision_id,
        story_text=story_text,
        domain=domain,
    )


def set_find_text_v1(
    *,
    panel: EditorTextFindReplacePanelV1,
    find_text: str,
    story_text: str,
    domain: StoryEditDomainV1,
) -> EditorTextFindReplacePanelV1:
    if not isinstance(find_text, str):
        _fail("invalid_find_text", "find_text must be string")
    updated = replace(
        panel,
        find_text=find_text,
        current_ordinal=None,
        panel_focus="find_field",
        status="empty_query" if find_text == "" else "ready",
        status_reason=None,
    )
    if find_text == "":
        return replace(updated, snapshot=None)
    return refresh_editor_text_find_snapshot_v1(
        panel=updated,
        revision_id=panel.revision_id,
        story_text=story_text,
        domain=domain,
    )


def set_replacement_text_v1(
    panel: EditorTextFindReplacePanelV1,
    replacement_text: str,
) -> EditorTextFindReplacePanelV1:
    if not isinstance(replacement_text, str):
        _fail("invalid_replacement_text", "replacement_text must be string")
    return replace(
        panel,
        replacement_text=replacement_text,
        panel_focus="replace_field",
    )


def set_panel_input_state_v1(
    panel: EditorTextFindReplacePanelV1,
    *,
    panel_focus: PanelFocusV1,
    composition_active: bool,
) -> EditorTextFindReplacePanelV1:
    if panel_focus not in {"story", "find_field", "replace_field", "modal"}:
        _fail("invalid_panel_focus", "unsupported panel focus owner")
    if not isinstance(composition_active, bool):
        _fail("invalid_composition_state", "composition_active must be boolean")
    return replace(
        panel,
        panel_focus=panel_focus,
        composition_active=composition_active,
    )


def refresh_editor_text_find_snapshot_v1(
    *,
    panel: EditorTextFindReplacePanelV1,
    revision_id: str,
    story_text: str,
    domain: StoryEditDomainV1,
) -> EditorTextFindReplacePanelV1:
    if panel.find_text == "":
        return replace(
            panel,
            revision_id=revision_id,
            snapshot=None,
            current_ordinal=None,
            status="empty_query",
            status_reason=None,
        )
    try:
        snapshot = build_text_find_snapshot_v1(
            revision_id=revision_id,
            story_id=panel.story_id,
            story_text=story_text,
            domain=domain,
            external_query=panel.find_text,
            extent=TextFindExtentV1("full_editable_story"),
        )
    except ValueError as exc:
        return replace(
            panel,
            revision_id=revision_id,
            snapshot=None,
            current_ordinal=None,
            status="unsupported",
            status_reason=str(exc),
        )
    return replace(
        panel,
        revision_id=revision_id,
        snapshot=snapshot,
        current_ordinal=None,
        status="ready",
        status_reason=None,
    )


def mark_find_panel_stale_v1(
    panel: EditorTextFindReplacePanelV1,
    *,
    reason: str,
) -> EditorTextFindReplacePanelV1:
    return replace(
        panel,
        snapshot=None,
        current_ordinal=None,
        status="stale",
        status_reason=reason,
    )


def _require_snapshot(panel: EditorTextFindReplacePanelV1) -> TextFindSnapshotV1:
    if panel.status != "ready" or panel.snapshot is None:
        _fail("find_snapshot_unavailable", panel.status_reason or "current Find snapshot is unavailable")
    return panel.snapshot


def build_find_navigation_jump_v1(
    *,
    panel: EditorTextFindReplacePanelV1,
    direction: Literal["next", "previous"],
    navigation_origin: int,
) -> tuple[EditorTextFindReplacePanelV1, TextProgrammaticJumpRequestV1 | None]:
    if panel.panel_focus in {"find_field", "replace_field", "modal"}:
        _fail("shortcut_owned_by_panel", "Find/Replace field or modal owns text input")
    if panel.composition_active:
        _fail("composition_active", "active Story IME composition must resolve before Find navigation")

    snapshot = _require_snapshot(panel)
    if direction == "next":
        match = find_next_v1(
            snapshot=snapshot,
            navigation_origin=navigation_origin,
            wrap=panel.wrap_navigation,
        )
    elif direction == "previous":
        match = find_previous_v1(
            snapshot=snapshot,
            navigation_origin=navigation_origin,
            wrap=panel.wrap_navigation,
        )
    else:
        _fail("invalid_find_direction", "direction must be next or previous")

    if match is None:
        return replace(panel, current_ordinal=None), None

    jump = TextProgrammaticJumpRequestV1(
        protocol_version="chaptera.text-programmatic-jump.v1",
        document_id=panel.document_id,
        revision_id=panel.revision_id,
        story_id=panel.story_id,
        start_scalar=match.start_scalar,
        end_scalar=match.end_scalar,
        selection_mode="exact_range",
        reason="find_result",
    )
    return replace(panel, current_ordinal=match.ordinal), jump


def build_story_find_replace_request_v1(
    *,
    panel: EditorTextFindReplacePanelV1,
    mode: Literal["current", "all"],
    base_revision_id: str,
    client_operation_id: str,
    source_hash: str,
    paragraph_ids_by_match: list[dict],
    format_generation_id: str,
) -> dict | None:
    if panel.composition_active:
        _fail("composition_active", "active Story IME composition must resolve before replacement")
    snapshot = _require_snapshot(panel)
    if not snapshot.matches:
        return None

    if mode == "current":
        if panel.current_ordinal is None:
            _fail("current_match_unset", "Replace Current requires a selected snapshot ordinal")
        ordinals = [panel.current_ordinal]
    elif mode == "all":
        ordinals = [match.ordinal for match in snapshot.matches]
    else:
        _fail("invalid_replace_mode", "replace mode must be current or all")

    if base_revision_id != panel.revision_id or snapshot.revision_id != panel.revision_id:
        _fail("find_snapshot_stale", "panel snapshot is not bound to current revision")
    if not isinstance(client_operation_id, str) or len(client_operation_id) < 8:
        _fail("invalid_client_operation_id", "client_operation_id is required")
    if not isinstance(source_hash, str) or not source_hash:
        _fail("invalid_source_hash", "source_hash is required")

    return {
        "protocol_version": "chaptera.story-find-replace-intent.v1",
        "document_id": panel.document_id,
        "source_hash": source_hash,
        "base_revision_id": base_revision_id,
        "client_operation_id": client_operation_id,
        "command": {
            "kind": "story_find_replace",
            "story_id": panel.story_id,
            "base_story_revision_id": panel.revision_id,
            "find_snapshot": snapshot.to_dict(),
            "selected_match_ordinals": ordinals,
            "external_replacement_text": panel.replacement_text,
            "paragraph_ids_by_match": paragraph_ids_by_match,
            "format_generation_id": format_generation_id,
        },
    }


def refresh_after_accepted_story_revision_v1(
    *,
    panel: EditorTextFindReplacePanelV1,
    revision_id: str,
    story_text: str,
    domain: StoryEditDomainV1,
) -> EditorTextFindReplacePanelV1:
    # Every accepted Story text revision invalidates the previous snapshot and
    # snapshot-local ordinal. Regenerate only from new canonical state.
    stale = mark_find_panel_stale_v1(panel, reason="accepted_story_revision")
    return refresh_editor_text_find_snapshot_v1(
        panel=stale,
        revision_id=revision_id,
        story_text=story_text,
        domain=domain,
    )
