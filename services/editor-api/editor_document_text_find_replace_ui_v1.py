#!/usr/bin/env python3
"""Desktop publication-wide exact-literal Find/Replace controller V1.

This product layer owns panel/navigation/session-reconciliation policy only.
Search identity remains DocumentTextFindSnapshotV1; result activation delegates
to TextProgrammaticJumpV1; mutation delegates to DocumentTextReplaceAllV1.
Canonical StoryId/scalar ordering is an explicitly non-reading-order
presentation policy.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from document_text_find_v1 import (
    DocumentStorySearchInputV1,
    DocumentTextFindSnapshotV1,
    build_document_text_find_snapshot_v1,
    ordered_document_matches_v1,
)
from story_edit_domain_v1 import StoryEditDomainV1
from text_edit_session_v1 import TextEditSessionV1
from text_programmatic_jump_v1 import TextProgrammaticJumpRequestV1
from text_selection_state_v1 import (
    PostEditSelectionIntentV1,
    RawTextEditV1,
    TextSelectionStateError,
    TextSelectionStateV1,
    build_text_edit_receipt_v1,
    reconcile_post_edit_selection_v1,
)


class EditorDocumentTextFindReplaceUIError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class DocumentFindResultRefV1:
    story_id: str
    match_ordinal: int
    start_scalar: int
    end_scalar: int
    matched_text: str


@dataclass(frozen=True)
class EditorDocumentTextFindReplacePanelV1:
    protocol_version: Literal[
        "chaptera.editor-document-text-find-replace-panel.v1"
    ]
    document_id: str
    revision_id: str
    find_text: str
    replacement_text: str
    snapshot: DocumentTextFindSnapshotV1 | None
    current_result_index: int | None
    wrap_navigation: bool
    presentation_policy: Literal["canonical_story_id_scalar"]
    presentation_semantics: Literal["non_reading_order"]
    status: Literal[
        "ready",
        "empty_query",
        "incomplete_search",
        "stale",
        "unsupported",
    ]
    status_reason: str | None

    @property
    def total_result_count(self) -> int:
        return 0 if self.snapshot is None else self.snapshot.total_match_count

    @property
    def exhaustive_searchable(self) -> bool:
        return bool(self.snapshot is not None and self.snapshot.exhaustive_searchable)


@dataclass(frozen=True)
class DocumentActiveSessionReconciliationV1:
    protocol_version: Literal[
        "chaptera.document-active-session-reconciliation.v1"
    ]
    status: Literal[
        "no_active_session",
        "unaffected_story_rebind",
        "affected_story_rebind",
        "selection_reconcile_required",
    ]
    active_story_id: str | None
    affected_story: bool
    current_result_replaced: bool
    resulting_selection: TextSelectionStateV1 | None
    typing_state_policy: Literal["none", "preserve", "recompute"]
    session_creation_count: Literal[0]
    other_story_session_count: Literal[0]
    rollback_document_revision: Literal[False]
    reason: str | None


def _fail(code: str, message: str) -> None:
    raise EditorDocumentTextFindReplaceUIError(code, message)


def _validate_panel_identity(
    panel: EditorDocumentTextFindReplacePanelV1,
) -> None:
    if not isinstance(panel, EditorDocumentTextFindReplacePanelV1):
        _fail("invalid_document_find_panel", "document Find/Replace panel is required")
    if panel.protocol_version != "chaptera.editor-document-text-find-replace-panel.v1":
        _fail("invalid_document_find_panel", "panel protocol mismatch")
    if panel.presentation_policy != "canonical_story_id_scalar":
        _fail("unsupported_presentation_policy", "V1 permits only canonical StoryId/scalar order")
    if panel.presentation_semantics != "non_reading_order":
        _fail(
            "invalid_presentation_semantics",
            "canonical result order must not be labeled semantic reading order",
        )


def _snapshot_status(
    snapshot: DocumentTextFindSnapshotV1,
) -> tuple[
    Literal["ready", "incomplete_search"],
    str | None,
]:
    unsupported = [
        f"{item.story_id}:{item.reason}"
        for item in snapshot.story_results
        if item.status == "unsupported"
    ]
    if unsupported:
        return "incomplete_search", "; ".join(unsupported)
    return "ready", None


def open_editor_document_text_find_replace_panel_v1(
    *,
    document_id: str,
    revision_id: str,
    stories: tuple[DocumentStorySearchInputV1, ...],
    find_text: str = "",
    replacement_text: str = "",
    wrap_navigation: bool = True,
) -> EditorDocumentTextFindReplacePanelV1:
    for label, value in (
        ("document_id", document_id),
        ("revision_id", revision_id),
    ):
        if not isinstance(value, str) or not value:
            _fail("invalid_document_find_context", f"{label} is required")
    if not isinstance(find_text, str) or not isinstance(replacement_text, str):
        _fail("invalid_document_find_input", "find/replacement text must be strings")

    panel = EditorDocumentTextFindReplacePanelV1(
        protocol_version="chaptera.editor-document-text-find-replace-panel.v1",
        document_id=document_id,
        revision_id=revision_id,
        find_text=find_text,
        replacement_text=replacement_text,
        snapshot=None,
        current_result_index=None,
        wrap_navigation=bool(wrap_navigation),
        presentation_policy="canonical_story_id_scalar",
        presentation_semantics="non_reading_order",
        status="empty_query" if find_text == "" else "ready",
        status_reason=None,
    )
    if find_text == "":
        return panel
    return refresh_document_text_find_snapshot_v1(
        panel=panel,
        revision_id=revision_id,
        stories=stories,
    )


def set_document_find_text_v1(
    *,
    panel: EditorDocumentTextFindReplacePanelV1,
    find_text: str,
    stories: tuple[DocumentStorySearchInputV1, ...],
) -> EditorDocumentTextFindReplacePanelV1:
    _validate_panel_identity(panel)
    if not isinstance(find_text, str):
        _fail("invalid_document_find_input", "find_text must be string")
    changed = replace(
        panel,
        find_text=find_text,
        snapshot=None,
        current_result_index=None,
        status="empty_query" if find_text == "" else "ready",
        status_reason=None,
    )
    if find_text == "":
        return changed
    return refresh_document_text_find_snapshot_v1(
        panel=changed,
        revision_id=panel.revision_id,
        stories=stories,
    )


def set_document_replacement_text_v1(
    panel: EditorDocumentTextFindReplacePanelV1,
    replacement_text: str,
) -> EditorDocumentTextFindReplacePanelV1:
    _validate_panel_identity(panel)
    if not isinstance(replacement_text, str):
        _fail("invalid_document_find_input", "replacement_text must be string")
    return replace(panel, replacement_text=replacement_text)


def refresh_document_text_find_snapshot_v1(
    *,
    panel: EditorDocumentTextFindReplacePanelV1,
    revision_id: str,
    stories: tuple[DocumentStorySearchInputV1, ...],
) -> EditorDocumentTextFindReplacePanelV1:
    _validate_panel_identity(panel)
    if panel.find_text == "":
        return replace(
            panel,
            revision_id=revision_id,
            snapshot=None,
            current_result_index=None,
            status="empty_query",
            status_reason=None,
        )
    try:
        snapshot = build_document_text_find_snapshot_v1(
            revision_id=revision_id,
            stories=stories,
            external_query=panel.find_text,
        )
    except ValueError as exc:
        return replace(
            panel,
            revision_id=revision_id,
            snapshot=None,
            current_result_index=None,
            status="unsupported",
            status_reason=str(exc),
        )
    status, reason = _snapshot_status(snapshot)
    return replace(
        panel,
        revision_id=revision_id,
        snapshot=snapshot,
        current_result_index=None,
        status=status,
        status_reason=reason,
    )


def mark_document_find_panel_stale_v1(
    panel: EditorDocumentTextFindReplacePanelV1,
    *,
    reason: str,
) -> EditorDocumentTextFindReplacePanelV1:
    _validate_panel_identity(panel)
    return replace(
        panel,
        snapshot=None,
        current_result_index=None,
        status="stale",
        status_reason=reason,
    )


def ordered_document_result_refs_v1(
    panel: EditorDocumentTextFindReplacePanelV1,
) -> tuple[DocumentFindResultRefV1, ...]:
    _validate_panel_identity(panel)
    if panel.snapshot is None:
        return ()
    return tuple(
        DocumentFindResultRefV1(
            story_id=story_id,
            match_ordinal=match.ordinal,
            start_scalar=match.start_scalar,
            end_scalar=match.end_scalar,
            matched_text=match.matched_text,
        )
        for story_id, match in ordered_document_matches_v1(panel.snapshot)
    )


def build_document_find_navigation_jump_v1(
    *,
    panel: EditorDocumentTextFindReplacePanelV1,
    direction: Literal["next", "previous"],
) -> tuple[
    EditorDocumentTextFindReplacePanelV1,
    TextProgrammaticJumpRequestV1 | None,
]:
    _validate_panel_identity(panel)
    if panel.snapshot is None or panel.status in {"empty_query", "stale", "unsupported"}:
        _fail(
            "document_find_snapshot_unavailable",
            panel.status_reason or "document Find snapshot is unavailable",
        )
    refs = ordered_document_result_refs_v1(panel)
    if not refs:
        return replace(panel, current_result_index=None), None

    if direction == "next":
        if panel.current_result_index is None:
            index = 0
        elif panel.current_result_index + 1 < len(refs):
            index = panel.current_result_index + 1
        elif panel.wrap_navigation:
            index = 0
        else:
            return panel, None
    elif direction == "previous":
        if panel.current_result_index is None:
            index = len(refs) - 1
        elif panel.current_result_index > 0:
            index = panel.current_result_index - 1
        elif panel.wrap_navigation:
            index = len(refs) - 1
        else:
            return panel, None
    else:
        _fail("invalid_find_direction", "direction must be next or previous")

    target = refs[index]
    jump = TextProgrammaticJumpRequestV1(
        protocol_version="chaptera.text-programmatic-jump.v1",
        document_id=panel.document_id,
        revision_id=panel.revision_id,
        story_id=target.story_id,
        start_scalar=target.start_scalar,
        end_scalar=target.end_scalar,
        selection_mode="exact_range",
        reason="document_find_result",
    )
    return replace(panel, current_result_index=index), jump


def build_document_text_replace_all_request_v1(
    *,
    panel: EditorDocumentTextFindReplacePanelV1,
    source_hash: str,
    client_operation_id: str,
    paragraph_ids_by_match: list[dict],
) -> dict:
    _validate_panel_identity(panel)
    if panel.snapshot is None or panel.status in {"empty_query", "stale", "unsupported"}:
        _fail(
            "document_find_snapshot_unavailable",
            panel.status_reason or "document Find snapshot is unavailable",
        )
    if not panel.snapshot.exhaustive_searchable:
        _fail(
            "document_search_incomplete",
            panel.status_reason or "publication Replace All requires every Story search domain",
        )
    if not isinstance(source_hash, str) or not source_hash:
        _fail("invalid_source_hash", "source_hash is required")
    if not isinstance(client_operation_id, str) or len(client_operation_id) < 8:
        _fail("invalid_client_operation_id", "client_operation_id is required")
    if not isinstance(paragraph_ids_by_match, list):
        _fail("invalid_paragraph_ids", "paragraph_ids_by_match must be list")

    # Zero-match snapshots are intentionally still valid requests. The engine
    # owns the deterministic DocumentTextReplaceAllNoOp receipt, with no
    # successor revision/Undo/session/view mutation.
    return {
        "protocol_version": "chaptera.document-text-replace-all-intent.v1",
        "document_id": panel.document_id,
        "source_hash": source_hash,
        "base_revision_id": panel.revision_id,
        "client_operation_id": client_operation_id,
        "command": {
            "kind": "document_text_replace_all",
            "base_revision_id": panel.revision_id,
            "document_find_snapshot": panel.snapshot.to_dict(),
            "external_replacement_text": panel.replacement_text,
            "paragraph_ids_by_match": paragraph_ids_by_match,
        },
    }


def refresh_after_document_revision_v1(
    *,
    panel: EditorDocumentTextFindReplacePanelV1,
    revision_id: str,
    stories: tuple[DocumentStorySearchInputV1, ...],
) -> EditorDocumentTextFindReplacePanelV1:
    # Any accepted publication text revision invalidates the prior document
    # snapshot and its presentation-local current result index.
    stale = mark_document_find_panel_stale_v1(
        panel,
        reason="accepted_document_revision",
    )
    return refresh_document_text_find_snapshot_v1(
        panel=stale,
        revision_id=revision_id,
        stories=stories,
    )


def _current_result(
    panel: EditorDocumentTextFindReplacePanelV1,
) -> DocumentFindResultRefV1 | None:
    if panel.current_result_index is None:
        return None
    refs = ordered_document_result_refs_v1(panel)
    if not (0 <= panel.current_result_index < len(refs)):
        _fail("invalid_current_result", "current result index is outside immutable snapshot")
    return refs[panel.current_result_index]


def _local_operation_for_story(
    canonical_operation: dict,
    story_id: str,
) -> dict | None:
    if (
        not isinstance(canonical_operation, dict)
        or canonical_operation.get("protocol_version")
        != "chaptera.document-text-replace-all.v1"
        or canonical_operation.get("kind") != "document_text_replace_all"
    ):
        _fail(
            "invalid_document_replace_operation",
            "canonical DocumentTextReplaceAllV1 operation is required",
        )
    multi = canonical_operation.get("multi_story_operation")
    if (
        not isinstance(multi, dict)
        or multi.get("protocol_version")
        != "chaptera.multi-story-text-transaction.v1"
    ):
        _fail(
            "invalid_document_replace_operation",
            "document operation is missing canonical multi-Story receipt",
        )
    found = [
        item
        for item in multi.get("local_operations", [])
        if isinstance(item, dict) and item.get("story_id") == story_id
    ]
    if len(found) > 1:
        _fail(
            "invalid_document_replace_operation",
            "multi-Story receipt contains duplicate active Story entries",
        )
    return None if not found else found[0]


def _receipt_for_local_story_find_replace(
    *,
    local_entry: dict,
    base_revision_id: str,
    resulting_revision_id: str,
) -> object:
    if local_entry.get("producer_kind") != "story_find_replace":
        _fail(
            "invalid_document_replace_operation",
            "document Replace All local producer must be StoryFindReplaceV1",
        )
    operation = local_entry.get("operation")
    if (
        not isinstance(operation, dict)
        or operation.get("protocol_version") != "chaptera.story-find-replace.v1"
    ):
        _fail(
            "invalid_document_replace_operation",
            "local StoryFindReplaceV1 operation is malformed",
        )
    try:
        before_text = operation["inverse_state"]["paragraph_state"]["story_text"]
        after_text = operation["after_state"]["paragraph_state"]["story_text"]
        normalized = operation["normalized_edits"]
        story_id = operation["story_id"]
    except (KeyError, TypeError):
        _fail(
            "invalid_document_replace_operation",
            "local operation is missing exact before/after text receipt",
        )
    if not isinstance(before_text, str) or not isinstance(after_text, str):
        _fail(
            "invalid_document_replace_operation",
            "local operation Story texts are invalid",
        )
    if not isinstance(normalized, list):
        _fail(
            "invalid_document_replace_operation",
            "local normalized edits must be list",
        )
    edits = []
    for item in normalized:
        try:
            source_ordinal = item["snapshot_match_ordinal"]
            start = item["base_start_scalar"]
            end = item["base_end_scalar"]
            inserted_start = item["inserted_start_scalar"]
            inserted_end = item["inserted_end_scalar"]
        except (KeyError, TypeError):
            _fail(
                "invalid_document_replace_operation",
                "local normalized edit is incomplete",
            )
        inserted_len = inserted_end - inserted_start
        edits.append(
            RawTextEditV1(
                source_ordinal=source_ordinal,
                base_start_scalar=start,
                base_end_scalar=end,
                inserted_scalar_len=inserted_len,
            )
        )
    try:
        receipt = build_text_edit_receipt_v1(
            story_id=story_id,
            base_revision_id=base_revision_id,
            resulting_revision_id=resulting_revision_id,
            base_scalar_len=len(before_text),
            edits=tuple(edits),
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))
    if receipt.resulting_scalar_len != len(after_text):
        _fail(
            "invalid_document_replace_operation",
            "local edit receipt length differs from canonical after-state",
        )
    return receipt


def reconcile_active_session_after_document_replace_all_v1(
    *,
    panel_before_command: EditorDocumentTextFindReplacePanelV1,
    canonical_operation: dict,
    resulting_revision_id: str,
    active_session: TextEditSessionV1 | None,
    base_domain: StoryEditDomainV1 | None = None,
    resulting_domain: StoryEditDomainV1 | None = None,
) -> DocumentActiveSessionReconciliationV1:
    _validate_panel_identity(panel_before_command)
    if not isinstance(resulting_revision_id, str) or not resulting_revision_id:
        _fail("invalid_resulting_revision", "resulting revision id is required")

    if active_session is None:
        return DocumentActiveSessionReconciliationV1(
            protocol_version="chaptera.document-active-session-reconciliation.v1",
            status="no_active_session",
            active_story_id=None,
            affected_story=False,
            current_result_replaced=False,
            resulting_selection=None,
            typing_state_policy="none",
            session_creation_count=0,
            other_story_session_count=0,
            rollback_document_revision=False,
            reason=None,
        )
    if not isinstance(active_session, TextEditSessionV1):
        _fail("invalid_session", "TextEditSessionV1 or None is required")
    if active_session.composition_session is not None:
        _fail(
            "composition_transition_required",
            "active Story composition must resolve before publication Replace All",
        )
    if base_domain is None or resulting_domain is None:
        _fail(
            "active_story_domains_required",
            "active Story reconciliation requires base and resulting edit domains",
        )
    if (
        base_domain.story_id != active_session.story_id
        or resulting_domain.story_id != active_session.story_id
    ):
        _fail(
            "active_story_domain_mismatch",
            "active Story domains target a different Story",
        )

    local_entry = _local_operation_for_story(
        canonical_operation,
        active_session.story_id,
    )
    current = _current_result(panel_before_command)

    if local_entry is None:
        # A publication revision changed, but this Story did not. Rebind the
        # canonical selection to the new document revision as a non-text change.
        try:
            receipt = build_text_edit_receipt_v1(
                story_id=active_session.story_id,
                base_revision_id=panel_before_command.revision_id,
                resulting_revision_id=resulting_revision_id,
                base_scalar_len=base_domain.raw_scalar_len,
                edits=(),
            )
            selection = reconcile_post_edit_selection_v1(
                state=active_session.selection,
                base_domain=base_domain,
                resulting_domain=resulting_domain,
                receipt=receipt,
                intent=PostEditSelectionIntentV1(
                    protocol_version="chaptera.post-edit-selection-intent.v1",
                    kind="preserve_exact_for_non_text_mutation",
                ),
            )
        except TextSelectionStateError as exc:
            _fail(exc.code, str(exc))
        return DocumentActiveSessionReconciliationV1(
            protocol_version="chaptera.document-active-session-reconciliation.v1",
            status="unaffected_story_rebind",
            active_story_id=active_session.story_id,
            affected_story=False,
            current_result_replaced=False,
            resulting_selection=selection,
            typing_state_policy="preserve",
            session_creation_count=0,
            other_story_session_count=0,
            rollback_document_revision=False,
            reason=None,
        )

    receipt = _receipt_for_local_story_find_replace(
        local_entry=local_entry,
        base_revision_id=panel_before_command.revision_id,
        resulting_revision_id=resulting_revision_id,
    )
    current_result_replaced = bool(
        current is not None and current.story_id == active_session.story_id
    )

    if current_result_replaced:
        matching = [
            item
            for item in receipt.edits
            if item.source_ordinal == current.match_ordinal
        ]
        if len(matching) != 1:
            _fail(
                "invalid_document_replace_operation",
                "current document result has no unique local replacement receipt",
            )
        intent = PostEditSelectionIntentV1(
            protocol_version="chaptera.post-edit-selection-intent.v1",
            kind="collapse_after_edit",
            edit_ordinal=matching[0].edit_ordinal,
        )
    else:
        intent = PostEditSelectionIntentV1(
            protocol_version="chaptera.post-edit-selection-intent.v1",
            kind="preserve_through_edits",
        )

    try:
        selection = reconcile_post_edit_selection_v1(
            state=active_session.selection,
            base_domain=base_domain,
            resulting_domain=resulting_domain,
            receipt=receipt,
            intent=intent,
        )
    except TextSelectionStateError as exc:
        if exc.code == "selection_reconcile_required":
            return DocumentActiveSessionReconciliationV1(
                protocol_version="chaptera.document-active-session-reconciliation.v1",
                status="selection_reconcile_required",
                active_story_id=active_session.story_id,
                affected_story=True,
                current_result_replaced=current_result_replaced,
                resulting_selection=None,
                typing_state_policy="recompute",
                session_creation_count=0,
                other_story_session_count=0,
                rollback_document_revision=False,
                reason=str(exc),
            )
        _fail(exc.code, str(exc))

    return DocumentActiveSessionReconciliationV1(
        protocol_version="chaptera.document-active-session-reconciliation.v1",
        status="affected_story_rebind",
        active_story_id=active_session.story_id,
        affected_story=True,
        current_result_replaced=current_result_replaced,
        resulting_selection=selection,
        typing_state_policy="recompute",
        session_creation_count=0,
        other_story_session_count=0,
        rollback_document_revision=False,
        reason=None,
    )
