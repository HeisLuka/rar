#!/usr/bin/env python3
"""Source-neutral Unicode keyboard navigation/delete policy V1.

Keyboard events are gestures, not durable document operations.

This module owns only logical Previous/Next/Extend/Delete semantics for the
current horizontal-LTR Story interaction slice. Durable deletion lowers to one
canonical ReplaceStoryRange intent; movement/extension return transient
TextSelectionStateV1 only.

Grapheme policy is deliberately pinned to Unicode 15.0.0, matching the Python
3.12 stdlib tables used by this service. The implementation covers the V1
interaction profile required by Chaptera: CR/LF, combining/spacing marks,
variation selectors, emoji modifiers, regional-indicator pairs, and
extended-pictographic ZWJ sequences. Unsupported physical caret positions still
fail closed through ResolvedTextCaretMapV1; logical grapheme boundaries never
invent layout geometry.
"""

from __future__ import annotations

from dataclasses import dataclass
import unicodedata
from typing import Literal

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
from story_range_v1 import validate_scalar_sequence_v1
from text_selection_state_v1 import (
    TextSelectionStateError,
    TextSelectionStateV1,
    build_text_selection_state_v1,
    validate_selection_state_v1,
)


UNICODE_GRAPHEME_VERSION = "15.0.0"
KEYBOARD_POLICY_VERSION = "chaptera.text-keyboard-policy.v1"

KeyboardCommandV1 = Literal[
    "move_previous",
    "move_next",
    "extend_previous",
    "extend_next",
    "delete_backward",
    "delete_forward",
]


class TextKeyboardPolicyError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class ReplaceStoryRangeIntentV1:
    protocol_version: Literal["chaptera.replace-story-range-intent.v1"]
    story_id: str
    start_scalar: int
    end_scalar: int
    replacement_text: Literal[""]


@dataclass(frozen=True)
class TextKeyboardDecisionV1:
    protocol_version: Literal["chaptera.text-keyboard-decision.v1"]
    command: KeyboardCommandV1
    action: Literal["selection", "delete", "boundary_noop"]
    selection: TextSelectionStateV1 | None
    delete_intent: ReplaceStoryRangeIntentV1 | None
    grapheme_version: str


def _fail(code: str, message: str) -> None:
    raise TextKeyboardPolicyError(code, message)


def _require_unicode_version() -> None:
    if unicodedata.unidata_version != UNICODE_GRAPHEME_VERSION:
        _fail(
            "unicode_version_mismatch",
            "TextKeyboardPolicyV1 requires Unicode "
            + UNICODE_GRAPHEME_VERSION
            + " tables; runtime provides "
            + unicodedata.unidata_version,
        )


def _is_control(ch: str) -> bool:
    cp = ord(ch)
    if cp in {0x000D, 0x000A}:
        return True
    return unicodedata.category(ch) in {"Cc", "Cf", "Cs", "Co", "Cn"} and cp != 0x200D


def _is_extend(ch: str) -> bool:
    cp = ord(ch)
    category = unicodedata.category(ch)
    return (
        category in {"Mn", "Me"}
        or 0xFE00 <= cp <= 0xFE0F
        or 0xE0100 <= cp <= 0xE01EF
        or 0x1F3FB <= cp <= 0x1F3FF
    )


def _is_spacing_mark(ch: str) -> bool:
    return unicodedata.category(ch) == "Mc"


def _is_zwj(ch: str) -> bool:
    return ord(ch) == 0x200D


def _is_regional_indicator(ch: str) -> bool:
    return 0x1F1E6 <= ord(ch) <= 0x1F1FF


def _is_prepend(ch: str) -> bool:
    cp = ord(ch)
    return (
        0x0600 <= cp <= 0x0605
        or cp == 0x06DD
        or cp == 0x070F
        or 0x0890 <= cp <= 0x0891
        or cp == 0x08E2
        or cp == 0x0D4E
        or cp == 0x110BD
        or cp == 0x110CD
        or 0x111C2 <= cp <= 0x111C3
        or cp == 0x1193F
        or cp == 0x11941
        or cp == 0x11A3A
        or 0x11A84 <= cp <= 0x11A89
        or cp == 0x11D46
    )


def _is_extended_pictographic(ch: str) -> bool:
    cp = ord(ch)
    return (
        0x1F000 <= cp <= 0x1FAFF
        or 0x1FC00 <= cp <= 0x1FFFD
        or 0x2600 <= cp <= 0x27BF
        or cp in {
            0x00A9,
            0x00AE,
            0x203C,
            0x2049,
            0x2122,
            0x2139,
            0x3030,
            0x303D,
            0x3297,
            0x3299,
        }
    )


