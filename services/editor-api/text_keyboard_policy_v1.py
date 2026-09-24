#!/usr/bin/env python3
"""Versioned source-neutral text keyboard policy V1.

Canonical Story identity stays Unicode-scalar indexed. Logical Previous/Next and
Delete use one pinned Chaptera extended-grapheme profile; physical placement is
still admitted only by ResolvedTextCaretMapV1. Platform key events are adapters,
never durable document operations.
"""

from __future__ import annotations

from bisect import bisect_left, bisect_right
from dataclasses import dataclass
from typing import Literal
import unicodedata

from resolved_text_caret_map_v1 import (
    ResolvedTextCaretMapError,
    ResolvedTextCaretMapV1,
    resolve_story_position_v1,
)
from story_edit_domain_v1 import (
    StoryEditDomainError,
    StoryEditDomainV1,
    validate_ordinary_story_range_v1,
)
from story_edit_transaction_v1 import validate_story_edit_transaction_request_v1
from story_range_v1 import validate_scalar_sequence_v1
from text_selection_state_v1 import (
    TextSelectionStateError,
    TextSelectionStateV1,
    build_text_selection_state_v1,
    project_selection_state_v1,
    validate_selection_state_v1,
)


UNICODE_GRAPHEME_DATA_VERSION_V1 = "15.0.0"
GRAPHEME_PROFILE_V1 = (
    "chaptera.uax29-extended-grapheme.v1/unicode-" + UNICODE_GRAPHEME_DATA_VERSION_V1
)

KeyboardCommandV1 = Literal[
    "move_previous",
    "move_next",
    "extend_previous",
    "extend_next",
    "delete_backward",
    "delete_forward",
    "delete_selection",
]
KeyboardResultKindV1 = Literal["selection", "delete_intent", "boundary_noop"]


class TextKeyboardPolicyError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextKeyboardResultV1:
    protocol_version: Literal["chaptera.text-keyboard-result.v1"]
    command: KeyboardCommandV1
    result_kind: KeyboardResultKindV1
    grapheme_profile: str
    selection: TextSelectionStateV1 | None
    request: dict | None
    delete_start_scalar: int | None
    delete_end_scalar: int | None


def _fail(code: str, message: str) -> None:
    raise TextKeyboardPolicyError(code, message)


def _require_unicode_profile_v1() -> None:
    if unicodedata.unidata_version != UNICODE_GRAPHEME_DATA_VERSION_V1:
        _fail(
            "unicode_profile_mismatch",
            "TextKeyboardPolicyV1 requires Unicode data "
            + UNICODE_GRAPHEME_DATA_VERSION_V1
            + ", runtime has "
            + unicodedata.unidata_version,
        )


def _cp(ch: str) -> int:
    return ord(ch)


def _is_cr(ch: str) -> bool:
    return ch == "\r"


def _is_lf(ch: str) -> bool:
    return ch == "\n"


def _is_zwj(ch: str) -> bool:
    return _cp(ch) == 0x200D


def _is_zwnj(ch: str) -> bool:
    return _cp(ch) == 0x200C


def _is_regional_indicator(ch: str) -> bool:
    value = _cp(ch)
    return 0x1F1E6 <= value <= 0x1F1FF


def _is_emoji_modifier(ch: str) -> bool:
    value = _cp(ch)
    return 0x1F3FB <= value <= 0x1F3FF


def _is_variation_selector(ch: str) -> bool:
    value = _cp(ch)
    return 0xFE00 <= value <= 0xFE0F or 0xE0100 <= value <= 0xE01EF


def _is_extend(ch: str) -> bool:
    if _is_zwnj(ch) or _is_variation_selector(ch) or _is_emoji_modifier(ch):
        return True
    category = unicodedata.category(ch)
    return category in {"Mn", "Me"} or unicodedata.combining(ch) != 0


