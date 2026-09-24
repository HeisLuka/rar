#!/usr/bin/env python3
"""Desktop Find/Replace-in-Selection mode V1."""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from editor_text_find_replace_ui_v1 import (
    EditorTextFindReplacePanelV1,
    build_story_find_replace_request_v1,
    mark_find_panel_stale_v1,
    refresh_after_accepted_story_revision_v1,
)
from story_edit_domain_v1 import StoryEditDomainV1
from text_find_scope_v1 import (
    TextFindScopeSessionV1,
    capture_text_find_scope_v1,
    invalidate_text_find_scope_v1,
    rebase_scope_after_replace_current_v1,
    scope_search_extent_v1,
    scoped_replace_current_delta_from_operation_v1,
    terminate_scope_after_replace_all_v1,
)
from text_find_snapshot_v1 import build_text_find_snapshot_v1
from text_selection_state_v1 import TextSelectionStateV1


class EditorTextFindScopeUIError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class EditorTextFindScopedModeV1:
    protocol_version: Literal["chaptera.editor-text-find-scoped-mode.v1"]
    panel: EditorTextFindReplacePanelV1
    scope: TextFindScopeSessionV1 | None
    status: Literal["inactive", "active", "invalidated"]
    status_reason: str | None


def _fail(code: str, message: str) -> None:
    raise EditorTextFindScopeUIError(code, message)


def _refresh_scoped_snapshot(
    *,
    panel: EditorTextFindReplacePanelV1,
    scope: TextFindScopeSessionV1,
    story_text: str,
    domain: StoryEditDomainV1,
) -> EditorTextFindReplacePanelV1:
    if panel.find_text == "":
        return replace(
            panel,
            revision_id=scope.revision_id,
            snapshot=None,
            current_ordinal=None,
            status="empty_query",
            status_reason=None,
        )
    extent=scope_search_extent_v1(scope)
    try:
        snapshot=build_text_find_snapshot_v1(
            revision_id=scope.revision_id,
            story_id=scope.story_id,
            story_text=story_text,
            domain=domain,
            external_query=panel.find_text,
            extent=extent,
        )
    except ValueError as exc:
        return replace(
            panel,
            revision_id=scope.revision_id,
            snapshot=None,
            current_ordinal=None,
            status="unsupported",
            status_reason=str(exc),
        )
    return replace(
        panel,
        revision_id=scope.revision_id,
        snapshot=snapshot,
        current_ordinal=None,
        status="ready",
        status_reason=None,
    )


def enter_find_in_selection_v1(
    *,
    panel: EditorTextFindReplacePanelV1,
    selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    story_text: str,
) -> EditorTextFindScopedModeV1:
    try:
        scope=capture_text_find_scope_v1(
            selection=selection,
            domain=domain,
            base_revision_id=panel.revision_id,
        )
    except ValueError as exc:
        _fail(getattr(exc,"code","scope_capture_failed"),str(exc))
    scoped_panel=_refresh_scoped_snapshot(
        panel=panel,
        scope=scope,
        story_text=story_text,
        domain=domain,
    )
    return EditorTextFindScopedModeV1(
        protocol_version="chaptera.editor-text-find-scoped-mode.v1",
        panel=scoped_panel,
        scope=scope,
        status="active",
        status_reason=None,
    )


def build_scoped_replace_request_v1(
    *,
    mode: EditorTextFindScopedModeV1,
    replace_mode: Literal["current","all"],
    base_revision_id: str,
    client_operation_id: str,
    source_hash: str,
    paragraph_ids_by_match: list[dict],
    format_generation_id: str,
) -> dict | None:
    if mode.status!="active" or mode.scope is None or not mode.scope.is_active:
        _fail("scope_inactive","Find in Selection mode is not active")
    if mode.scope.revision_id!=mode.panel.revision_id:
        _fail("scope_stale","scope and panel snapshot revisions differ")
    return build_story_find_replace_request_v1(
        panel=mode.panel,
        mode=replace_mode,
        base_revision_id=base_revision_id,
        client_operation_id=client_operation_id,
        source_hash=source_hash,
        paragraph_ids_by_match=paragraph_ids_by_match,
        format_generation_id=format_generation_id,
    )


def after_scoped_replace_current_v1(
    *,
    mode: EditorTextFindScopedModeV1,
    canonical_operation: dict,
    base_revision_id: str,
    resulting_revision_id: str,
    resulting_story_text: str,
    resulting_domain: StoryEditDomainV1,
) -> EditorTextFindScopedModeV1:
    if mode.status!="active" or mode.scope is None:
        _fail("scope_inactive","Replace Current result requires active scoped mode")
    try:
        delta=scoped_replace_current_delta_from_operation_v1(
            scope=mode.scope,
            operation=canonical_operation,
            base_revision_id=base_revision_id,
            resulting_revision_id=resulting_revision_id,
        )
        scope=rebase_scope_after_replace_current_v1(
            scope=mode.scope,
            delta=delta,
            resulting_domain=resulting_domain,
        )
    except ValueError as exc:
        _fail(getattr(exc,"code","scope_rebase_failed"),str(exc))

    if not scope.is_active:
        stale=mark_find_panel_stale_v1(
            mode.panel,reason=scope.status_reason or "scope_invalidated"
        )
        return EditorTextFindScopedModeV1(
            protocol_version=mode.protocol_version,
            panel=replace(stale,revision_id=resulting_revision_id),
            scope=scope,
            status="invalidated",
            status_reason=scope.status_reason,
        )

    panel=_refresh_scoped_snapshot(
        panel=mode.panel,
        scope=scope,
        story_text=resulting_story_text,
        domain=resulting_domain,
    )
    return EditorTextFindScopedModeV1(
        protocol_version=mode.protocol_version,
        panel=panel,
        scope=scope,
        status="active",
        status_reason=None,
    )


def after_scoped_replace_all_v1(
    *,
    mode: EditorTextFindScopedModeV1,
    resulting_revision_id: str,
    resulting_story_text: str,
    resulting_domain: StoryEditDomainV1,
) -> EditorTextFindScopedModeV1:
    if mode.status!="active" or mode.scope is None:
        _fail("scope_inactive","Replace All result requires active scoped mode")
    ended=terminate_scope_after_replace_all_v1(
        mode.scope,
        resulting_revision_id=resulting_revision_id,
    )
    panel=refresh_after_accepted_story_revision_v1(
        panel=mode.panel,
        revision_id=resulting_revision_id,
        story_text=resulting_story_text,
        domain=resulting_domain,
    )
    return EditorTextFindScopedModeV1(
        protocol_version=mode.protocol_version,
        panel=panel,
        scope=None,
        status="inactive",
        status_reason=ended.status_reason,
    )


def invalidate_scoped_find_mode_v1(
    *,
    mode: EditorTextFindScopedModeV1,
    current_revision_id: str,
    reason: Literal[
        "unrelated_story_edit",
        "undo_redo",
        "history_jump",
        "story_change",
        "focus_change",
        "session_change",
        "stale_or_unknown_delta",
    ],
) -> EditorTextFindScopedModeV1:
    if mode.scope is None:
        return mode
    scope=invalidate_text_find_scope_v1(
        mode.scope,
        current_revision_id=current_revision_id,
        reason=reason,
    )
    panel=mark_find_panel_stale_v1(mode.panel,reason=reason)
    return EditorTextFindScopedModeV1(
        protocol_version=mode.protocol_version,
        panel=replace(panel,revision_id=current_revision_id),
        scope=scope,
        status="invalidated",
        status_reason=reason,
    )
