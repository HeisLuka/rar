#!/usr/bin/env python3
"""Canonical source-neutral TextRangeFragmentV1.

A text-range fragment is not an object-clone fragment. It contains only:
- exact canonical Story text selected from one non-empty ordinary-editable range;
- normalized *effective* supported character-format values in fragment-local
  Unicode-scalar coordinates;
- explicit paragraph-semantics diagnostics.

Base V1 carries no hyperlinks or other persistent anchored semantics. Capture
fails closed when the selected range intersects such semantics. Paragraph U+000D
scalars are preserved, but source paragraph properties/features are never
silently transferred; their presence is surfaced as paragraph_semantics_loss.

Paste lowering produces one StoryEditTransactionV1 request. Destination
paragraph lifecycle owns ParagraphIds/properties; supported fragment character
format is materialized through TextInsertFormatV1 inside that transaction.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from typing import Any, Literal

from story_edit_domain_v1 import (
    StoryEditDomainError,
    derive_story_edit_domain_v1,
    validate_ordinary_story_range_v1,
)
from story_edit_transaction_v1 import StoryEditCoreStateV1
from text_format_overlay_v1 import (
    FormatPropertyV1,
    effective_property_segments_v1,
)


_SUPPORTED_PROPERTIES: tuple[FormatPropertyV1, ...] = (
    "bold",
    "font_size_emu",
    "italic",
    "text_color_rgb",
)


class TextRangeFragmentError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class UnsupportedSemanticSpanV1:
    semantic_kind: str
    start_scalar: int
    end_scalar: int


@dataclass(frozen=True)
class FragmentFormatRunV1:
    start_offset: int
    end_offset: int
    property: FormatPropertyV1
    value: Any


@dataclass(frozen=True)
class TextRangeFragmentV1:
    protocol_version: Literal["chaptera.text-range-fragment.v1"]
    text: str
    scalar_len: int
    format_runs: tuple[FragmentFormatRunV1, ...]
    paragraph_semantics: Literal[
        "lossless_under_base",
        "paragraph_semantics_loss",
    ]
    diagnostics: tuple[str, ...]


@dataclass(frozen=True)
class CaptureTextRangeReceiptV1:
    protocol_version: Literal["chaptera.capture-text-range-receipt.v1"]
    source_story_id: str
    source_start_scalar: int
    source_end_scalar: int
    fragment: TextRangeFragmentV1
    fragment_sha256: str


@dataclass(frozen=True)
class PasteTextRangePlanV1:
    protocol_version: Literal["chaptera.paste-text-range-plan.v1"]
    request: dict
    fragment_sha256: str
    diagnostics: tuple[str, ...]


def _fail(code: str, message: str) -> None:
    raise TextRangeFragmentError(code, message)


def _overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:
    return start < other_end and other_start < end


def _validate_unsupported_span(
    span: UnsupportedSemanticSpanV1,
    *,
    story_len: int,
) -> None:
    if not isinstance(span, UnsupportedSemanticSpanV1):
        _fail("semantic_inventory_invalid", "unsupported semantic span is malformed")
    if not isinstance(span.semantic_kind, str) or not span.semantic_kind:
        _fail("semantic_inventory_invalid", "unsupported semantic kind is required")
    if (
        not isinstance(span.start_scalar, int)
        or isinstance(span.start_scalar, bool)
        or not isinstance(span.end_scalar, int)
        or isinstance(span.end_scalar, bool)
        or span.start_scalar < 0
        or span.end_scalar <= span.start_scalar
        or span.end_scalar > story_len
    ):
        _fail("semantic_inventory_invalid", "unsupported semantic span range is invalid")


def _selected_paragraph_indices(
    text: str,
    start: int,
    end: int,
    *,
    protected_terminal_cr: bool,
) -> tuple[int, ...]:
    boundaries = [i for i, ch in enumerate(text) if ch == "\r"]
    if protected_terminal_cr and boundaries and boundaries[-1] == len(text) - 1:
        boundaries = boundaries[:-1]

    indices = set()
    for scalar in range(start, end):
        # A paragraph delimiter belongs to its upstream paragraph. Scalars
        # after it belong to the downstream paragraph.
        indices.add(sum(1 for boundary in boundaries if boundary < scalar))
    return tuple(sorted(indices))


def _paragraph_semantics(
    state: StoryEditCoreStateV1,
    start: int,
    end: int,
) -> tuple[str, tuple[str, ...]]:
    selected = _selected_paragraph_indices(
        state.paragraph_state.story_text,
        start,
        end,
        protected_terminal_cr=state.paragraph_state.protected_terminal_cr,
    )
    loss = bool(state.active_paragraph_features)
    for index in selected:
        paragraph = state.paragraph_state.paragraphs[index]
        if paragraph.properties.items:
            loss = True
            break
    if loss:
        return (
            "paragraph_semantics_loss",
            ("paragraph_semantics_loss",),
        )
    return ("lossless_under_base", ())


def _capture_effective_format_runs(
    state: StoryEditCoreStateV1,
    start: int,
    end: int,
) -> tuple[FragmentFormatRunV1, ...]:
    out = []
    for prop in _SUPPORTED_PROPERTIES:
        segments = effective_property_segments_v1(
            state=state.format_state,
            prop=prop,
            start_scalar=start,
            end_scalar=end,
        )
        for segment in segments:
            out.append(
                FragmentFormatRunV1(
                    start_offset=segment.start_scalar - start,
                    end_offset=segment.end_scalar - start,
                    property=prop,
                    value=segment.value,
                )
            )
    out.sort(
        key=lambda run: (
            run.property,
            run.start_offset,
            run.end_offset,
            json.dumps(run.value, sort_keys=True),
        )
    )
    return tuple(out)


def _validate_fragment(fragment: TextRangeFragmentV1) -> TextRangeFragmentV1:
    if not isinstance(fragment, TextRangeFragmentV1):
        _fail("fragment_invalid", "TextRangeFragmentV1 is required")
    if fragment.protocol_version != "chaptera.text-range-fragment.v1":
        _fail("fragment_invalid", "TextRangeFragmentV1 protocol mismatch")
    if not isinstance(fragment.text, str) or not fragment.text:
        _fail("fragment_invalid", "TextRangeFragmentV1 text must be non-empty")
    if fragment.scalar_len != len(fragment.text) or fragment.scalar_len <= 0:
        _fail("fragment_invalid", "fragment scalar_len does not match text")
    if fragment.paragraph_semantics not in {
        "lossless_under_base",
        "paragraph_semantics_loss",
    }:
        _fail("fragment_invalid", "paragraph semantics classification is invalid")
    expected_diagnostics = (
        ("paragraph_semantics_loss",)
        if fragment.paragraph_semantics == "paragraph_semantics_loss"
        else ()
    )
    if fragment.diagnostics != expected_diagnostics:
        _fail("fragment_invalid", "fragment diagnostics/classification disagree")

    runs = fragment.format_runs
    if not isinstance(runs, tuple):
        _fail("fragment_invalid", "format_runs must be an ordered tuple")
    by_property: dict[str, list[FragmentFormatRunV1]] = {
        prop: [] for prop in _SUPPORTED_PROPERTIES
    }
    for run in runs:
        if not isinstance(run, FragmentFormatRunV1):
            _fail("fragment_invalid", "fragment format run is malformed")
        if run.property not in by_property:
            _fail("fragment_invalid", "fragment carries unsupported format property")
        if (
            not isinstance(run.start_offset, int)
            or isinstance(run.start_offset, bool)
            or not isinstance(run.end_offset, int)
            or isinstance(run.end_offset, bool)
            or run.start_offset < 0
            or run.end_offset <= run.start_offset
            or run.end_offset > fragment.scalar_len
        ):
            _fail("fragment_invalid", "fragment format run range is invalid")
        by_property[run.property].append(run)

    for prop, prop_runs in by_property.items():
        if not prop_runs:
            _fail("fragment_invalid", f"fragment lacks effective {prop} coverage")
        prop_runs.sort(key=lambda run: (run.start_offset, run.end_offset))
        cursor = 0
        for run in prop_runs:
            if run.start_offset != cursor:
                _fail("fragment_invalid", f"fragment {prop} coverage has gap/overlap")
            cursor = run.end_offset
        if cursor != fragment.scalar_len:
            _fail("fragment_invalid", f"fragment {prop} does not cover full text")
    return fragment


def text_range_fragment_to_dict(fragment: TextRangeFragmentV1) -> dict:
    fragment = _validate_fragment(fragment)
    return {
        "protocol_version": fragment.protocol_version,
        "text": fragment.text,
        "scalar_len": fragment.scalar_len,
        "format_runs": [
            {
                "start_offset": run.start_offset,
                "end_offset": run.end_offset,
                "property": run.property,
                "value": run.value,
            }
            for run in fragment.format_runs
        ],
        "paragraph_semantics": fragment.paragraph_semantics,
        "diagnostics": list(fragment.diagnostics),
    }


def canonical_text_range_fragment_json_v1(fragment: TextRangeFragmentV1) -> str:
    return json.dumps(
        text_range_fragment_to_dict(fragment),
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )


def text_range_fragment_sha256_v1(fragment: TextRangeFragmentV1) -> str:
    return hashlib.sha256(
        canonical_text_range_fragment_json_v1(fragment).encode("utf-8")
    ).hexdigest()


def capture_text_range_fragment_v1(
    *,
    state: StoryEditCoreStateV1,
    start_scalar: int,
    end_scalar: int,
    unsupported_semantic_spans: tuple[UnsupportedSemanticSpanV1, ...] = (),
) -> CaptureTextRangeReceiptV1:
    if not isinstance(state, StoryEditCoreStateV1):
        _fail("capture_invalid", "StoryEditCoreStateV1 is required")
    if start_scalar == end_scalar:
        _fail("capture_empty", "TextRangeFragmentV1 requires non-empty selection")

    try:
        domain = derive_story_edit_domain_v1(
            story_id=state.story_id,
            story_text=state.paragraph_state.story_text,
            provenance=state.provenance,
        )
        validate_ordinary_story_range_v1(
            domain=domain,
            start_scalar=start_scalar,
            end_scalar=end_scalar,
        )
    except StoryEditDomainError as exc:
        _fail(exc.code, str(exc))

    story_len = len(state.paragraph_state.story_text)
    if not isinstance(unsupported_semantic_spans, tuple):
        _fail("semantic_inventory_invalid", "unsupported semantic inventory must be tuple")
    inventory_kinds = set()
    for span in unsupported_semantic_spans:
        _validate_unsupported_span(span, story_len=story_len)
        inventory_kinds.add(span.semantic_kind)
        if _overlaps(
            start_scalar,
            end_scalar,
            span.start_scalar,
            span.end_scalar,
        ):
            _fail(
                "capture_unsupported",
                f"selection intersects unsupported semantic: {span.semantic_kind}",
            )

    missing_inventory = (
        set(state.unsupported_anchored_semantics) - inventory_kinds
    )
    if missing_inventory:
        _fail(
            "semantic_inventory_incomplete",
            "unsupported semantic classes lack authoritative span inventory",
        )

    # Base fragment carries character format only. Any generic persistent
    # semantic intersecting the selection is therefore explicit unsupported.
    for anchor in state.generic_anchors:
        if _overlaps(
            start_scalar,
            end_scalar,
            anchor.start_scalar,
            anchor.end_scalar,
        ):
            _fail(
                "capture_unsupported",
                f"selection intersects unsupported fragment semantic: {anchor.semantic_kind}",
            )

    text = state.paragraph_state.story_text[start_scalar:end_scalar]
    paragraph_semantics, diagnostics = _paragraph_semantics(
        state,
        start_scalar,
        end_scalar,
    )
    fragment = _validate_fragment(
        TextRangeFragmentV1(
            protocol_version="chaptera.text-range-fragment.v1",
            text=text,
            scalar_len=len(text),
            format_runs=_capture_effective_format_runs(
                state,
                start_scalar,
                end_scalar,
            ),
            paragraph_semantics=paragraph_semantics,
            diagnostics=diagnostics,
        )
    )
    return CaptureTextRangeReceiptV1(
        protocol_version="chaptera.capture-text-range-receipt.v1",
        source_story_id=state.story_id,
        source_start_scalar=start_scalar,
        source_end_scalar=end_scalar,
        fragment=fragment,
        fragment_sha256=text_range_fragment_sha256_v1(fragment),
    )


def build_paste_text_range_request_v1(
    *,
    fragment: TextRangeFragmentV1,
    document_id: str,
    source_hash: str,
    base_revision_id: str,
    client_operation_id: str,
    destination_story_id: str,
    destination_start_scalar: int,
    destination_end_scalar: int,
    expected_before: str,
    paragraph_inserted_ids: tuple[str, ...] = (),
    accept_paragraph_semantics_loss: bool = False,
) -> PasteTextRangePlanV1:
    fragment = _validate_fragment(fragment)
    for label, value in (
        ("document_id", document_id),
        ("source_hash", source_hash),
        ("base_revision_id", base_revision_id),
        ("client_operation_id", client_operation_id),
        ("destination_story_id", destination_story_id),
    ):
        if not isinstance(value, str) or not value:
            _fail("paste_invalid", f"{label} is required")
    if (
        not isinstance(destination_start_scalar, int)
        or isinstance(destination_start_scalar, bool)
        or not isinstance(destination_end_scalar, int)
        or isinstance(destination_end_scalar, bool)
        or destination_start_scalar < 0
        or destination_end_scalar < destination_start_scalar
    ):
        _fail("paste_invalid", "destination Story range is invalid")
    if not isinstance(expected_before, str):
        _fail("paste_invalid", "expected_before must be canonical string")
    if not isinstance(paragraph_inserted_ids, tuple):
        _fail("paste_invalid", "paragraph_inserted_ids must be tuple")
    if len(paragraph_inserted_ids) != fragment.text.count("\r"):
        _fail(
            "paste_invalid",
            "one preallocated ParagraphId is required per fragment U+000D",
        )
    if fragment.paragraph_semantics == "paragraph_semantics_loss":
        if not accept_paragraph_semantics_loss:
            _fail(
                "paragraph_semantics_loss",
                "paste requires explicit acceptance of paragraph-property loss",
            )

    request = {
        "protocol_version": "chaptera.story-edit-transaction-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": base_revision_id,
        "client_operation_id": client_operation_id,
        "command": {
            "kind": "story_edit_transaction",
            "story_id": destination_story_id,
            "start_scalar": destination_start_scalar,
            "end_scalar": destination_end_scalar,
            "expected_before": expected_before,
            "replacement_text": fragment.text,
            "paragraph_inserted_ids": list(paragraph_inserted_ids),
            "paragraph_inserted_property_presets": [],
            "typing_format": None,
            "fragment_format_runs": [
                {
                    "start_offset": run.start_offset,
                    "end_offset": run.end_offset,
                    "property": run.property,
                    "value": run.value,
                }
                for run in fragment.format_runs
            ],
            "incoming_semantic_kinds": [],
        },
    }
    return PasteTextRangePlanV1(
        protocol_version="chaptera.paste-text-range-plan.v1",
        request=request,
        fragment_sha256=text_range_fragment_sha256_v1(fragment),
        diagnostics=fragment.diagnostics,
    )