_PREPEND_RANGES = (
    (0x0600, 0x0605),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x0D4E, 0x0D4E),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x111C2, 0x111C3),
    (0x1193F, 0x1193F),
    (0x11941, 0x11941),
    (0x11A3A, 0x11A3A),
    (0x11A84, 0x11A89),
    (0x11D46, 0x11D46),
    (0x11F02, 0x11F02),
)


def _is_prepend(ch: str) -> bool:
    value = _cp(ch)
    return any(start <= value <= end for start, end in _PREPEND_RANGES)


def _is_spacing_mark(ch: str) -> bool:
    # UAX #29 GCB=SpacingMark is close to general category Mc for the V1
    # horizontal-LTR profile. Exceptions that matter to future full Unicode
    # conformance must extend this versioned table rather than inherit a widget.
    return unicodedata.category(ch) == "Mc"


def _is_control(ch: str) -> bool:
    if _is_cr(ch) or _is_lf(ch) or _is_zwj(ch) or _is_zwnj(ch) or _is_prepend(ch):
        return False
    return unicodedata.category(ch) in {"Cc", "Cf", "Cs", "Co", "Cn"}


def _hangul_class(ch: str) -> str | None:
    value = _cp(ch)
    if 0x1100 <= value <= 0x115F or 0xA960 <= value <= 0xA97C:
        return "L"
    if 0x1160 <= value <= 0x11A7 or 0xD7B0 <= value <= 0xD7C6:
        return "V"
    if 0x11A8 <= value <= 0x11FF or 0xD7CB <= value <= 0xD7FB:
        return "T"
    if 0xAC00 <= value <= 0xD7A3:
        return "LV" if (value - 0xAC00) % 28 == 0 else "LVT"
    return None


_EXTENDED_PICTOGRAPHIC_RANGES = (
    (0x00A9, 0x00A9),
    (0x00AE, 0x00AE),
    (0x203C, 0x203C),
    (0x2049, 0x2049),
    (0x2122, 0x2122),
    (0x2139, 0x2139),
    (0x2194, 0x21FF),
    (0x2300, 0x23FF),
    (0x2460, 0x24FF),
    (0x25A0, 0x27BF),
    (0x2934, 0x2935),
    (0x2B00, 0x2BFF),
    (0x3030, 0x3030),
    (0x303D, 0x303D),
    (0x3297, 0x3297),
    (0x3299, 0x3299),
    (0x1F000, 0x1FAFF),
)


def _is_extended_pictographic(ch: str) -> bool:
    value = _cp(ch)
    return any(start <= value <= end for start, end in _EXTENDED_PICTOGRAPHIC_RANGES)


def _should_break_v1(text: str, index: int) -> bool:
    """Return whether an EGC boundary exists before text[index]."""
    previous = text[index - 1]
    current = text[index]

    # GB3 / GB4 / GB5.
    if _is_cr(previous) and _is_lf(current):
        return False
    if _is_cr(previous) or _is_lf(previous) or _is_control(previous):
        return True
    if _is_cr(current) or _is_lf(current) or _is_control(current):
        return True

    # GB6..GB8 Hangul syllable composition.
    left = _hangul_class(previous)
    right = _hangul_class(current)
    if left == "L" and right in {"L", "V", "LV", "LVT"}:
        return False
    if left in {"LV", "V"} and right in {"V", "T"}:
        return False
    if left in {"LVT", "T"} and right == "T":
        return False

    # GB9 / GB9a / GB9b.
    if _is_extend(current) or _is_zwj(current):
        return False
    if _is_spacing_mark(current):
        return False
    if _is_prepend(previous):
        return False

    # GB11: Extended_Pictographic Extend* ZWJ × Extended_Pictographic.
    if _is_extended_pictographic(current) and _is_zwj(previous):
        cursor = index - 2
        while cursor >= 0 and _is_extend(text[cursor]):
            cursor -= 1
        if cursor >= 0 and _is_extended_pictographic(text[cursor]):
            return False

    # GB12/GB13: pair Regional Indicators from the start of the RI run.
    if _is_regional_indicator(previous) and _is_regional_indicator(current):
        count = 0
        cursor = index - 1
        while cursor >= 0 and _is_regional_indicator(text[cursor]):
            count += 1
            cursor -= 1
        if count % 2 == 1:
            return False

    return True


