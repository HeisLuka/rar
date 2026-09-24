#!/usr/bin/env python3
"""Transient frozen Find-in-Selection scope V1."""

from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
import json
from typing import Literal

from range_anchor_rebase_v1 import (
    AnchoredRangeV1,
    RangeAnchorPolicyV1,
    StoryRangeEditV1,
    rebase_anchored_range_v1,
)
from story_edit_domain_v1 import StoryEditDomainV1, validate_ordinary_story_range_v1
from text_find_snapshot_v1 import TextFindExtentV1
from text_selection_state_v1 import (
    TextSelectionStateV1,
    validate_selection_state_v1,
)


class TextFindScopeError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextFindScopeSessionV1:
    protocol_version: Literal["chaptera.text-find-scope-session.v1"]
    scope_session_id: str
    story_id: str
    revision_id: str
    start_scalar: int
    end_scalar: int
    status: Literal["active", "invalidated", "terminated"]
    status_reason: str | None

    @property
    def is_active(self) -> bool:
        return self.status == "active"


@dataclass(frozen=True)
class ScopedReplaceCurrentDeltaV1:
    protocol_version: Literal["chaptera.scoped-replace-current-delta.v1"]
    scope_session_id: str
    story_id: str
    base_revision_id: str
    resulting_revision_id: str
    edit_start_scalar: int
    edit_end_scalar: int
    replacement_scalar_len: int


def _fail(code: str, message: str) -> None:
    raise TextFindScopeError(code, message)


def _scope_id(
    *,
    story_id: str,
    revision_id: str,
    start_scalar: int,
    end_scalar: int,
) -> str:
    raw=json.dumps(
        {
            "protocol_version":"chaptera.text-find-scope-capture.v1",
            "story_id":story_id,
            "revision_id":revision_id,
            "start_scalar":start_scalar,
            "end_scalar":end_scalar,
        },
        sort_keys=True,
        separators=(",",":"),
    ).encode("utf-8")
    return "sha256:"+hashlib.sha256(raw).hexdigest()


def capture_text_find_scope_v1(
    *,
    selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    base_revision_id: str,
) -> TextFindScopeSessionV1:
    try:
        validate_selection_state_v1(
            state=selection,
            domain=domain,
            expected_revision_id=base_revision_id,
        )
    except ValueError as exc:
        _fail(getattr(exc,"code","invalid_selection"),str(exc))
    start,end=selection.normalized_range
    if start==end:
        _fail("nonempty_scope_required","Find in Selection requires a non-empty canonical range")
    try:
        validate_ordinary_story_range_v1(
            domain=domain,
            start_scalar=start,
            end_scalar=end,
        )
    except ValueError as exc:
        _fail(getattr(exc,"code","invalid_scope"),str(exc))
    return TextFindScopeSessionV1(
        protocol_version="chaptera.text-find-scope-session.v1",
        scope_session_id=_scope_id(
            story_id=selection.story_id,
            revision_id=base_revision_id,
            start_scalar=start,
            end_scalar=end,
        ),
        story_id=selection.story_id,
        revision_id=base_revision_id,
        start_scalar=start,
        end_scalar=end,
        status="active",
        status_reason=None,
    )


def scope_search_extent_v1(scope: TextFindScopeSessionV1) -> TextFindExtentV1:
    if not isinstance(scope,TextFindScopeSessionV1):
        _fail("invalid_scope","TextFindScopeSessionV1 is required")
    if not scope.is_active:
        _fail("scope_inactive",scope.status_reason or f"scope is {scope.status}")
    return TextFindExtentV1("range",scope.start_scalar,scope.end_scalar)


