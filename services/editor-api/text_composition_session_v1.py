#!/usr/bin/env python3
"""Transient source-neutral IME composition session V1.

Composition is provisional interaction state over one canonical Story range.
Updates create no revision. Commit normalizes final external text once and emits
exactly one existing StoryEditTransactionV1 request; cancel emits no mutation.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from story_edit_domain_v1 import (
    StoryEditDomainV1,
    validate_ordinary_story_range_v1,
)
from story_range_v1 import validate_scalar_sequence_v1
from text_ingress_v1 import normalize_external_text_v1
from text_insert_format_v1 import TypingFormatSnapshotV1
from text_selection_state_v1 import (
    PostEditSelectionIntentV1,
    TextSelectionStateV1,
    edit_domain_id_v1,
    validate_selection_state_v1,
)
from text_typing_format_state_v1 import (
    TextTypingFormatStateV1,
    snapshot_typing_format_v1,
)


class TextCompositionSessionError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextCompositionSessionV1:
    protocol_version: Literal["chaptera.text-composition-session.v1"]
    composition_id: str
    story_id: str
    base_revision_id: str
    edit_domain_id: str
    start_scalar: int
    end_scalar: int
    expected_before: str
    captured_selection: TextSelectionStateV1
    typing_snapshot: TypingFormatSnapshotV1 | None
    provisional_external_text: str
    provisional_selection_start_scalar: int
    provisional_selection_end_scalar: int


@dataclass(frozen=True)
class CompositionCancelResultV1:
    protocol_version: Literal["chaptera.text-composition-cancelled.v1"]
    restored_selection: TextSelectionStateV1
    document_mutation_count: Literal[0]
    undo_group_boundary: Literal[False]


@dataclass(frozen=True)
class CompositionCommitPlanV1:
    protocol_version: Literal["chaptera.text-composition-commit-plan.v1"]
    composition_id: str
    canonical_replacement_text: str
    request: dict
    post_edit_selection_intent: PostEditSelectionIntentV1
    document_mutation_count: Literal[1]
    undo_group_boundary: Literal[True]


def _fail(code: str, message: str) -> None:
    raise TextCompositionSessionError(code, message)


def _require_string(value: str, label: str) -> str:
    if not isinstance(value, str) or not value:
        _fail("invalid_composition", f"{label} is required")
    return value


def _validate_external_provisional(text: str) -> int:
    try:
        return validate_scalar_sequence_v1(text, "provisional_external_text")
    except ValueError as exc:
        _fail("invalid_composition_text", str(exc))


def start_text_composition_v1(
    *,
    composition_id: str,
    canonical_story_text: str,
    domain: StoryEditDomainV1,
    selection: TextSelectionStateV1,
    typing_state: TextTypingFormatStateV1 | None = None,
) -> TextCompositionSessionV1:
    _require_string(composition_id, "composition_id")
    validate_selection_state_v1(
        state=selection,
        domain=domain,
        expected_revision_id=selection.revision_id,
    )
    if selection.story_id != domain.story_id:
        _fail("composition_story_mismatch", "selection/domain target different Stories")
    if selection.edit_domain_id != edit_domain_id_v1(domain):
        _fail("reconcile_required", "selection edit-domain fence is stale")
    try:
        story_len = validate_scalar_sequence_v1(canonical_story_text, "canonical_story_text")
    except ValueError as exc:
        _fail("invalid_story", str(exc))
    if story_len != domain.raw_scalar_len:
        _fail("reconcile_required", "canonical Story text differs from captured edit domain")

    start = min(selection.anchor_scalar, selection.focus_scalar)
    end = max(selection.anchor_scalar, selection.focus_scalar)
    try:
        validate_ordinary_story_range_v1(
            domain=domain,
            start_scalar=start,
            end_scalar=end,
        )
    except ValueError as exc:
        code = getattr(exc, "code", "reconcile_required")
        _fail(code, str(exc))

    if typing_state is not None:
        if not selection.is_collapsed:
            _fail(
                "invalid_typing_snapshot",
                "noncollapsed composition cannot carry collapsed-caret typing state",
            )
        if (
            typing_state.story_id != selection.story_id
            or typing_state.revision_id != selection.revision_id
            or typing_state.caret_scalar != selection.focus_scalar
            or typing_state.edit_domain_id != selection.edit_domain_id
        ):
            _fail("reconcile_required", "typing state belongs to a different caret context")

    expected_before = canonical_story_text[start:end]
    snapshot = snapshot_typing_format_v1(typing_state)
    return TextCompositionSessionV1(
        protocol_version="chaptera.text-composition-session.v1",
        composition_id=composition_id,
        story_id=selection.story_id,
        base_revision_id=selection.revision_id,
        edit_domain_id=selection.edit_domain_id,
        start_scalar=start,
        end_scalar=end,
        expected_before=expected_before,
        captured_selection=selection,
        typing_snapshot=snapshot,
        provisional_external_text=expected_before,
        provisional_selection_start_scalar=0,
        provisional_selection_end_scalar=len(expected_before),
    )


def update_text_composition_v1(
    session: TextCompositionSessionV1,
    *,
    provisional_external_text: str,
    provisional_selection_start_scalar: int,
    provisional_selection_end_scalar: int,
) -> TextCompositionSessionV1:
    if not isinstance(session, TextCompositionSessionV1):
        _fail("invalid_composition", "TextCompositionSessionV1 is required")
    scalar_len = _validate_external_provisional(provisional_external_text)
    if (
        not isinstance(provisional_selection_start_scalar, int)
        or isinstance(provisional_selection_start_scalar, bool)
        or not isinstance(provisional_selection_end_scalar, int)
        or isinstance(provisional_selection_end_scalar, bool)
        or provisional_selection_start_scalar < 0
        or provisional_selection_end_scalar < provisional_selection_start_scalar
        or provisional_selection_end_scalar > scalar_len
    ):
        _fail("invalid_provisional_selection", "provisional selection is outside composition text")
    return replace(
        session,
        provisional_external_text=provisional_external_text,
        provisional_selection_start_scalar=provisional_selection_start_scalar,
        provisional_selection_end_scalar=provisional_selection_end_scalar,
    )


def _require_same_authoritative_base(
    session: TextCompositionSessionV1,
    *,
    current_revision_id: str,
    current_story_text: str,
    current_domain: StoryEditDomainV1,
) -> None:
    if current_revision_id != session.base_revision_id:
        _fail(
            "reconcile_required",
            "authoritative revision changed during active composition",
        )
    if current_domain.story_id != session.story_id:
        _fail("reconcile_required", "active composition Story focus changed")
    if edit_domain_id_v1(current_domain) != session.edit_domain_id:
        _fail("reconcile_required", "Story edit domain changed during active composition")
    try:
        validate_scalar_sequence_v1(current_story_text, "current_story_text")
    except ValueError as exc:
        _fail("invalid_story", str(exc))
    if (
        session.end_scalar > len(current_story_text)
        or current_story_text[session.start_scalar:session.end_scalar] != session.expected_before
    ):
        _fail(
            "reconcile_required",
            "captured composition range no longer matches canonical Story state",
        )


def cancel_text_composition_v1(
    session: TextCompositionSessionV1,
    *,
    current_revision_id: str,
    current_story_text: str,
    current_domain: StoryEditDomainV1,
) -> CompositionCancelResultV1:
    _require_same_authoritative_base(
        session,
        current_revision_id=current_revision_id,
        current_story_text=current_story_text,
        current_domain=current_domain,
    )
    return CompositionCancelResultV1(
        protocol_version="chaptera.text-composition-cancelled.v1",
        restored_selection=session.captured_selection,
        document_mutation_count=0,
        undo_group_boundary=False,
    )


def plan_text_composition_commit_v1(
    session: TextCompositionSessionV1,
    *,
    current_revision_id: str,
    current_story_text: str,
    current_domain: StoryEditDomainV1,
    final_external_text: str,
    document_id: str,
    source_hash: str,
    client_operation_id: str,
    paragraph_inserted_ids: tuple[str, ...] = (),
    paragraph_inserted_property_presets: tuple[dict | None, ...] = (),
) -> CompositionCommitPlanV1:
    _require_same_authoritative_base(
        session,
        current_revision_id=current_revision_id,
        current_story_text=current_story_text,
        current_domain=current_domain,
    )
    _require_string(document_id, "document_id")
    _require_string(source_hash, "source_hash")
    _require_string(client_operation_id, "client_operation_id")

    try:
        canonical = normalize_external_text_v1(final_external_text)
    except ValueError as exc:
        _fail("invalid_composition_text", str(exc))

    required_paragraph_ids = canonical.text.count("\r")
    if (
        not isinstance(paragraph_inserted_ids, tuple)
        or len(paragraph_inserted_ids) != required_paragraph_ids
        or any(not isinstance(value, str) or not value for value in paragraph_inserted_ids)
        or len(set(paragraph_inserted_ids)) != len(paragraph_inserted_ids)
    ):
        _fail(
            "paragraph_ids_required",
            "composition commit requires one unique preallocated ParagraphId per inserted U+000D",
        )
    if (
        not isinstance(paragraph_inserted_property_presets, tuple)
        or len(paragraph_inserted_property_presets) not in {0, required_paragraph_ids}
    ):
        _fail(
            "invalid_paragraph_presets",
            "paragraph property presets must be empty or match inserted ParagraphIds",
        )

    typing = (
        None
        if session.typing_snapshot is None
        else {prop: value for prop, value in session.typing_snapshot.items}
    )
    request = {
        "protocol_version": "chaptera.story-edit-transaction-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": session.base_revision_id,
        "client_operation_id": client_operation_id,
        "depends_on_client_operation_id": None,
        "command": {
            "kind": "story_edit_transaction",
            "story_id": session.story_id,
            "start_scalar": session.start_scalar,
            "end_scalar": session.end_scalar,
            "expected_before": session.expected_before,
            "replacement_text": canonical.text,
            "paragraph_inserted_ids": list(paragraph_inserted_ids),
            "paragraph_inserted_property_presets": list(
                paragraph_inserted_property_presets
            ),
            "typing_format": typing,
            "fragment_format_runs": [],
            "incoming_semantic_kinds": [],
        },
    }
    return CompositionCommitPlanV1(
        protocol_version="chaptera.text-composition-commit-plan.v1",
        composition_id=session.composition_id,
        canonical_replacement_text=canonical.text,
        request=request,
        post_edit_selection_intent=PostEditSelectionIntentV1(
            protocol_version="chaptera.post-edit-selection-intent.v1",
            kind="collapse_after_edit",
            edit_ordinal=0,
        ),
        document_mutation_count=1,
        undo_group_boundary=True,
    )


def composition_suppresses_document_shortcuts_v1(
    session: TextCompositionSessionV1 | None,
) -> bool:
    return session is not None