def grapheme_boundaries_v1(text: str) -> tuple[int, ...]:
    """Pinned Chaptera logical EGC boundaries in Unicode-scalar coordinates.

    V1 is deliberately horizontal/logical. Visual bidi order is outside this
    task. The algorithm encodes the UAX #29 rules needed by the admitted V1
    profile and fails independently of browser/native-widget deletion behavior.
    """
    _require_unicode_profile_v1()
    try:
        validate_scalar_sequence_v1(text, "story_text")
    except ValueError as exc:
        _fail("invalid_story", str(exc))

    if not text:
        return (0,)
    boundaries = [0]
    for index in range(1, len(text)):
        if _should_break_v1(text, index):
            boundaries.append(index)
    boundaries.append(len(text))
    return tuple(boundaries)


def _validate_common(
    *,
    story_text: str,
    domain: StoryEditDomainV1,
    selection: TextSelectionStateV1,
    caret_map: ResolvedTextCaretMapV1,
) -> tuple[int, int, tuple[int, ...]]:
    try:
        scalar_len = validate_scalar_sequence_v1(story_text, "story_text")
    except ValueError as exc:
        _fail("invalid_story", str(exc))
    if (
        domain.story_id != selection.story_id
        or caret_map.story_id != selection.story_id
    ):
        _fail("story_mismatch", "keyboard inputs target different Stories")
    if domain.raw_scalar_len != scalar_len or caret_map.story_scalar_len != scalar_len:
        _fail("reconcile_required", "Story extent differs across keyboard inputs")
    try:
        validate_selection_state_v1(
            state=selection,
            domain=domain,
            expected_revision_id=selection.revision_id,
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))
    if domain.status != "known":
        _fail("edit_domain_unknown", "ordinary Story keyboard domain is unknown")
    assert domain.caret_start_boundary is not None
    assert domain.caret_end_boundary is not None
    boundaries = tuple(
        value
        for value in grapheme_boundaries_v1(story_text)
        if domain.caret_start_boundary <= value <= domain.caret_end_boundary
    )
    if not boundaries:
        boundaries = (domain.caret_start_boundary,)
    return domain.caret_start_boundary, domain.caret_end_boundary, boundaries


def _resolve_physical_stop(
    *,
    caret_map: ResolvedTextCaretMapV1,
    scalar: int,
    stop_id: str | None = None,
):
    try:
        return resolve_story_position_v1(
            caret_map=caret_map,
            scalar_boundary=scalar,
            stop_id=stop_id,
            expected_layout_revision_id=caret_map.layout_revision_id,
        )
    except ResolvedTextCaretMapError as exc:
        if exc.code == "caret_affinity_required":
            _fail(exc.code, str(exc))
        if exc.code == "stale_layout_map":
            _fail("reconcile_required", str(exc))
        _fail(
            "caret_geometry_unsupported",
            f"logical grapheme boundary {scalar} has no admitted physical caret: {exc}",
        )


def _project_new_selection(
    *,
    domain: StoryEditDomainV1,
    previous: TextSelectionStateV1,
    caret_map: ResolvedTextCaretMapV1,
    anchor: int,
    focus: int,
    focus_stop_id: str,
    collapse: bool,
) -> TextSelectionStateV1:
    pending = build_text_selection_state_v1(
        domain=domain,
        revision_id=previous.revision_id,
        anchor_scalar=anchor,
        focus_scalar=focus,
        preferred_inline_x_emu=None,
    )
    anchor_stop_id = focus_stop_id if collapse else (
        previous.anchor_visual_stop_id
        if previous.projection_state == "projected"
        and previous.layout_revision_id == caret_map.layout_revision_id
        and previous.anchor_scalar == anchor
        else None
    )
    try:
        projection = project_selection_state_v1(
            state=pending,
            domain=domain,
            caret_map=caret_map,
            anchor_stop_id=anchor_stop_id,
            focus_stop_id=focus_stop_id,
        )
    except TextSelectionStateError as exc:
        if exc.code in {
            "internal_cluster_unsupported",
            "unplaced_story_position",
            "unplaced_selection",
        }:
            _fail("caret_geometry_unsupported", str(exc))
        _fail(exc.code, str(exc))
    return projection.state