def _gb11_no_break(text: str, boundary: int) -> bool:
    # Extended_Pictographic Extend* ZWJ × Extended_Pictographic
    if boundary <= 0 or boundary >= len(text):
        return False
    if not _is_extended_pictographic(text[boundary]):
        return False
    left = boundary - 1
    if not _is_zwj(text[left]):
        return False
    left -= 1
    while left >= 0 and _is_extend(text[left]):
        left -= 1
    return left >= 0 and _is_extended_pictographic(text[left])


def grapheme_boundaries_v1(text: str) -> tuple[int, ...]:
    _require_unicode_version()
    try:
        validate_scalar_sequence_v1(text, "story_text")
    except Exception as exc:
        _fail("invalid_story", str(exc))

    if not text:
        return (0,)

    boundaries = [0]
    ri_run = 1 if _is_regional_indicator(text[0]) else 0

    for boundary in range(1, len(text)):
        left = text[boundary - 1]
        right = text[boundary]
        should_break = True

        # GB3
        if left == "\r" and right == "\n":
            should_break = False
        # GB4/GB5
        elif _is_control(left) or _is_control(right):
            should_break = True
        # GB9 / GB9a
        elif _is_extend(right) or _is_zwj(right) or _is_spacing_mark(right):
            should_break = False
        # GB9b
        elif _is_prepend(left):
            should_break = False
        # GB11
        elif _gb11_no_break(text, boundary):
            should_break = False
        # GB12/GB13: pair regional indicators.
        elif _is_regional_indicator(left) and _is_regional_indicator(right):
            should_break = (ri_run % 2 == 0)

        if should_break:
            boundaries.append(boundary)

        if _is_regional_indicator(right):
            ri_run = ri_run + 1 if _is_regional_indicator(left) else 1
        else:
            ri_run = 0

    boundaries.append(len(text))
    return tuple(boundaries)


def _require_context(
    *,
    story_text: str,
    domain: StoryEditDomainV1,
    selection: TextSelectionStateV1,
    caret_map: ResolvedTextCaretMapV1,
    expected_revision_id: str,
) -> tuple[int, ...]:
    _require_unicode_version()
    if domain.story_id != selection.story_id or caret_map.story_id != selection.story_id:
        _fail("story_mismatch", "keyboard inputs target different Stories")
    if len(story_text) != domain.raw_scalar_len:
        _fail("stale_story", "Story text length disagrees with StoryEditDomainV1")
    if caret_map.story_scalar_len != domain.raw_scalar_len:
        _fail("stale_layout_map", "caret map Story length disagrees with current domain")
    try:
        validate_selection_state_v1(
            state=selection,
            domain=domain,
            expected_revision_id=expected_revision_id,
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))
    return grapheme_boundaries_v1(story_text)


def _require_physical_boundary(
    *,
    caret_map: ResolvedTextCaretMapV1,
    scalar_boundary: int,
    stop_id: str | None,
) -> None:
    try:
        resolve_story_position_v1(
            caret_map=caret_map,
            scalar_boundary=scalar_boundary,
            stop_id=stop_id,
            expected_layout_revision_id=caret_map.layout_revision_id,
        )
    except ResolvedTextCaretMapError as exc:
        if exc.code in {
            "internal_cluster_unsupported",
            "unplaced_story_position",
            "caret_affinity_required",
            "invalid_caret_affinity",
            "stale_layout_map",
        }:
            _fail("caret_geometry_unsupported", str(exc))
        _fail(exc.code, str(exc))


def _previous_boundary(boundaries: tuple[int, ...], scalar: int, floor: int) -> int:
    candidates = [item for item in boundaries if floor <= item < scalar]
    return max(candidates) if candidates else scalar


def _next_boundary(boundaries: tuple[int, ...], scalar: int, ceiling: int) -> int:
    candidates = [item for item in boundaries if scalar < item <= ceiling]
    return min(candidates) if candidates else scalar


def _selection_decision(
    *,
    command: KeyboardCommandV1,
    domain: StoryEditDomainV1,
    selection: TextSelectionStateV1,
    revision_id: str,
    anchor: int,
    focus: int,
) -> TextKeyboardDecisionV1:
    state = build_text_selection_state_v1(
        domain=domain,
        revision_id=revision_id,
        anchor_scalar=anchor,
        focus_scalar=focus,
    )
    return TextKeyboardDecisionV1(
        protocol_version="chaptera.text-keyboard-decision.v1",
        command=command,
        action="selection",
        selection=state,
        delete_intent=None,
        grapheme_version=UNICODE_GRAPHEME_VERSION,
    )