def scoped_replace_current_delta_from_operation_v1(
    *,
    scope: TextFindScopeSessionV1,
    operation: dict,
    base_revision_id: str,
    resulting_revision_id: str,
) -> ScopedReplaceCurrentDeltaV1:
    if not scope.is_active:
        _fail("scope_inactive","scoped Replace Current requires active scope")
    if scope.revision_id!=base_revision_id:
        _fail("scope_stale","scope revision differs from replacement base")
    if (
        not isinstance(operation,dict)
        or operation.get("protocol_version")!="chaptera.story-find-replace.v1"
        or operation.get("kind")!="story_find_replace"
        or operation.get("story_id")!=scope.story_id
    ):
        _fail("invalid_scoped_replace_receipt","canonical StoryFindReplaceV1 operation is required")
    edits=operation.get("normalized_edits")
    selected=operation.get("selected_match_ordinals")
    if not isinstance(edits,list) or len(edits)!=1 or not isinstance(selected,list) or len(selected)!=1:
        _fail("not_replace_current","scope rebasing is admitted only for one selected Replace Current")
    edit=edits[0]
    required={
        "edit_ordinal",
        "snapshot_match_ordinal",
        "base_start_scalar",
        "base_end_scalar",
        "inserted_start_scalar",
        "inserted_end_scalar",
        "inserted_paragraph_ids",
    }
    if not isinstance(edit,dict) or set(edit)!=required:
        _fail("invalid_scoped_replace_receipt","normalized edit receipt shape is invalid")
    if edit["snapshot_match_ordinal"]!=selected[0]:
        _fail("invalid_scoped_replace_receipt","selected match and normalized edit disagree")
    replacement_text=operation.get("replacement_text")
    if not isinstance(replacement_text,str):
        _fail("invalid_scoped_replace_receipt","replacement text is missing")
    return ScopedReplaceCurrentDeltaV1(
        protocol_version="chaptera.scoped-replace-current-delta.v1",
        scope_session_id=scope.scope_session_id,
        story_id=scope.story_id,
        base_revision_id=base_revision_id,
        resulting_revision_id=resulting_revision_id,
        edit_start_scalar=edit["base_start_scalar"],
        edit_end_scalar=edit["base_end_scalar"],
        replacement_scalar_len=len(replacement_text),
    )


def rebase_scope_after_replace_current_v1(
    *,
    scope: TextFindScopeSessionV1,
    delta: ScopedReplaceCurrentDeltaV1,
    resulting_domain: StoryEditDomainV1,
) -> TextFindScopeSessionV1:
    if not scope.is_active:
        _fail("scope_inactive","cannot rebase inactive scope")
    if (
        not isinstance(delta,ScopedReplaceCurrentDeltaV1)
        or delta.scope_session_id!=scope.scope_session_id
        or delta.story_id!=scope.story_id
        or delta.base_revision_id!=scope.revision_id
    ):
        _fail("scope_delta_mismatch","accepted edit is not attributable to this scoped session")
    anchored=AnchoredRangeV1(scope.start_scalar,scope.end_scalar)
    receipt=rebase_anchored_range_v1(
        anchored=anchored,
        policy=RangeAnchorPolicyV1(
            start_affinity="left",
            end_affinity="right",
            full_cover_policy="replacement",
        ),
        edit=StoryRangeEditV1(
            delta.edit_start_scalar,
            delta.edit_end_scalar,
            delta.replacement_scalar_len,
        ),
    )
    if receipt.result.status!="survives" or receipt.result.range is None:
        return replace(
            scope,
            revision_id=delta.resulting_revision_id,
            status="invalidated",
            status_reason="scope_consumed_by_replace_current",
        )
    rr=receipt.result.range
    try:
        validate_ordinary_story_range_v1(
            domain=resulting_domain,
            start_scalar=rr.start_scalar,
            end_scalar=rr.end_scalar,
        )
    except ValueError:
        return replace(
            scope,
            revision_id=delta.resulting_revision_id,
            status="invalidated",
            status_reason="rebased_scope_outside_resulting_edit_domain",
        )
    if rr.start_scalar==rr.end_scalar:
        return replace(
            scope,
            revision_id=delta.resulting_revision_id,
            status="invalidated",
            status_reason="rebased_scope_became_empty",
        )
    return TextFindScopeSessionV1(
        protocol_version=scope.protocol_version,
        scope_session_id=scope.scope_session_id,
        story_id=scope.story_id,
        revision_id=delta.resulting_revision_id,
        start_scalar=rr.start_scalar,
        end_scalar=rr.end_scalar,
        status="active",
        status_reason=None,
    )


def terminate_scope_after_replace_all_v1(
    scope: TextFindScopeSessionV1,
    *,
    resulting_revision_id: str,
) -> TextFindScopeSessionV1:
    if not scope.is_active:
        _fail("scope_inactive","Replace All scope termination requires active scope")
    return replace(
        scope,
        revision_id=resulting_revision_id,
        status="terminated",
        status_reason="replace_all_completed",
    )


def invalidate_text_find_scope_v1(
    scope: TextFindScopeSessionV1,
    *,
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
) -> TextFindScopeSessionV1:
    if reason not in {
        "unrelated_story_edit","undo_redo","history_jump","story_change",
        "focus_change","session_change","stale_or_unknown_delta",
    }:
        _fail("invalid_invalidation_reason","scope invalidation reason is unsupported")
    if scope.status!="active":
        return scope
    return replace(
        scope,
        revision_id=current_revision_id,
        status="invalidated",
        status_reason=reason,
    )