def _previous_boundary(boundaries: tuple[int, ...], scalar: int) -> int | None:
    index = bisect_left(boundaries, scalar)
    if index <= 0:
        return None
    return boundaries[index - 1]


def _next_boundary(boundaries: tuple[int, ...], scalar: int) -> int | None:
    index = bisect_right(boundaries, scalar)
    if index >= len(boundaries):
        return None
    return boundaries[index]


def _delete_request(
    *,
    story_text: str,
    story_id: str,
    start: int,
    end: int,
    document_id: str,
    source_hash: str,
    base_revision_id: str,
    client_operation_id: str,
) -> dict:
    if not all(isinstance(value, str) and value for value in (
        document_id,
        source_hash,
        base_revision_id,
        client_operation_id,
    )):
        _fail("invalid_delete_context", "durable delete request identity is required")
    request = {
        "protocol_version": "chaptera.story-edit-transaction-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": base_revision_id,
        "client_operation_id": client_operation_id,
        "depends_on_client_operation_id": None,
        "command": {
            "kind": "story_edit_transaction",
            "story_id": story_id,
            "start_scalar": start,
            "end_scalar": end,
            "expected_before": story_text[start:end],
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
        _fail("invalid_delete_context", str(exc))
    return request


def apply_text_keyboard_policy_v1(
    *,
    command: KeyboardCommandV1,
    story_text: str,
    domain: StoryEditDomainV1,
    selection: TextSelectionStateV1,
    caret_map: ResolvedTextCaretMapV1,
    document_id: str | None = None,
    source_hash: str | None = None,
    client_operation_id: str | None = None,
) -> TextKeyboardResultV1:
    if command not in {
        "move_previous",
        "move_next",
        "extend_previous",
        "extend_next",
        "delete_backward",
        "delete_forward",
        "delete_selection",
    }:
        _fail("unsupported_keyboard_command", "command is outside TextKeyboardPolicyV1")

    start_boundary, end_boundary, boundaries = _validate_common(
        story_text=story_text,
        domain=domain,
        selection=selection,
        caret_map=caret_map,
    )

    # Navigation is transient only.
    if command in {
        "move_previous",
        "move_next",
        "extend_previous",
        "extend_next",
    }:
        extending = command.startswith("extend_")
        previous_direction = command.endswith("previous")

        if not extending and not selection.is_collapsed:
            target = min(selection.anchor_scalar, selection.focus_scalar) if previous_direction else max(
                selection.anchor_scalar, selection.focus_scalar
            )
        else:
            current = selection.focus_scalar
            target = (
                _previous_boundary(boundaries, current)
                if previous_direction
                else _next_boundary(boundaries, current)
            )
            if target is None:
                return TextKeyboardResultV1(
                    protocol_version="chaptera.text-keyboard-result.v1",
                    command=command,
                    result_kind="boundary_noop",
                    grapheme_profile=GRAPHEME_PROFILE_V1,
                    selection=selection,
                    request=None,
                    delete_start_scalar=None,
                    delete_end_scalar=None,
                )

        if not start_boundary <= target <= end_boundary:
            _fail("reconcile_required", "navigation target lies outside ordinary caret domain")
        stop = _resolve_physical_stop(caret_map=caret_map, scalar=target)
        new_anchor = selection.anchor_scalar if extending else target
        new_focus = target
        projected = _project_new_selection(
            domain=domain,
            previous=selection,
            caret_map=caret_map,
            anchor=new_anchor,
            focus=new_focus,
            focus_stop_id=stop.stop_id,
            collapse=not extending,
        )
        return TextKeyboardResultV1(
            protocol_version="chaptera.text-keyboard-result.v1",
            command=command,
            result_kind="selection",
            grapheme_profile=GRAPHEME_PROFILE_V1,
            selection=projected,
            request=None,
            delete_start_scalar=None,
            delete_end_scalar=None,
        )

    # Deletion lowers to one canonical StoryEditTransactionV1 request.
    if not selection.is_collapsed:
        delete_start = min(selection.anchor_scalar, selection.focus_scalar)
        delete_end = max(selection.anchor_scalar, selection.focus_scalar)
    else:
        caret = selection.focus_scalar
        if caret not in boundaries:
            _fail(
                "reconcile_required",
                "collapsed keyboard delete starts from a non-grapheme Story boundary",
            )
        if command == "delete_selection":
            return TextKeyboardResultV1(
                protocol_version="chaptera.text-keyboard-result.v1",
                command=command,
                result_kind="boundary_noop",
                grapheme_profile=GRAPHEME_PROFILE_V1,
                selection=selection,
                request=None,
                delete_start_scalar=None,
                delete_end_scalar=None,
            )
        if command == "delete_backward":
            target = _previous_boundary(boundaries, caret)
            if target is None:
                return TextKeyboardResultV1(
                    protocol_version="chaptera.text-keyboard-result.v1",
                    command=command,
                    result_kind="boundary_noop",
                    grapheme_profile=GRAPHEME_PROFILE_V1,
                    selection=selection,
                    request=None,
                    delete_start_scalar=None,
                    delete_end_scalar=None,
                )
            delete_start, delete_end = target, caret
        else:
            target = _next_boundary(boundaries, caret)
            if target is None:
                return TextKeyboardResultV1(
                    protocol_version="chaptera.text-keyboard-result.v1",
                    command=command,
                    result_kind="boundary_noop",
                    grapheme_profile=GRAPHEME_PROFILE_V1,
                    selection=selection,
                    request=None,
                    delete_start_scalar=None,
                    delete_end_scalar=None,
                )
            delete_start, delete_end = caret, target

    try:
        validate_ordinary_story_range_v1(
            domain=domain,
            start_scalar=delete_start,
            end_scalar=delete_end,
        )
    except StoryEditDomainError as exc:
        _fail(exc.code, str(exc))

    # Both selection endpoints must be physically grounded before a keyboard
    # edit is admitted. This prevents overset/unplaced/widget-only geometry
    # from silently becoming canonical deletion authority.
    for scalar, stop_id in (
        (
            delete_start,
            selection.anchor_visual_stop_id
            if selection.anchor_scalar == delete_start
            and selection.projection_state == "projected"
            else None,
        ),
        (
            delete_end,
            selection.focus_visual_stop_id
            if selection.focus_scalar == delete_end
            and selection.projection_state == "projected"
            else None,
        ),
    ):
        _resolve_physical_stop(
            caret_map=caret_map,
            scalar=scalar,
            stop_id=stop_id,
        )

    request = _delete_request(
        story_text=story_text,
        story_id=selection.story_id,
        start=delete_start,
        end=delete_end,
        document_id=document_id or "",
        source_hash=source_hash or "",
        base_revision_id=selection.revision_id,
        client_operation_id=client_operation_id or "",
    )
    return TextKeyboardResultV1(
        protocol_version="chaptera.text-keyboard-result.v1",
        command=command,
        result_kind="delete_intent",
        grapheme_profile=GRAPHEME_PROFILE_V1,
        selection=selection,
        request=request,
        delete_start_scalar=delete_start,
        delete_end_scalar=delete_end,
    )
