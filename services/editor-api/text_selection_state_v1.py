#!/usr/bin/env python3
"""Transient canonical Story selection state V1.

This module owns session/UI selection semantics only. It never persists a caret
or DOM Range into EditorProject/document state.

Canonical order:
1. selection lives in Story Unicode-scalar coordinates and StoryEditDomainV1;
2. accepted text edits expose one normalized TextEditReceiptV1 in base coords;
3. PostEditSelectionIntentV1 deterministically reconciles anchor/focus;
4. accepted text mutation clears stale layout/visual-stop binding;
5. authoritative reflow produces ResolvedTextCaretMapV1;
6. projection binds canonical endpoints back to physical caret stops/selection
   geometry. Browser/widget geometry is never input authority.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
import json
from typing import Literal

from resolved_text_caret_map_v1 import (
    CaretStopV1,
    ResolvedTextCaretMapError,
    ResolvedTextCaretMapV1,
    SelectionGeometryV1,
    resolve_story_position_v1,
    selection_geometry_v1,
)
from story_edit_domain_v1 import StoryEditDomainV1


EndpointRebasePolicyV1 = Literal["left", "right", "reconcile_required"]
PostEditSelectionKindV1 = Literal[
    "collapse_after_edit",
    "collapse_at_edit_start",
    "preserve_through_edits",
    "preserve_exact_for_non_text_mutation",
    "select_inserted_range",
]
ProjectionStateV1 = Literal["layout_pending", "projected"]


class TextSelectionStateError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextSelectionStateV1:
    protocol_version: Literal["chaptera.text-selection-state.v1"]
    story_id: str
    anchor_scalar: int
    focus_scalar: int
    revision_id: str
    edit_domain_id: str
    projection_state: ProjectionStateV1
    layout_revision_id: str | None
    anchor_visual_stop_id: str | None
    focus_visual_stop_id: str | None
    preferred_inline_x_emu: int | None

    @property
    def is_collapsed(self) -> bool:
        return self.anchor_scalar == self.focus_scalar

    @property
    def normalized_range(self) -> tuple[int, int]:
        return (
            min(self.anchor_scalar, self.focus_scalar),
            max(self.anchor_scalar, self.focus_scalar),
        )


@dataclass(frozen=True)
class RawTextEditV1:
    """One accepted edit expressed in the common base Story coordinate space."""

    source_ordinal: int
    base_start_scalar: int
    base_end_scalar: int
    inserted_scalar_len: int


@dataclass(frozen=True)
class TextEditReceiptEntryV1:
    edit_ordinal: int
    source_ordinal: int
    base_start_scalar: int
    base_end_scalar: int
    inserted_scalar_len: int
    final_inserted_start_scalar: int
    final_inserted_end_scalar: int


@dataclass(frozen=True)
class TextEditReceiptV1:
    protocol_version: Literal["chaptera.text-edit-receipt.v1"]
    story_id: str
    base_revision_id: str
    resulting_revision_id: str
    base_scalar_len: int
    resulting_scalar_len: int
    edits: tuple[TextEditReceiptEntryV1, ...]


@dataclass(frozen=True)
class PostEditSelectionIntentV1:
    protocol_version: Literal["chaptera.post-edit-selection-intent.v1"]
    kind: PostEditSelectionKindV1
    edit_ordinal: int | None = None
    anchor_policy: EndpointRebasePolicyV1 = "reconcile_required"
    focus_policy: EndpointRebasePolicyV1 = "reconcile_required"


@dataclass(frozen=True)
class TextSelectionProjectionV1:
    protocol_version: Literal["chaptera.text-selection-projection.v1"]
    state: TextSelectionStateV1
    anchor_stop: CaretStopV1
    focus_stop: CaretStopV1
    geometry: SelectionGeometryV1


def _fail(code: str, message: str) -> None:
    raise TextSelectionStateError(code, message)


def _require_nonempty_string(value: str, label: str) -> str:
    if not isinstance(value, str) or not value:
        _fail("invalid_selection_state", f"{label} is required")
    return value


def _require_scalar(value: int, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        _fail("invalid_selection_state", f"{label} must be a non-negative scalar boundary")
    return value


def edit_domain_id_v1(domain: StoryEditDomainV1) -> str:
    if not isinstance(domain, StoryEditDomainV1):
        _fail("invalid_edit_domain", "StoryEditDomainV1 is required")
    payload = json.dumps(
        domain.to_dict(),
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def _require_known_domain(domain: StoryEditDomainV1, story_id: str) -> None:
    if not isinstance(domain, StoryEditDomainV1):
        _fail("invalid_edit_domain", "StoryEditDomainV1 is required")
    if domain.story_id != story_id:
        _fail("selection_story_mismatch", "selection and edit domain target different Stories")
    if domain.status != "known":
        _fail("edit_domain_unknown", "ordinary Story selection domain is unknown")
    if domain.caret_start_boundary is None or domain.caret_end_boundary is None:
        _fail("edit_domain_unknown", "ordinary Story caret boundaries are unavailable")


def _require_admitted_boundary(
    domain: StoryEditDomainV1,
    story_id: str,
    scalar: int,
    label: str,
) -> None:
    _require_known_domain(domain, story_id)
    _require_scalar(scalar, label)
    assert domain.caret_start_boundary is not None
    assert domain.caret_end_boundary is not None
    if not domain.caret_start_boundary <= scalar <= domain.caret_end_boundary:
        _fail(
            "selection_reconcile_required",
            f"{label} lies outside the current ordinary Story edit domain",
        )


def build_text_selection_state_v1(
    *,
    domain: StoryEditDomainV1,
    revision_id: str,
    anchor_scalar: int,
    focus_scalar: int,
    preferred_inline_x_emu: int | None = None,
) -> TextSelectionStateV1:
    story_id = domain.story_id
    _require_nonempty_string(revision_id, "revision_id")
    _require_admitted_boundary(domain, story_id, anchor_scalar, "anchor_scalar")
    _require_admitted_boundary(domain, story_id, focus_scalar, "focus_scalar")
    if (
        preferred_inline_x_emu is not None
        and (
            not isinstance(preferred_inline_x_emu, int)
            or isinstance(preferred_inline_x_emu, bool)
        )
    ):
        _fail("invalid_selection_state", "preferred_inline_x_emu must be integer or null")

    return TextSelectionStateV1(
        protocol_version="chaptera.text-selection-state.v1",
        story_id=story_id,
        anchor_scalar=anchor_scalar,
        focus_scalar=focus_scalar,
        revision_id=revision_id,
        edit_domain_id=edit_domain_id_v1(domain),
        projection_state="layout_pending",
        layout_revision_id=None,
        anchor_visual_stop_id=None,
        focus_visual_stop_id=None,
        preferred_inline_x_emu=preferred_inline_x_emu,
    )


def validate_selection_state_v1(
    *,
    state: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    expected_revision_id: str | None = None,
) -> None:
    if not isinstance(state, TextSelectionStateV1):
        _fail("invalid_selection_state", "TextSelectionStateV1 is required")
    if state.protocol_version != "chaptera.text-selection-state.v1":
        _fail("invalid_selection_state", "selection protocol_version mismatch")
    _require_known_domain(domain, state.story_id)
    if state.edit_domain_id != edit_domain_id_v1(domain):
        _fail(
            "selection_reconcile_required",
            "selection was projected against a different Story edit domain",
        )
    if expected_revision_id is not None and state.revision_id != expected_revision_id:
        _fail("stale_selection_revision", "selection belongs to a different canonical revision")
    _require_admitted_boundary(domain, state.story_id, state.anchor_scalar, "anchor_scalar")
    _require_admitted_boundary(domain, state.story_id, state.focus_scalar, "focus_scalar")

    if state.projection_state == "layout_pending":
        if any(
            value is not None
            for value in (
                state.layout_revision_id,
                state.anchor_visual_stop_id,
                state.focus_visual_stop_id,
            )
        ):
            _fail("invalid_selection_state", "layout-pending selection carries stale geometry")
    elif state.projection_state == "projected":
        if not state.layout_revision_id:
            _fail("invalid_selection_state", "projected selection requires layout_revision_id")
        if not state.anchor_visual_stop_id or not state.focus_visual_stop_id:
            _fail("invalid_selection_state", "projected selection requires endpoint visual stops")
        if state.is_collapsed and state.anchor_visual_stop_id != state.focus_visual_stop_id:
            _fail("invalid_selection_state", "collapsed selection must use one physical caret stop")
    else:
        _fail("invalid_selection_state", "unsupported projection_state")


def build_text_edit_receipt_v1(
    *,
    story_id: str,
    base_revision_id: str,
    resulting_revision_id: str,
    base_scalar_len: int,
    edits: tuple[RawTextEditV1, ...],
) -> TextEditReceiptV1:
    _require_nonempty_string(story_id, "story_id")
    _require_nonempty_string(base_revision_id, "base_revision_id")
    _require_nonempty_string(resulting_revision_id, "resulting_revision_id")
    if (
        not isinstance(base_scalar_len, int)
        or isinstance(base_scalar_len, bool)
        or base_scalar_len < 0
    ):
        _fail("invalid_text_edit_receipt", "base_scalar_len must be non-negative")
    if not isinstance(edits, tuple):
        _fail("invalid_text_edit_receipt", "edits must be an immutable tuple")

    seen_source_ordinals: set[int] = set()
    raw = []
    for item in edits:
        if not isinstance(item, RawTextEditV1):
            _fail("invalid_text_edit_receipt", "raw edit entry is malformed")
        if (
            not isinstance(item.source_ordinal, int)
            or isinstance(item.source_ordinal, bool)
            or item.source_ordinal < 0
            or item.source_ordinal in seen_source_ordinals
        ):
            _fail("invalid_text_edit_receipt", "source edit ordinals must be unique non-negative integers")
        seen_source_ordinals.add(item.source_ordinal)
        if (
            not isinstance(item.base_start_scalar, int)
            or isinstance(item.base_start_scalar, bool)
            or not isinstance(item.base_end_scalar, int)
            or isinstance(item.base_end_scalar, bool)
            or item.base_start_scalar < 0
            or item.base_end_scalar < item.base_start_scalar
            or item.base_end_scalar > base_scalar_len
        ):
            _fail("invalid_text_edit_receipt", "base edit range is invalid")
        if (
            not isinstance(item.inserted_scalar_len, int)
            or isinstance(item.inserted_scalar_len, bool)
            or item.inserted_scalar_len < 0
        ):
            _fail("invalid_text_edit_receipt", "inserted_scalar_len must be non-negative")
        raw.append(item)

    raw.sort(key=lambda item: (item.base_start_scalar, item.base_end_scalar, item.source_ordinal))
    previous_end = 0
    for index, item in enumerate(raw):
        if index and item.base_start_scalar < previous_end:
            _fail("invalid_text_edit_receipt", "base-coordinate edits overlap")
        previous_end = max(previous_end, item.base_end_scalar)

    normalized = []
    delta = 0
    for edit_ordinal, item in enumerate(raw):
        final_start = item.base_start_scalar + delta
        final_end = final_start + item.inserted_scalar_len
        normalized.append(
            TextEditReceiptEntryV1(
                edit_ordinal=edit_ordinal,
                source_ordinal=item.source_ordinal,
                base_start_scalar=item.base_start_scalar,
                base_end_scalar=item.base_end_scalar,
                inserted_scalar_len=item.inserted_scalar_len,
                final_inserted_start_scalar=final_start,
                final_inserted_end_scalar=final_end,
            )
        )
        delta += item.inserted_scalar_len - (
            item.base_end_scalar - item.base_start_scalar
        )

    resulting_scalar_len = base_scalar_len + delta
    if resulting_scalar_len < 0:
        _fail("invalid_text_edit_receipt", "resulting Story scalar length is negative")

    return TextEditReceiptV1(
        protocol_version="chaptera.text-edit-receipt.v1",
        story_id=story_id,
        base_revision_id=base_revision_id,
        resulting_revision_id=resulting_revision_id,
        base_scalar_len=base_scalar_len,
        resulting_scalar_len=resulting_scalar_len,
        edits=tuple(normalized),
    )


def single_edit_receipt_from_story_transaction_v1(
    *,
    operation: dict,
    base_revision_id: str,
    resulting_revision_id: str,
    source_ordinal: int = 0,
) -> TextEditReceiptV1:
    if (
        not isinstance(operation, dict)
        or operation.get("protocol_version") != "chaptera.story-edit-transaction.v1"
        or operation.get("kind") != "story_edit_transaction"
    ):
        _fail("invalid_text_edit_receipt", "canonical StoryEditTransactionV1 operation is required")
    inverse = operation.get("inverse_state")
    after = operation.get("after_state")
    try:
        before_text = inverse["paragraph_state"]["story_text"]
        after_text = after["paragraph_state"]["story_text"]
        story_id = operation["story_id"]
        start = operation["start_scalar"]
        end = operation["end_scalar"]
        replacement_text = operation["replacement_text"]
    except (KeyError, TypeError):
        _fail("invalid_text_edit_receipt", "Story transaction receipt is incomplete")
    if not isinstance(before_text, str) or not isinstance(after_text, str):
        _fail("invalid_text_edit_receipt", "Story transaction text state is invalid")
    if len(after_text) != len(before_text) - (end - start) + len(replacement_text):
        _fail("invalid_text_edit_receipt", "Story transaction scalar lengths are inconsistent")
    return build_text_edit_receipt_v1(
        story_id=story_id,
        base_revision_id=base_revision_id,
        resulting_revision_id=resulting_revision_id,
        base_scalar_len=len(before_text),
        edits=(
            RawTextEditV1(
                source_ordinal=source_ordinal,
                base_start_scalar=start,
                base_end_scalar=end,
                inserted_scalar_len=len(replacement_text),
            ),
        ),
    )


def _entry(receipt: TextEditReceiptV1, edit_ordinal: int | None) -> TextEditReceiptEntryV1:
    if not isinstance(edit_ordinal, int) or isinstance(edit_ordinal, bool):
        _fail("invalid_selection_intent", "edit_ordinal is required")
    matches = [item for item in receipt.edits if item.edit_ordinal == edit_ordinal]
    if len(matches) != 1:
        _fail("invalid_selection_intent", "edit_ordinal is not present in TextEditReceiptV1")
    return matches[0]


def _transform_point(
    *,
    scalar: int,
    edits: tuple[TextEditReceiptEntryV1, ...],
    policy: EndpointRebasePolicyV1,
) -> int:
    if policy not in {"left", "right", "reconcile_required"}:
        _fail("invalid_selection_intent", "endpoint rebase policy is invalid")

    delta = 0
    for item in edits:
        start = item.base_start_scalar
        end = item.base_end_scalar
        final_start = item.final_inserted_start_scalar
        final_end = item.final_inserted_end_scalar

        if scalar < start:
            return scalar + delta
        if scalar > end:
            delta += item.inserted_scalar_len - (end - start)
            continue

        # Insertion boundary: one base point has two legitimate post-edit sides.
        if start == end and scalar == start:
            if policy == "left":
                return final_start
            if policy == "right":
                return final_end
            _fail(
                "selection_reconcile_required",
                "selection endpoint lies on an insertion boundary without explicit affinity",
            )

        # End boundary is canonically after the replaced base range.
        if scalar == end:
            return final_end

        # Start boundary/interior of a replacement needs caller-owned affinity.
        if scalar == start or start < scalar < end:
            if policy == "left":
                return final_start
            if policy == "right":
                return final_end
            _fail(
                "selection_reconcile_required",
                "selection endpoint lies in replaced content without explicit affinity",
            )

    return scalar + delta


def _pending_state(
    *,
    previous: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    revision_id: str,
    anchor_scalar: int,
    focus_scalar: int,
    clear_preferred_inline_x: bool,
) -> TextSelectionStateV1:
    _require_admitted_boundary(domain, previous.story_id, anchor_scalar, "anchor_scalar")
    _require_admitted_boundary(domain, previous.story_id, focus_scalar, "focus_scalar")
    return TextSelectionStateV1(
        protocol_version="chaptera.text-selection-state.v1",
        story_id=previous.story_id,
        anchor_scalar=anchor_scalar,
        focus_scalar=focus_scalar,
        revision_id=revision_id,
        edit_domain_id=edit_domain_id_v1(domain),
        projection_state="layout_pending",
        layout_revision_id=None,
        anchor_visual_stop_id=None,
        focus_visual_stop_id=None,
        preferred_inline_x_emu=(
            None if clear_preferred_inline_x else previous.preferred_inline_x_emu
        ),
    )


def reconcile_post_edit_selection_v1(
    *,
    state: TextSelectionStateV1,
    base_domain: StoryEditDomainV1,
    resulting_domain: StoryEditDomainV1,
    receipt: TextEditReceiptV1,
    intent: PostEditSelectionIntentV1,
) -> TextSelectionStateV1:
    validate_selection_state_v1(
        state=state,
        domain=base_domain,
        expected_revision_id=receipt.base_revision_id,
    )
    if receipt.story_id != state.story_id or resulting_domain.story_id != state.story_id:
        _fail("selection_story_mismatch", "selection/edit receipt/resulting domain disagree")
    if receipt.base_scalar_len != base_domain.raw_scalar_len:
        _fail("invalid_text_edit_receipt", "receipt base length disagrees with base edit domain")
    if receipt.resulting_scalar_len != resulting_domain.raw_scalar_len:
        _fail("invalid_text_edit_receipt", "receipt result length disagrees with resulting edit domain")
    if not isinstance(intent, PostEditSelectionIntentV1):
        _fail("invalid_selection_intent", "PostEditSelectionIntentV1 is required")
    if intent.protocol_version != "chaptera.post-edit-selection-intent.v1":
        _fail("invalid_selection_intent", "selection intent protocol mismatch")

    if intent.kind == "collapse_after_edit":
        entry = _entry(receipt, intent.edit_ordinal)
        anchor = focus = entry.final_inserted_end_scalar
    elif intent.kind == "collapse_at_edit_start":
        entry = _entry(receipt, intent.edit_ordinal)
        anchor = focus = entry.final_inserted_start_scalar
    elif intent.kind == "select_inserted_range":
        entry = _entry(receipt, intent.edit_ordinal)
        anchor = entry.final_inserted_start_scalar
        focus = entry.final_inserted_end_scalar
    elif intent.kind == "preserve_through_edits":
        anchor = _transform_point(
            scalar=state.anchor_scalar,
            edits=receipt.edits,
            policy=intent.anchor_policy,
        )
        focus = _transform_point(
            scalar=state.focus_scalar,
            edits=receipt.edits,
            policy=intent.focus_policy,
        )
    elif intent.kind == "preserve_exact_for_non_text_mutation":
        if receipt.edits or receipt.base_scalar_len != receipt.resulting_scalar_len:
            _fail(
                "invalid_selection_intent",
                "PreserveExactForNonTextMutation requires an empty text-edit receipt",
            )
        anchor = state.anchor_scalar
        focus = state.focus_scalar
    else:
        _fail("invalid_selection_intent", "unsupported post-edit selection intent")

    return _pending_state(
        previous=state,
        domain=resulting_domain,
        revision_id=receipt.resulting_revision_id,
        anchor_scalar=anchor,
        focus_scalar=focus,
        clear_preferred_inline_x=bool(receipt.edits),
    )


def reconcile_rejected_edit_v1(
    *,
    last_authoritative_state: TextSelectionStateV1,
    current_domain: StoryEditDomainV1,
    current_revision_id: str,
) -> TextSelectionStateV1:
    """Return last authoritative selection only when it is still valid.

    A browser/DOM provisional selection is intentionally not an input.
    """
    validate_selection_state_v1(
        state=last_authoritative_state,
        domain=current_domain,
        expected_revision_id=current_revision_id,
    )
    return last_authoritative_state


def project_selection_state_v1(
    *,
    state: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    anchor_stop_id: str | None = None,
    focus_stop_id: str | None = None,
) -> TextSelectionProjectionV1:
    validate_selection_state_v1(
        state=state,
        domain=domain,
        expected_revision_id=state.revision_id,
    )
    if caret_map.story_id != state.story_id:
        _fail("selection_story_mismatch", "caret map targets a different Story")

    use_anchor_stop = (
        anchor_stop_id
        if anchor_stop_id is not None
        else (
            state.anchor_visual_stop_id
            if state.layout_revision_id == caret_map.layout_revision_id
            else None
        )
    )
    use_focus_stop = (
        focus_stop_id
        if focus_stop_id is not None
        else (
            state.focus_visual_stop_id
            if state.layout_revision_id == caret_map.layout_revision_id
            else None
        )
    )
    if state.is_collapsed:
        if use_anchor_stop is None and use_focus_stop is not None:
            use_anchor_stop = use_focus_stop
        if use_focus_stop is None and use_anchor_stop is not None:
            use_focus_stop = use_anchor_stop
        if (
            use_anchor_stop is not None
            and use_focus_stop is not None
            and use_anchor_stop != use_focus_stop
        ):
            _fail("invalid_caret_affinity", "collapsed selection cannot use two visual stops")

    try:
        anchor = resolve_story_position_v1(
            caret_map=caret_map,
            scalar_boundary=state.anchor_scalar,
            stop_id=use_anchor_stop,
            expected_layout_revision_id=caret_map.layout_revision_id,
        )
        focus = resolve_story_position_v1(
            caret_map=caret_map,
            scalar_boundary=state.focus_scalar,
            stop_id=use_focus_stop,
            expected_layout_revision_id=caret_map.layout_revision_id,
        )
        start, end = state.normalized_range
        geometry = selection_geometry_v1(
            caret_map=caret_map,
            start_scalar=start,
            end_scalar=end,
            expected_layout_revision_id=caret_map.layout_revision_id,
        )
    except ResolvedTextCaretMapError as exc:
        _fail(exc.code, str(exc))

    projected = replace(
        state,
        projection_state="projected",
        layout_revision_id=caret_map.layout_revision_id,
        anchor_visual_stop_id=anchor.stop_id,
        focus_visual_stop_id=focus.stop_id,
    )
    validate_selection_state_v1(
        state=projected,
        domain=domain,
        expected_revision_id=state.revision_id,
    )
    return TextSelectionProjectionV1(
        protocol_version="chaptera.text-selection-projection.v1",
        state=projected,
        anchor_stop=anchor,
        focus_stop=focus,
        geometry=geometry,
    )


def selection_discontinuity_v1(
    before: TextSelectionStateV1,
    after: TextSelectionStateV1,
) -> bool:
    """Stable input to a later Undo-grouping policy.

    The grouping task decides whether this transition closes a group; this
    function only reports semantic selection discontinuity.
    """
    if before.story_id != after.story_id:
        return True
    return (
        before.anchor_scalar,
        before.focus_scalar,
        before.anchor_visual_stop_id,
        before.focus_visual_stop_id,
    ) != (
        after.anchor_scalar,
        after.focus_scalar,
        after.anchor_visual_stop_id,
        after.focus_visual_stop_id,
    )
