#!/usr/bin/env python3
"""Transient collapsed-caret typing-format state V1.

Typing format is an interaction intention, never a zero-length durable format
span. Only explicit pending Chaptera character-property overrides are stored.
Absent properties inherit from canonical formatting when text actually commits.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Any, Literal

from story_edit_domain_v1 import StoryEditDomainV1
from text_format_overlay_v1 import (
    FormatPropertyV1,
    TextFormatOverlayStateV1,
    effective_property_segments_v1,
    state_hash_v1,
)
from text_insert_format_v1 import TypingFormatSnapshotV1
from text_selection_state_v1 import (
    TextSelectionStateV1,
    edit_domain_id_v1,
    validate_selection_state_v1,
)


_ALLOWED_PROPERTIES = {
    "font_size_emu",
    "bold",
    "italic",
    "text_color_rgb",
}


class TextTypingFormatStateError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextTypingFormatStateV1:
    protocol_version: Literal["chaptera.text-typing-format-state.v1"]
    story_id: str
    revision_id: str
    caret_scalar: int
    edit_domain_id: str
    format_state_hash: str
    pending_explicit_properties: tuple[tuple[FormatPropertyV1, Any], ...]


def _fail(code: str, message: str) -> None:
    raise TextTypingFormatStateError(code, message)


def _validate_value(prop: str, value: Any) -> Any:
    if prop not in _ALLOWED_PROPERTIES:
        _fail("unsupported_typing_property", "unsupported typing-format property")
    if prop == "font_size_emu":
        if (
            not isinstance(value, int)
            or isinstance(value, bool)
            or value <= 0
            or value > 9_007_199_254_740_991
        ):
            _fail("invalid_typing_value", "font_size_emu must be positive JS-safe integer")
        return value
    if prop in {"bold", "italic"}:
        if not isinstance(value, bool):
            _fail("invalid_typing_value", f"{prop} must be boolean")
        return value
    if (
        not isinstance(value, str)
        or len(value) != 7
        or not value.startswith("#")
        or any(ch not in "0123456789abcdefABCDEF" for ch in value[1:])
    ):
        _fail("invalid_typing_value", "text_color_rgb must be #RRGGBB")
    return value.upper()


def _canonical_pending(
    items: tuple[tuple[FormatPropertyV1, Any], ...],
) -> tuple[tuple[FormatPropertyV1, Any], ...]:
    if not isinstance(items, tuple):
        _fail("invalid_typing_state", "pending properties must be tuple")
    seen = set()
    out = []
    for pair in items:
        if not isinstance(pair, tuple) or len(pair) != 2:
            _fail("invalid_typing_state", "pending property entry is malformed")
        prop, value = pair
        if prop in seen:
            _fail("invalid_typing_state", "pending property names must be unique")
        seen.add(prop)
        out.append((prop, _validate_value(prop, value)))
    out.sort(key=lambda pair: pair[0])
    return tuple(out)


def _validate_context(
    *,
    selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    format_state: TextFormatOverlayStateV1,
) -> None:
    validate_selection_state_v1(
        state=selection,
        domain=domain,
        expected_revision_id=selection.revision_id,
    )
    if selection.story_id != format_state.story_id:
        _fail("typing_context_mismatch", "selection and format state target different Stories")
    if not selection.is_collapsed:
        _fail("noncollapsed_selection", "typing format exists only at collapsed caret")
    if selection.anchor_scalar != selection.focus_scalar:
        _fail("noncollapsed_selection", "collapsed caret is required")
    if selection.edit_domain_id != edit_domain_id_v1(domain):
        _fail("typing_context_mismatch", "selection edit-domain fence is stale")
    if selection.anchor_scalar > format_state.story_scalar_len:
        _fail("typing_context_mismatch", "caret lies outside format Story extent")


def derive_typing_format_state_v1(
    *,
    selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    format_state: TextFormatOverlayStateV1,
) -> TextTypingFormatStateV1 | None:
    """Create empty pending state for a collapsed caret; noncollapsed => no typing state."""
    validate_selection_state_v1(
        state=selection,
        domain=domain,
        expected_revision_id=selection.revision_id,
    )
    if not selection.is_collapsed:
        return None
    _validate_context(selection=selection, domain=domain, format_state=format_state)
    return TextTypingFormatStateV1(
        protocol_version="chaptera.text-typing-format-state.v1",
        story_id=selection.story_id,
        revision_id=selection.revision_id,
        caret_scalar=selection.focus_scalar,
        edit_domain_id=selection.edit_domain_id,
        format_state_hash=state_hash_v1(format_state),
        pending_explicit_properties=(),
    )


def validate_typing_format_state_v1(
    *,
    state: TextTypingFormatStateV1,
    selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    format_state: TextFormatOverlayStateV1,
) -> None:
    if not isinstance(state, TextTypingFormatStateV1):
        _fail("invalid_typing_state", "TextTypingFormatStateV1 is required")
    if state.protocol_version != "chaptera.text-typing-format-state.v1":
        _fail("invalid_typing_state", "typing-state protocol mismatch")
    _validate_context(selection=selection, domain=domain, format_state=format_state)
    if (
        state.story_id != selection.story_id
        or state.revision_id != selection.revision_id
        or state.caret_scalar != selection.focus_scalar
        or state.edit_domain_id != selection.edit_domain_id
        or state.format_state_hash != state_hash_v1(format_state)
    ):
        _fail("typing_context_changed", "typing state belongs to a different insertion context")
    _canonical_pending(state.pending_explicit_properties)


def set_pending_typing_property_v1(
    *,
    state: TextTypingFormatStateV1,
    prop: FormatPropertyV1,
    value: Any,
) -> TextTypingFormatStateV1:
    normalized = _validate_value(prop, value)
    pending = dict(state.pending_explicit_properties)
    pending[prop] = normalized
    return replace(
        state,
        pending_explicit_properties=_canonical_pending(tuple(pending.items())),
    )


def clear_pending_typing_property_v1(
    *,
    state: TextTypingFormatStateV1,
    prop: FormatPropertyV1,
) -> TextTypingFormatStateV1:
    if prop not in _ALLOWED_PROPERTIES:
        _fail("unsupported_typing_property", "unsupported typing-format property")
    pending = dict(state.pending_explicit_properties)
    pending.pop(prop, None)
    return replace(
        state,
        pending_explicit_properties=_canonical_pending(tuple(pending.items())),
    )


def clear_typing_state_for_context_change_v1(
    *,
    selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    format_state: TextFormatOverlayStateV1,
) -> TextTypingFormatStateV1 | None:
    """Pointer/nav/focus/history/reconcile changes clear explicit pending state."""
    return derive_typing_format_state_v1(
        selection=selection,
        domain=domain,
        format_state=format_state,
    )


def advance_typing_state_after_continuous_insert_v1(
    *,
    state: TextTypingFormatStateV1,
    previous_selection: TextSelectionStateV1,
    previous_domain: StoryEditDomainV1,
    previous_format_state: TextFormatOverlayStateV1,
    resulting_selection: TextSelectionStateV1,
    resulting_domain: StoryEditDomainV1,
    resulting_format_state: TextFormatOverlayStateV1,
) -> TextTypingFormatStateV1:
    """Preserve explicit pending properties only for caller-proven continuous typing."""
    validate_typing_format_state_v1(
        state=state,
        selection=previous_selection,
        domain=previous_domain,
        format_state=previous_format_state,
    )
    _validate_context(
        selection=resulting_selection,
        domain=resulting_domain,
        format_state=resulting_format_state,
    )
    return TextTypingFormatStateV1(
        protocol_version="chaptera.text-typing-format-state.v1",
        story_id=resulting_selection.story_id,
        revision_id=resulting_selection.revision_id,
        caret_scalar=resulting_selection.focus_scalar,
        edit_domain_id=resulting_selection.edit_domain_id,
        format_state_hash=state_hash_v1(resulting_format_state),
        pending_explicit_properties=state.pending_explicit_properties,
    )


def snapshot_typing_format_v1(
    state: TextTypingFormatStateV1 | None,
) -> TypingFormatSnapshotV1 | None:
    """Immutable snapshot consumed by insertion/IME commit.

    Empty pending state means inherit at commit and therefore produces None.
    """
    if state is None or not state.pending_explicit_properties:
        return None
    return TypingFormatSnapshotV1(
        tuple((prop, value) for prop, value in state.pending_explicit_properties)
    )


def displayed_character_properties_v1(
    *,
    state: TextTypingFormatStateV1 | None,
    selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    format_state: TextFormatOverlayStateV1,
) -> dict[str, Any]:
    """Canonical effective caret formatting overlaid by pending explicit intentions."""
    if not selection.is_collapsed:
        _fail("noncollapsed_selection", "displayed typing format requires collapsed caret")
    _validate_context(selection=selection, domain=domain, format_state=format_state)
    if state is not None:
        validate_typing_format_state_v1(
            state=state,
            selection=selection,
            domain=domain,
            format_state=format_state,
        )

    effective: dict[str, Any] = {}
    if format_state.story_scalar_len > 0:
        caret = selection.focus_scalar
        scalar = caret - 1 if caret > 0 else 0
        if scalar >= format_state.story_scalar_len:
            scalar = format_state.story_scalar_len - 1
        for prop in sorted(_ALLOWED_PROPERTIES):
            segments = effective_property_segments_v1(
                state=format_state,
                prop=prop,
                start_scalar=scalar,
                end_scalar=scalar + 1,
            )
            if len(segments) != 1:
                _fail("typing_context_mismatch", "effective caret formatting is not scalar-deterministic")
            effective[prop] = segments[0].value

    if state is not None:
        for prop, value in state.pending_explicit_properties:
            effective[prop] = value
    return effective
