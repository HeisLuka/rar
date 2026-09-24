#!/usr/bin/env python3
"""Deterministic formatting assignment for newly inserted Story scalars V1.

ReplaceStoryRange decides which canonical scalars exist. This module decides the
character formatting of replacement scalars and returns a normalized
TextFormatOverlayStateV1 for the post-edit Story.

Boundary law is explicitly left-biased:
- insertion strictly inside an effective format inherits it;
- at a boundary between two formats, inserted text inherits the left side;
- at Story start (no left side), inherit the old first scalar;
- at Story end, inherit the old last scalar;
- for an empty author-created Story, caller must supply the explicit
  AuthoringTextPreset-derived base format.

Existing Chaptera overrides are rebased through the shared range transform with
start=right/end=right and full-cover=delete. This makes a left span include an
insertion at its end while a right span excludes an insertion at its start.
Replacement formatting itself is then assigned explicitly from inherited
effective formatting, a transient typing snapshot, or an admitted semantic
fragment payload.

No zero-length durable formatting spans, browser/DOM authority, paragraph
ownership, native Quill/BTE/STSH/FONT write, OT or CRDT lives here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Literal

from range_anchor_rebase_v1 import (
    AnchoredRangeV1,
    RangeAnchorPolicyV1,
    StoryRangeEditV1,
    rebase_anchored_range_v1,
)
from story_range_v1 import validate_scalar_sequence_v1
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    FormatPropertyV1,
    TextFormatOverlayError,
    TextFormatOverlayStateV1,
    TextFormatOverrideRunV1,
    build_text_format_overlay_state_v1,
    effective_property_segments_v1,
    set_text_format_property_v1,
    state_hash_v1,
)


class TextInsertFormatError(ValueError):
    pass


@dataclass(frozen=True)
class TypingFormatSnapshotV1:
    items: tuple[tuple[FormatPropertyV1, Any], ...]


@dataclass(frozen=True)
class RelativeFragmentFormatRunV1:
    start_offset: int
    end_offset: int
    property: FormatPropertyV1
    value: Any


@dataclass(frozen=True)
class TextInsertFormatReceiptV1:
    protocol_version: Literal["chaptera.text-insert-format-receipt.v1"]
    before_state: TextFormatOverlayStateV1
    after_state: TextFormatOverlayStateV1
    edit_start_scalar: int
    edit_end_scalar: int
    replacement_text: str
    inserted_start_scalar: int
    inserted_end_scalar: int
    source: Literal[
        "inherited",
        "typing_snapshot",
        "semantic_fragment",
        "deletion_only",
    ]
    inherited_base_format: BaseCharacterFormatV1 | None
    typing_snapshot: TypingFormatSnapshotV1 | None
    fragment_runs: tuple[RelativeFragmentFormatRunV1, ...]
    empty_story_preset_format: BaseCharacterFormatV1 | None
    post_edit_revision_id: str


_FORMAT_POLICY = RangeAnchorPolicyV1(
    start_affinity="right",
    end_affinity="right",
    full_cover_policy="delete",
)


def _fail(message: str) -> None:
    raise TextInsertFormatError(message)


def _base_format_at(
    state: TextFormatOverlayStateV1,
    scalar: int,
) -> BaseCharacterFormatV1:
    for run in state.base_runs:
        if run.start_scalar <= scalar < run.end_scalar:
            return run.format
    _fail("base formatting does not cover inheritance scalar")


def _effective_value_at(
    state: TextFormatOverlayStateV1,
    prop: FormatPropertyV1,
    scalar: int,
) -> Any:
    segments = effective_property_segments_v1(
        state=state,
        prop=prop,
        start_scalar=scalar,
        end_scalar=scalar + 1,
    )
    if len(segments) != 1:
        _fail("effective formatting resolution is not scalar-deterministic")
    return segments[0].value


def _effective_format_at(
    state: TextFormatOverlayStateV1,
    scalar: int,
) -> BaseCharacterFormatV1:
    base = _base_format_at(state, scalar)
    return BaseCharacterFormatV1(
        font_resource_id=base.font_resource_id,
        font_size_emu=_effective_value_at(state, "font_size_emu", scalar),
        bold=_effective_value_at(state, "bold", scalar),
        italic=_effective_value_at(state, "italic", scalar),
        text_color_rgb=_effective_value_at(state, "text_color_rgb", scalar),
    )


def _inheritance_format(
    *,
    before_state: TextFormatOverlayStateV1,
    start: int,
    end: int,
    replacement_length: int,
    empty_story_preset_format: BaseCharacterFormatV1 | None,
) -> BaseCharacterFormatV1 | None:
    if replacement_length == 0:
        return None
    if before_state.story_scalar_len == 0:
        if empty_story_preset_format is None:
            _fail(
                "empty author-created Story insertion requires explicit "
                "AuthoringTextPreset-derived base format"
            )
        # Let overlay-state construction validate the preset.
        return empty_story_preset_format

    if start > 0:
        scalar = start - 1
    elif start < before_state.story_scalar_len:
        # Story-start insertion/replacement fallback: old first scalar.
        scalar = start
    elif end > 0:
        scalar = end - 1
    else:
        scalar = before_state.story_scalar_len - 1
    return _effective_format_at(before_state, scalar)


def _append_base_piece(
    out: list[BaseFormatRunV1],
    start: int,
    end: int,
    fmt: BaseCharacterFormatV1,
) -> None:
    if end <= start:
        return
    if out and out[-1].end_scalar == start and out[-1].format == fmt:
        previous = out[-1]
        out[-1] = BaseFormatRunV1(previous.start_scalar, end, fmt)
    else:
        out.append(BaseFormatRunV1(start, end, fmt))


def _rebase_base_runs(
    *,
    state: TextFormatOverlayStateV1,
    start: int,
    end: int,
    replacement_length: int,
    inherited: BaseCharacterFormatV1 | None,
) -> tuple[BaseFormatRunV1, ...]:
    delta = replacement_length - (end - start)
    pieces: list[BaseFormatRunV1] = []

    for run in state.base_runs:
        left_start = run.start_scalar
        left_end = min(run.end_scalar, start)
        if left_end > left_start:
            _append_base_piece(pieces, left_start, left_end, run.format)

    if replacement_length:
        if inherited is None:
            _fail("inserted scalars require an inherited/preset base format")
        _append_base_piece(
            pieces,
            start,
            start + replacement_length,
            inherited,
        )

    for run in state.base_runs:
        old_start = max(run.start_scalar, end)
        old_end = run.end_scalar
        if old_end <= old_start:
            continue
        _append_base_piece(
            pieces,
            old_start + delta,
            old_end + delta,
            run.format,
        )
    return tuple(pieces)


def _rebase_overrides(
    *,
    state: TextFormatOverlayStateV1,
    start: int,
    end: int,
    replacement_length: int,
) -> tuple[TextFormatOverrideRunV1, ...]:
    edit = StoryRangeEditV1(start, end, replacement_length)
    out = []
    for run in state.overrides:
        receipt = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(run.start_scalar, run.end_scalar),
            policy=_FORMAT_POLICY,
            edit=edit,
        )
        if receipt.result.status != "survives":
            continue
        rebased = receipt.result.range
        if rebased is None or rebased.start_scalar == rebased.end_scalar:
            continue
        out.append(
            TextFormatOverrideRunV1(
                rebased.start_scalar,
                rebased.end_scalar,
                run.property,
                run.value,
            )
        )
    return tuple(out)


def _canonical_typing_snapshot(
    snapshot: TypingFormatSnapshotV1 | None,
) -> TypingFormatSnapshotV1 | None:
    if snapshot is None:
        return None
    if not isinstance(snapshot, TypingFormatSnapshotV1):
        _fail("typing_snapshot must be TypingFormatSnapshotV1")
    seen = set()
    items = []
    for prop, value in snapshot.items:
        if prop in seen:
            _fail("typing snapshot properties must be unique")
        seen.add(prop)
        items.append((prop, value))
    items.sort(key=lambda pair: pair[0])
    return TypingFormatSnapshotV1(tuple(items))


def _canonical_fragment_runs(
    runs: tuple[RelativeFragmentFormatRunV1, ...],
    replacement_length: int,
) -> tuple[RelativeFragmentFormatRunV1, ...]:
    if not isinstance(runs, tuple):
        _fail("fragment_runs must be an ordered tuple")
    out = []
    for index, run in enumerate(runs):
        if not isinstance(run, RelativeFragmentFormatRunV1):
            _fail(f"fragment_runs[{index}] must be RelativeFragmentFormatRunV1")
        if (
            not isinstance(run.start_offset, int)
            or isinstance(run.start_offset, bool)
            or not isinstance(run.end_offset, int)
            or isinstance(run.end_offset, bool)
            or run.start_offset < 0
            or run.end_offset <= run.start_offset
            or run.end_offset > replacement_length
        ):
            _fail("fragment formatting range must be non-empty and inside replacement")
        out.append(run)
    out.sort(key=lambda run: (run.property, run.start_offset, run.end_offset))
    for previous, current in zip(out, out[1:]):
        if (
            previous.property == current.property
            and previous.end_offset > current.start_offset
        ):
            _fail("semantic fragment format runs for one property must not overlap")
    return tuple(out)


def _apply_explicit_insert_format(
    *,
    state: TextFormatOverlayStateV1,
    inserted_start: int,
    inserted_end: int,
    typing_snapshot: TypingFormatSnapshotV1 | None,
    fragment_runs: tuple[RelativeFragmentFormatRunV1, ...],
) -> TextFormatOverlayStateV1:
    if inserted_end == inserted_start:
        return state

    current = state
    if typing_snapshot is not None:
        for prop, value in typing_snapshot.items:
            current = set_text_format_property_v1(
                state=current,
                start_scalar=inserted_start,
                end_scalar=inserted_end,
                prop=prop,
                value=value,
                expected_state_hash=state_hash_v1(current),
            ).after_state

    for run in fragment_runs:
        current = set_text_format_property_v1(
            state=current,
            start_scalar=inserted_start + run.start_offset,
            end_scalar=inserted_start + run.end_offset,
            prop=run.property,
            value=run.value,
            expected_state_hash=state_hash_v1(current),
        ).after_state
    return current


def plan_text_insert_format_v1(
    *,
    before_state: TextFormatOverlayStateV1,
    edit_start_scalar: int,
    edit_end_scalar: int,
    replacement_text: str,
    post_edit_revision_id: str,
    typing_snapshot: TypingFormatSnapshotV1 | None = None,
    fragment_runs: tuple[RelativeFragmentFormatRunV1, ...] = (),
    empty_story_preset_format: BaseCharacterFormatV1 | None = None,
) -> TextInsertFormatReceiptV1:
    if not isinstance(before_state, TextFormatOverlayStateV1):
        _fail("before_state must be TextFormatOverlayStateV1")
    if (
        not isinstance(edit_start_scalar, int)
        or isinstance(edit_start_scalar, bool)
        or not isinstance(edit_end_scalar, int)
        or isinstance(edit_end_scalar, bool)
        or edit_start_scalar < 0
        or edit_end_scalar < edit_start_scalar
        or edit_end_scalar > before_state.story_scalar_len
    ):
        _fail("Story replacement range is invalid")
    validate_scalar_sequence_v1(replacement_text, "replacement_text")
    if "\n" in replacement_text:
        _fail("replacement_text must already use canonical U+000D paragraph boundaries")
    if not isinstance(post_edit_revision_id, str) or not post_edit_revision_id:
        _fail("post_edit_revision_id is required")

    snapshot = _canonical_typing_snapshot(typing_snapshot)
    replacement_length = len(replacement_text)
    fragments = _canonical_fragment_runs(fragment_runs, replacement_length)
    if snapshot is not None and fragments:
        _fail("typing snapshot and semantic fragment formatting are mutually exclusive")

    inherited = _inheritance_format(
        before_state=before_state,
        start=edit_start_scalar,
        end=edit_end_scalar,
        replacement_length=replacement_length,
        empty_story_preset_format=empty_story_preset_format,
    )
    new_len = (
        before_state.story_scalar_len
        - (edit_end_scalar - edit_start_scalar)
        + replacement_length
    )
    base_runs = _rebase_base_runs(
        state=before_state,
        start=edit_start_scalar,
        end=edit_end_scalar,
        replacement_length=replacement_length,
        inherited=inherited,
    )
    rebased_overrides = _rebase_overrides(
        state=before_state,
        start=edit_start_scalar,
        end=edit_end_scalar,
        replacement_length=replacement_length,
    )
    try:
        provisional = build_text_format_overlay_state_v1(
            story_id=before_state.story_id,
            base_revision_id=post_edit_revision_id,
            story_scalar_len=new_len,
            base_runs=base_runs,
            overrides=rebased_overrides,
        )
        after = _apply_explicit_insert_format(
            state=provisional,
            inserted_start=edit_start_scalar,
            inserted_end=edit_start_scalar + replacement_length,
            typing_snapshot=snapshot,
            fragment_runs=fragments,
        )
    except TextFormatOverlayError as exc:
        raise TextInsertFormatError(str(exc)) from exc

    if replacement_length == 0:
        source = "deletion_only"
    elif fragments:
        source = "semantic_fragment"
    elif snapshot is not None:
        source = "typing_snapshot"
    else:
        source = "inherited"

    return TextInsertFormatReceiptV1(
        protocol_version="chaptera.text-insert-format-receipt.v1",
        before_state=before_state,
        after_state=after,
        edit_start_scalar=edit_start_scalar,
        edit_end_scalar=edit_end_scalar,
        replacement_text=replacement_text,
        inserted_start_scalar=edit_start_scalar,
        inserted_end_scalar=edit_start_scalar + replacement_length,
        source=source,
        inherited_base_format=inherited,
        typing_snapshot=snapshot,
        fragment_runs=fragments,
        empty_story_preset_format=empty_story_preset_format,
        post_edit_revision_id=post_edit_revision_id,
    )


def undo_text_insert_format_v1(
    receipt: TextInsertFormatReceiptV1,
) -> TextFormatOverlayStateV1:
    if not isinstance(receipt, TextInsertFormatReceiptV1):
        _fail("TextInsertFormatReceiptV1 is required")
    return receipt.before_state


def replay_text_insert_format_v1(
    receipt: TextInsertFormatReceiptV1,
) -> TextFormatOverlayStateV1:
    if not isinstance(receipt, TextInsertFormatReceiptV1):
        _fail("TextInsertFormatReceiptV1 is required")
    replay = plan_text_insert_format_v1(
        before_state=receipt.before_state,
        edit_start_scalar=receipt.edit_start_scalar,
        edit_end_scalar=receipt.edit_end_scalar,
        replacement_text=receipt.replacement_text,
        post_edit_revision_id=receipt.post_edit_revision_id,
        typing_snapshot=receipt.typing_snapshot,
        fragment_runs=receipt.fragment_runs,
        empty_story_preset_format=receipt.empty_story_preset_format,
    )
    if replay.after_state != receipt.after_state:
        _fail("insert-format replay did not reproduce canonical state")
    return replay.after_state
