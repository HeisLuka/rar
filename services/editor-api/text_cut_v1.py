#!/usr/bin/env python3
"""Two-phase canonical text Cut orchestration V1.

Cut is deliberately not a DOM/OS cut primitive:
1. capture one immutable semantic fragment + text/plain from one canonical
   Story selection/revision;
2. let a product clipboard adapter attempt the external write;
3. only an explicit confirmed-success receipt may lower the frozen deletion to
   one idempotent StoryEditTransactionV1 request.

Clipboard state is external side effect. Undo/Redo never restore it.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from story_edit_domain_v1 import (
    StoryEditDomainV1,
    derive_story_edit_domain_v1,
)
from story_edit_transaction_v1 import (
    StoryEditCoreStateV1,
    validate_story_edit_transaction_operation_v1,
    validate_story_edit_transaction_request_v1,
)
from text_egress_v1 import TextPlainEgressV1, egress_story_range_v1
from text_range_fragment_v1 import (
    TextRangeFragmentError,
    TextRangeFragmentV1,
    UnsupportedSemanticSpanV1,
    capture_text_range_fragment_v1,
)
from text_selection_state_v1 import (
    PostEditSelectionIntentV1,
    TextSelectionStateError,
    TextSelectionStateV1,
    reconcile_post_edit_selection_v1,
    single_edit_receipt_from_story_transaction_v1,
    validate_selection_state_v1,
)


CutInputOwnerV1 = Literal[
    "story_text",
    "composition",
    "modal",
    "inspector",
    "canvas",
]
ClipboardWriteStatusV1 = Literal[
    "confirmed_success",
    "failed",
    "denied",
    "cancelled",
    "timeout",
    "unknown",
    "unsupported",
]


class TextCutError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class ClipboardWriteResultV1:
    protocol_version: Literal["chaptera.clipboard-write-result.v1"]
    status: ClipboardWriteStatusV1
    transport_receipt_id: str | None = None
    reason: str | None = None


@dataclass(frozen=True)
class TextCutClipboardPayloadV1:
    protocol_version: Literal["chaptera.text-cut-clipboard-payload.v1"]
    story_id: str
    start_scalar: int
    end_scalar: int
    fragment: TextRangeFragmentV1
    fragment_sha256: str
    plain_text: TextPlainEgressV1


@dataclass(frozen=True)
class TextCutCaptureV1:
    protocol_version: Literal["chaptera.text-cut-capture.v1"]
    document_id: str
    source_hash: str
    base_revision_id: str
    client_operation_id: str
    story_id: str
    start_scalar: int
    end_scalar: int
    expected_before: str
    base_domain: StoryEditDomainV1
    captured_selection: TextSelectionStateV1
    payload: TextCutClipboardPayloadV1


@dataclass(frozen=True)
class TextCutPrepareResultV1:
    protocol_version: Literal["chaptera.text-cut-prepare-result.v1"]
    status: Literal[
        "ready",
        "no_op",
        "suppressed",
        "clipboard_semantics_unsupported",
    ]
    capture: TextCutCaptureV1 | None
    clipboard_write_required: bool
    document_mutation_count: Literal[0]
    reason: str | None


@dataclass(frozen=True)
class TextCutDeleteDecisionV1:
    protocol_version: Literal["chaptera.text-cut-delete-decision.v1"]
    status: Literal[
        "delete_ready",
        "clipboard_not_confirmed",
        "no_delete",
    ]
    request: dict | None
    client_operation_id: str | None
    clipboard_status: ClipboardWriteStatusV1 | None
    clipboard_external_state_unknown: bool
    retry_policy: Literal["none", "reconcile_same_operation_id"]
    document_mutation_count: Literal[0]
    reason: str | None


@dataclass(frozen=True)
class TextCutAcceptedResultV1:
    protocol_version: Literal["chaptera.text-cut-accepted-result.v1"]
    story_id: str
    base_revision_id: str
    resulting_revision_id: str
    client_operation_id: str
    selection: TextSelectionStateV1
    typing_state_cleared: Literal[True]
    preferred_inline_x_cleared: Literal[True]
    authoring_revision_count: Literal[1]
    undo_history_entry_count: Literal[1]
    clipboard_undo_policy: Literal["external_not_restored"]


_ALLOWED_OWNERS = {
    "story_text",
    "composition",
    "modal",
    "inspector",
    "canvas",
}
_ALLOWED_CLIPBOARD = {
    "confirmed_success",
    "failed",
    "denied",
    "cancelled",
    "timeout",
    "unknown",
    "unsupported",
}


def _fail(code: str, message: str) -> None:
    raise TextCutError(code, message)


def _require_id(value: str, label: str, *, min_len: int = 1) -> str:
    if not isinstance(value, str) or len(value) < min_len:
        _fail("invalid_cut_context", f"{label} is required")
    return value


def _base_domain(before_state: StoryEditCoreStateV1) -> StoryEditDomainV1:
    try:
        return derive_story_edit_domain_v1(
            story_id=before_state.story_id,
            story_text=before_state.paragraph_state.story_text,
            provenance=before_state.provenance,
        )
    except ValueError as exc:
        _fail(getattr(exc, "code", "invalid_cut_context"), str(exc))


def prepare_text_cut_v1(
    *,
    document_id: str,
    source_hash: str,
    base_revision_id: str,
    client_operation_id: str,
    current_selection: TextSelectionStateV1,
    before_state: StoryEditCoreStateV1,
    unsupported_semantic_spans: tuple[UnsupportedSemanticSpanV1, ...] = (),
    input_owner: CutInputOwnerV1 = "story_text",
    composition_active: bool = False,
) -> TextCutPrepareResultV1:
    _require_id(document_id, "document_id")
    _require_id(source_hash, "source_hash")
    _require_id(base_revision_id, "base_revision_id")
    _require_id(client_operation_id, "client_operation_id", min_len=8)
    if input_owner not in _ALLOWED_OWNERS:
        _fail("invalid_input_owner", "unsupported Cut focus owner")
    if not isinstance(composition_active, bool):
        _fail("invalid_composition_state", "composition_active must be boolean")
    if not isinstance(before_state, StoryEditCoreStateV1):
        _fail("invalid_cut_context", "StoryEditCoreStateV1 is required")
    if not isinstance(current_selection, TextSelectionStateV1):
        _fail("invalid_cut_context", "TextSelectionStateV1 is required")

    domain = _base_domain(before_state)
    try:
        validate_selection_state_v1(
            state=current_selection,
            domain=domain,
            expected_revision_id=base_revision_id,
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))

    start, end = current_selection.normalized_range

    if input_owner != "story_text" or composition_active:
        return TextCutPrepareResultV1(
            protocol_version="chaptera.text-cut-prepare-result.v1",
            status="suppressed",
            capture=None,
            clipboard_write_required=False,
            document_mutation_count=0,
            reason=(
                "active composition owns text input"
                if composition_active
                else f"{input_owner} owns Cut input"
            ),
        )

    if start == end:
        return TextCutPrepareResultV1(
            protocol_version="chaptera.text-cut-prepare-result.v1",
            status="no_op",
            capture=None,
            clipboard_write_required=False,
            document_mutation_count=0,
            reason="Cut requires a non-empty canonical Story selection",
        )

    try:
        fragment_receipt = capture_text_range_fragment_v1(
            state=before_state,
            start_scalar=start,
            end_scalar=end,
            unsupported_semantic_spans=unsupported_semantic_spans,
        )
    except TextRangeFragmentError as exc:
        if exc.code in {
            "capture_unsupported",
            "semantic_inventory_incomplete",
            "semantic_inventory_invalid",
        }:
            return TextCutPrepareResultV1(
                protocol_version="chaptera.text-cut-prepare-result.v1",
                status="clipboard_semantics_unsupported",
                capture=None,
                clipboard_write_required=False,
                document_mutation_count=0,
                reason=f"{exc.code}:{exc}",
            )
        _fail(exc.code, str(exc))

    # Copy can explicitly diagnose paragraph-semantics loss because the source
    # remains intact. Cut cannot safely delete the only authoritative copy when
    # the richest admitted clipboard fragment cannot reconstruct that semantic
    # state, so V1 fails before touching the external clipboard.
    if fragment_receipt.fragment.paragraph_semantics != "lossless_under_base":
        return TextCutPrepareResultV1(
            protocol_version="chaptera.text-cut-prepare-result.v1",
            status="clipboard_semantics_unsupported",
            capture=None,
            clipboard_write_required=False,
            document_mutation_count=0,
            reason="paragraph_semantics_loss",
        )

    try:
        plain = egress_story_range_v1(
            story_id=before_state.story_id,
            story_text=before_state.paragraph_state.story_text,
            provenance=before_state.provenance,
            start_scalar=start,
            end_scalar=end,
        )
    except ValueError as exc:
        _fail(getattr(exc, "code", "text_egress_rejected"), str(exc))

    fragment = fragment_receipt.fragment
    if (
        fragment.text != plain.canonical_text
        or fragment.scalar_len != plain.canonical_scalar_len
    ):
        _fail(
            "clipboard_payload_mismatch",
            "semantic fragment and text/plain were not captured from one canonical range",
        )

    payload = TextCutClipboardPayloadV1(
        protocol_version="chaptera.text-cut-clipboard-payload.v1",
        story_id=before_state.story_id,
        start_scalar=start,
        end_scalar=end,
        fragment=fragment,
        fragment_sha256=fragment_receipt.fragment_sha256,
        plain_text=plain,
    )
    capture = TextCutCaptureV1(
        protocol_version="chaptera.text-cut-capture.v1",
        document_id=document_id,
        source_hash=source_hash,
        base_revision_id=base_revision_id,
        client_operation_id=client_operation_id,
        story_id=before_state.story_id,
        start_scalar=start,
        end_scalar=end,
        expected_before=fragment.text,
        base_domain=domain,
        captured_selection=current_selection,
        payload=payload,
    )
    return TextCutPrepareResultV1(
        protocol_version="chaptera.text-cut-prepare-result.v1",
        status="ready",
        capture=capture,
        clipboard_write_required=True,
        document_mutation_count=0,
        reason=None,
    )


def plan_text_cut_after_clipboard_v1(
    *,
    prepared: TextCutPrepareResultV1,
    clipboard_result: ClipboardWriteResultV1,
) -> TextCutDeleteDecisionV1:
    if not isinstance(prepared, TextCutPrepareResultV1):
        _fail("invalid_cut_state", "TextCutPrepareResultV1 is required")
    if not isinstance(clipboard_result, ClipboardWriteResultV1):
        _fail("invalid_clipboard_result", "ClipboardWriteResultV1 is required")
    if (
        clipboard_result.protocol_version
        != "chaptera.clipboard-write-result.v1"
        or clipboard_result.status not in _ALLOWED_CLIPBOARD
    ):
        _fail("invalid_clipboard_result", "clipboard write result is invalid")

    if prepared.status != "ready" or prepared.capture is None:
        return TextCutDeleteDecisionV1(
            protocol_version="chaptera.text-cut-delete-decision.v1",
            status="no_delete",
            request=None,
            client_operation_id=None,
            clipboard_status=clipboard_result.status,
            clipboard_external_state_unknown=False,
            retry_policy="none",
            document_mutation_count=0,
            reason=prepared.reason or f"Cut prepare status is {prepared.status}",
        )

    capture = prepared.capture
    if clipboard_result.status != "confirmed_success":
        return TextCutDeleteDecisionV1(
            protocol_version="chaptera.text-cut-delete-decision.v1",
            status="clipboard_not_confirmed",
            request=None,
            client_operation_id=capture.client_operation_id,
            clipboard_status=clipboard_result.status,
            clipboard_external_state_unknown=clipboard_result.status in {"timeout", "unknown"},
            retry_policy="none",
            document_mutation_count=0,
            reason=clipboard_result.reason or f"clipboard:{clipboard_result.status}",
        )

    request = {
        "protocol_version": "chaptera.story-edit-transaction-intent.v1",
        "document_id": capture.document_id,
        "source_hash": capture.source_hash,
        "base_revision_id": capture.base_revision_id,
        "client_operation_id": capture.client_operation_id,
        "command": {
            "kind": "story_edit_transaction",
            "story_id": capture.story_id,
            "start_scalar": capture.start_scalar,
            "end_scalar": capture.end_scalar,
            "expected_before": capture.expected_before,
            "replacement_text": "",
            "paragraph_inserted_ids": [],
            "paragraph_inserted_property_presets": [],
            "typing_format": None,
            "fragment_format_runs": [],
            "incoming_semantic_kinds": [],
        },
    }
    try:
        validate_story_edit_transaction_request_v1(request)
    except ValueError as exc:
        _fail("invalid_cut_delete_request", str(exc))

    return TextCutDeleteDecisionV1(
        protocol_version="chaptera.text-cut-delete-decision.v1",
        status="delete_ready",
        request=request,
        client_operation_id=capture.client_operation_id,
        clipboard_status=clipboard_result.status,
        clipboard_external_state_unknown=False,
        retry_policy="reconcile_same_operation_id",
        document_mutation_count=0,
        reason=None,
    )


def reconcile_text_cut_accepted_v1(
    *,
    capture: TextCutCaptureV1,
    request: dict,
    canonical_operation: dict,
    resulting_revision_id: str,
    resulting_domain: StoryEditDomainV1,
) -> TextCutAcceptedResultV1:
    if not isinstance(capture, TextCutCaptureV1):
        _fail("invalid_cut_state", "TextCutCaptureV1 is required")
    _require_id(resulting_revision_id, "resulting_revision_id")
    if request.get("client_operation_id") != capture.client_operation_id:
        _fail("cut_operation_mismatch", "accepted request uses another client operation id")
    if request.get("base_revision_id") != capture.base_revision_id:
        _fail("cut_operation_mismatch", "accepted request uses another base revision")
    command = request.get("command")
    expected_command = {
        "kind": "story_edit_transaction",
        "story_id": capture.story_id,
        "start_scalar": capture.start_scalar,
        "end_scalar": capture.end_scalar,
        "expected_before": capture.expected_before,
        "replacement_text": "",
        "paragraph_inserted_ids": [],
        "paragraph_inserted_property_presets": [],
        "typing_format": None,
        "fragment_format_runs": [],
        "incoming_semantic_kinds": [],
    }
    if command != expected_command:
        _fail("cut_operation_mismatch", "accepted delete differs from frozen Cut capture")
    try:
        validate_story_edit_transaction_operation_v1(command, canonical_operation)
        receipt = single_edit_receipt_from_story_transaction_v1(
            operation=canonical_operation,
            base_revision_id=capture.base_revision_id,
            resulting_revision_id=resulting_revision_id,
            source_ordinal=0,
        )
        selection = reconcile_post_edit_selection_v1(
            state=capture.captured_selection,
            base_domain=capture.base_domain,
            resulting_domain=resulting_domain,
            receipt=receipt,
            intent=PostEditSelectionIntentV1(
                protocol_version="chaptera.post-edit-selection-intent.v1",
                kind="collapse_at_edit_start",
                edit_ordinal=0,
            ),
        )
    except (ValueError, TextSelectionStateError) as exc:
        _fail(getattr(exc, "code", "cut_reconciliation_failed"), str(exc))

    if (
        selection.anchor_scalar != capture.start_scalar
        or selection.focus_scalar != capture.start_scalar
    ):
        _fail(
            "cut_reconciliation_failed",
            "accepted Cut did not collapse selection at removed range start",
        )

    return TextCutAcceptedResultV1(
        protocol_version="chaptera.text-cut-accepted-result.v1",
        story_id=capture.story_id,
        base_revision_id=capture.base_revision_id,
        resulting_revision_id=resulting_revision_id,
        client_operation_id=capture.client_operation_id,
        selection=selection,
        typing_state_cleared=True,
        preferred_inline_x_cleared=True,
        authoring_revision_count=1,
        undo_history_entry_count=1,
        clipboard_undo_policy="external_not_restored",
    )