def _delete_decision(
    *,
    command: KeyboardCommandV1,
    domain: StoryEditDomainV1,
    start: int,
    end: int,
) -> TextKeyboardDecisionV1:
    if start == end:
        return TextKeyboardDecisionV1(
            protocol_version="chaptera.text-keyboard-decision.v1",
            command=command,
            action="boundary_noop",
            selection=None,
            delete_intent=None,
            grapheme_version=UNICODE_GRAPHEME_VERSION,
        )
    try:
        validate_ordinary_story_range_v1(
            domain=domain,
            start_scalar=start,
            end_scalar=end,
        )
    except StoryEditDomainError as exc:
        _fail(exc.code, str(exc))
    return TextKeyboardDecisionV1(
        protocol_version="chaptera.text-keyboard-decision.v1",
        command=command,
        action="delete",
        selection=None,
        delete_intent=ReplaceStoryRangeIntentV1(
            protocol_version="chaptera.replace-story-range-intent.v1",
            story_id=domain.story_id,
            start_scalar=start,
            end_scalar=end,
            replacement_text="",
        ),
        grapheme_version=UNICODE_GRAPHEME_VERSION,
    )


def apply_text_keyboard_policy_v1(
    *,
    command: KeyboardCommandV1,
    story_text: str,
    domain: StoryEditDomainV1,
    selection: TextSelectionStateV1,
    caret_map: ResolvedTextCaretMapV1,
    expected_revision_id: str,
) -> TextKeyboardDecisionV1:
    if command not in {
        "move_previous",
        "move_next",
        "extend_previous",
        "extend_next",
        "delete_backward",
        "delete_forward",
    }:
        _fail("unsupported_keyboard_command", "command is outside TextKeyboardPolicyV1")

    boundaries = _require_context(
        story_text=story_text,
        domain=domain,
        selection=selection,
        caret_map=caret_map,
        expected_revision_id=expected_revision_id,
    )
    assert domain.caret_start_boundary is not None
    assert domain.caret_end_boundary is not None
    floor = domain.caret_start_boundary
    ceiling = domain.caret_end_boundary

    # The active focus must itself be a physically admitted caret position.
    _require_physical_boundary(
        caret_map=caret_map,
        scalar_boundary=selection.focus_scalar,
        stop_id=selection.focus_visual_stop_id,
    )

    start, end = selection.normalized_range

    if command == "move_previous":
        target = (
            start
            if not selection.is_collapsed
            else _previous_boundary(boundaries, selection.focus_scalar, floor)
        )
        _require_physical_boundary(caret_map=caret_map, scalar_boundary=target, stop_id=None)
        return _selection_decision(
            command=command,
            domain=domain,
            selection=selection,
            revision_id=expected_revision_id,
            anchor=target,
            focus=target,
        )

    if command == "move_next":
        target = (
            end
            if not selection.is_collapsed
            else _next_boundary(boundaries, selection.focus_scalar, ceiling)
        )
        _require_physical_boundary(caret_map=caret_map, scalar_boundary=target, stop_id=None)
        return _selection_decision(
            command=command,
            domain=domain,
            selection=selection,
            revision_id=expected_revision_id,
            anchor=target,
            focus=target,
        )

    if command in {"extend_previous", "extend_next"}:
        if command == "extend_previous":
            target = _previous_boundary(boundaries, selection.focus_scalar, floor)
        else:
            target = _next_boundary(boundaries, selection.focus_scalar, ceiling)
        _require_physical_boundary(caret_map=caret_map, scalar_boundary=target, stop_id=None)
        return _selection_decision(
            command=command,
            domain=domain,
            selection=selection,
            revision_id=expected_revision_id,
            anchor=selection.anchor_scalar,
            focus=target,
        )

    if not selection.is_collapsed:
        return _delete_decision(
            command=command,
            domain=domain,
            start=start,
            end=end,
        )

    if command == "delete_backward":
        target = _previous_boundary(boundaries, selection.focus_scalar, floor)
        if target == selection.focus_scalar:
            return _delete_decision(
                command=command,
                domain=domain,
                start=target,
                end=target,
            )
        _require_physical_boundary(caret_map=caret_map, scalar_boundary=target, stop_id=None)
        return _delete_decision(
            command=command,
            domain=domain,
            start=target,
            end=selection.focus_scalar,
        )

    target = _next_boundary(boundaries, selection.focus_scalar, ceiling)
    if target == selection.focus_scalar:
        return _delete_decision(
            command=command,
            domain=domain,
            start=target,
            end=target,
        )
    _require_physical_boundary(caret_map=caret_map, scalar_boundary=target, stop_id=None)
    return _delete_decision(
        command=command,
        domain=domain,
        start=selection.focus_scalar,
        end=target,
    )
