#!/usr/bin/env python3
"""Deterministic paragraph identity/property lifecycle V1.

Canonical paragraph boundaries are U+000D scalars admitted by Chaptera. A
source-proven terminal structural U+000D may be carried as protected terminal
state; it is not counted as an authoring paragraph split and cannot be targeted.

For each normalized Story edit:
1. remove base/current paragraph boundaries covered by the replaced range;
   upstream ParagraphId/properties survive and removed downstream paragraphs are
   retained in inverse state;
2. insert replacement U+000D boundaries in order; every new downstream
   ParagraphId is preallocated in the operation payload and replay never
   generates an ID;
3. without an explicit preset, each inserted downstream paragraph inherits the
   effective properties of the current upstream paragraph.

Multi-edit scripts are normalized by base (start,end), reject overlap/duplicate
ranges, and bind inserted IDs to (normalized edit ordinal, boundary ordinal).
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import uuid
from typing import Any, Literal

from story_range_v1 import validate_scalar_sequence_v1


JsonScalarV1 = str | int | bool | None
ParagraphProvenanceV1 = Literal["source_or_existing", "chaptera_created"]


class ParagraphLifecycleError(ValueError):
    pass


@dataclass(frozen=True)
class ParagraphPropertiesV1:
    items: tuple[tuple[str, JsonScalarV1], ...] = ()


@dataclass(frozen=True)
class ParagraphV1:
    paragraph_id: str
    properties: ParagraphPropertiesV1
    provenance: ParagraphProvenanceV1 = "source_or_existing"


@dataclass(frozen=True)
class StoryParagraphStateV1:
    protocol_version: Literal["chaptera.story-paragraph-state.v1"]
    story_id: str
    story_text: str
    paragraphs: tuple[ParagraphV1, ...]
    protected_terminal_cr: bool = False


@dataclass(frozen=True)
class ParagraphEditV1:
    base_start_scalar: int
    base_end_scalar: int
    expected_before: str
    replacement_text: str
    inserted_paragraph_ids: tuple[str, ...] = ()
    inserted_property_presets: tuple[ParagraphPropertiesV1 | None, ...] = ()


@dataclass(frozen=True)
class RemovedParagraphV1:
    normalized_edit_ordinal: int
    boundary_scalar_at_removal: int
    paragraph: ParagraphV1


@dataclass(frozen=True)
class ParagraphLifecycleReceiptV1:
    protocol_version: Literal["chaptera.paragraph-lifecycle-receipt.v1"]
    before_state: StoryParagraphStateV1
    after_state: StoryParagraphStateV1
    normalized_edits: tuple[ParagraphEditV1, ...]
    removed_paragraphs: tuple[RemovedParagraphV1, ...]
    before_state_hash: str
    after_state_hash: str


def _fail(message: str) -> None:
    raise ParagraphLifecycleError(message)


def _canonical_properties(
    value: ParagraphPropertiesV1,
) -> ParagraphPropertiesV1:
    if not isinstance(value, ParagraphPropertiesV1):
        _fail("paragraph properties must be ParagraphPropertiesV1")
    seen = set()
    normalized = []
    for key, item in value.items:
        if not isinstance(key, str) or not key:
            _fail("paragraph property key must be non-empty string")
        if key in seen:
            _fail("paragraph property keys must be unique")
        seen.add(key)
        if not (
            isinstance(item, (str, int, bool))
            or item is None
        ):
            _fail("paragraph property values must be JSON scalar values")
        if isinstance(item, float):
            _fail("paragraph property float values are not admitted in V1")
        normalized.append((key, item))
    normalized.sort(key=lambda pair: pair[0])
    return ParagraphPropertiesV1(tuple(normalized))


def _is_uuidv7(value: str) -> bool:
    try:
        parsed = uuid.UUID(value)
    except (ValueError, AttributeError, TypeError):
        return False
    return parsed.version == 7 and parsed.variant == uuid.RFC_4122


def _validate_paragraph_id(value: str, *, require_v7: bool) -> str:
    if not isinstance(value, str) or not value:
        _fail("paragraph_id must be a non-empty string")
    if require_v7 and not _is_uuidv7(value):
        _fail("new ParagraphId must be a preallocated UUIDv7")
    return value


def _effective_boundary_positions(
    story_text: str,
    *,
    protected_terminal_cr: bool,
) -> tuple[int, ...]:
    positions = [i for i, ch in enumerate(story_text) if ch == "\r"]
    if protected_terminal_cr:
        if not story_text.endswith("\r"):
            _fail("protected terminal CR state requires final U+000D")
        if not positions or positions[-1] != len(story_text) - 1:
            _fail("protected terminal CR state is inconsistent")
        positions = positions[:-1]
    return tuple(positions)


def build_story_paragraph_state_v1(
    *,
    story_id: str,
    story_text: str,
    paragraphs: tuple[ParagraphV1, ...],
    protected_terminal_cr: bool = False,
) -> StoryParagraphStateV1:
    if not isinstance(story_id, str) or not story_id:
        _fail("story_id is required")
    validate_scalar_sequence_v1(story_text, "story_text")
    if "\n" in story_text:
        _fail("canonical Story text must not contain LF")
    if not isinstance(protected_terminal_cr, bool):
        _fail("protected_terminal_cr must be boolean")
    boundaries = _effective_boundary_positions(
        story_text,
        protected_terminal_cr=protected_terminal_cr,
    )
    if not isinstance(paragraphs, tuple):
        _fail("paragraphs must be an ordered tuple")
    if len(paragraphs) != len(boundaries) + 1:
        _fail("Paragraph entity count must equal admitted boundaries + one")

    ids = set()
    canonical = []
    for index, paragraph in enumerate(paragraphs):
        if not isinstance(paragraph, ParagraphV1):
            _fail(f"paragraphs[{index}] must be ParagraphV1")
        pid = _validate_paragraph_id(paragraph.paragraph_id, require_v7=False)
        if pid in ids:
            _fail("ParagraphIds must be unique within one Story")
        ids.add(pid)
        if paragraph.provenance not in {"source_or_existing", "chaptera_created"}:
            _fail("paragraph provenance is invalid")
        canonical.append(
            ParagraphV1(
                paragraph_id=pid,
                properties=_canonical_properties(paragraph.properties),
                provenance=paragraph.provenance,
            )
        )
    return StoryParagraphStateV1(
        protocol_version="chaptera.story-paragraph-state.v1",
        story_id=story_id,
        story_text=story_text,
        paragraphs=tuple(canonical),
        protected_terminal_cr=protected_terminal_cr,
    )


def _paragraph_to_dict(value: ParagraphV1) -> dict:
    return {
        "paragraph_id": value.paragraph_id,
        "properties": list(value.properties.items),
        "provenance": value.provenance,
    }


def state_dict_v1(state: StoryParagraphStateV1) -> dict:
    return {
        "protocol_version": state.protocol_version,
        "story_id": state.story_id,
        "story_text": state.story_text,
        "protected_terminal_cr": state.protected_terminal_cr,
        "paragraphs": [_paragraph_to_dict(p) for p in state.paragraphs],
    }


def state_hash_v1(state: StoryParagraphStateV1) -> str:
    return hashlib.sha256(
        json.dumps(
            state_dict_v1(state),
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
    ).hexdigest()


def _validate_edit_against_base(
    *,
    edit: ParagraphEditV1,
    base_text: str,
    protected_terminal_cr: bool,
) -> ParagraphEditV1:
    if not isinstance(edit, ParagraphEditV1):
        _fail("edits must contain ParagraphEditV1 values")
    start = edit.base_start_scalar
    end = edit.base_end_scalar
    if (
        not isinstance(start, int)
        or isinstance(start, bool)
        or not isinstance(end, int)
        or isinstance(end, bool)
        or start < 0
        or end < start
        or end > len(base_text)
    ):
        _fail("base edit range is invalid")

    if protected_terminal_cr:
        protected_start = len(base_text) - 1
        if start > protected_start or end > protected_start:
            _fail("edit targets protected terminal Story structure")

    validate_scalar_sequence_v1(edit.expected_before, "expected_before")
    validate_scalar_sequence_v1(edit.replacement_text, "replacement_text")
    if "\n" in edit.expected_before or "\n" in edit.replacement_text:
        _fail("paragraph lifecycle consumes canonical U+000D text only")
    if base_text[start:end] != edit.expected_before:
        _fail("expected_before does not match base Story range")

    boundary_count = edit.replacement_text.count("\r")
    if not isinstance(edit.inserted_paragraph_ids, tuple):
        _fail("inserted_paragraph_ids must be a tuple")
    if len(edit.inserted_paragraph_ids) != boundary_count:
        _fail("one preallocated ParagraphId is required per inserted boundary")
    for value in edit.inserted_paragraph_ids:
        _validate_paragraph_id(value, require_v7=True)
    if len(set(edit.inserted_paragraph_ids)) != len(edit.inserted_paragraph_ids):
        _fail("inserted ParagraphIds must be unique within edit")

    presets = edit.inserted_property_presets
    if not presets:
        presets = tuple(None for _ in range(boundary_count))
    if not isinstance(presets, tuple) or len(presets) != boundary_count:
        _fail("inserted_property_presets must match inserted boundaries")
    canonical_presets = tuple(
        None if preset is None else _canonical_properties(preset)
        for preset in presets
    )
    return ParagraphEditV1(
        base_start_scalar=start,
        base_end_scalar=end,
        expected_before=edit.expected_before,
        replacement_text=edit.replacement_text,
        inserted_paragraph_ids=edit.inserted_paragraph_ids,
        inserted_property_presets=canonical_presets,
    )


def _normalize_edits(
    *,
    base_state: StoryParagraphStateV1,
    edits: tuple[ParagraphEditV1, ...],
) -> tuple[ParagraphEditV1, ...]:
    if not isinstance(edits, tuple) or not edits:
        _fail("one or more paragraph Story edits are required")
    canonical = [
        _validate_edit_against_base(
            edit=edit,
            base_text=base_state.story_text,
            protected_terminal_cr=base_state.protected_terminal_cr,
        )
        for edit in edits
    ]
    canonical.sort(key=lambda edit: (edit.base_start_scalar, edit.base_end_scalar))

    for previous, current in zip(canonical, canonical[1:]):
        if (
            previous.base_start_scalar == current.base_start_scalar
            and previous.base_end_scalar == current.base_end_scalar
        ):
            _fail("duplicate base edit range is ambiguous")
        if previous.base_end_scalar > current.base_start_scalar:
            _fail("paragraph edit script ranges must not overlap")

    all_new_ids = [
        pid
        for edit in canonical
        for pid in edit.inserted_paragraph_ids
    ]
    base_ids = {p.paragraph_id for p in base_state.paragraphs}
    if len(set(all_new_ids)) != len(all_new_ids):
        _fail("preallocated ParagraphIds must be unique across edit script")
    if any(pid in base_ids for pid in all_new_ids):
        _fail("preallocated ParagraphId collides with existing paragraph")
    return tuple(canonical)


def _paragraph_index_before_scalar(
    story_text: str,
    scalar: int,
    *,
    protected_terminal_cr: bool,
) -> int:
    boundaries = _effective_boundary_positions(
        story_text,
        protected_terminal_cr=protected_terminal_cr,
    )
    return sum(1 for position in boundaries if position < scalar)


def apply_paragraph_lifecycle_v1(
    *,
    base_state: StoryParagraphStateV1,
    edits: tuple[ParagraphEditV1, ...],
    expected_base_state_hash: str,
) -> ParagraphLifecycleReceiptV1:
    base_state = build_story_paragraph_state_v1(
        story_id=base_state.story_id,
        story_text=base_state.story_text,
        paragraphs=base_state.paragraphs,
        protected_terminal_cr=base_state.protected_terminal_cr,
    )
    if expected_base_state_hash != state_hash_v1(base_state):
        _fail("stale paragraph lifecycle base state")

    normalized = _normalize_edits(base_state=base_state, edits=edits)
    current_text = base_state.story_text
    current_paragraphs = list(base_state.paragraphs)
    removed: list[RemovedParagraphV1] = []
    cumulative_delta = 0

    for edit_ordinal, edit in enumerate(normalized):
        start = edit.base_start_scalar + cumulative_delta
        end = edit.base_end_scalar + cumulative_delta
        if current_text[start:end] != edit.expected_before:
            _fail("normalized edit no longer matches deterministic current Story")

        if base_state.protected_terminal_cr:
            protected_index = len(current_text) - 1
            if start > protected_index or end > protected_index:
                _fail("normalized edit reached protected terminal Story structure")

        boundary_positions = [
            position
            for position in _effective_boundary_positions(
                current_text,
                protected_terminal_cr=base_state.protected_terminal_cr,
            )
            if start <= position < end
        ]
        boundary_paragraph_indices = [
            _paragraph_index_before_scalar(
                current_text,
                position,
                protected_terminal_cr=base_state.protected_terminal_cr,
            )
            for position in boundary_positions
        ]

        # Each covered boundary removes the downstream paragraph. Delete from
        # right to left so indices remain stable; record deterministic ascending
        # boundary order in inverse state afterwards.
        edit_removed = []
        for position, upstream_index in reversed(
            list(zip(boundary_positions, boundary_paragraph_indices))
        ):
            downstream_index = upstream_index + 1
            paragraph = current_paragraphs.pop(downstream_index)
            edit_removed.append(
                RemovedParagraphV1(
                    normalized_edit_ordinal=edit_ordinal,
                    boundary_scalar_at_removal=position,
                    paragraph=paragraph,
                )
            )
        removed.extend(reversed(edit_removed))

        text_without_range = current_text[:start] + current_text[end:]
        upstream_index = _paragraph_index_before_scalar(
            text_without_range,
            start,
            protected_terminal_cr=base_state.protected_terminal_cr,
        )

        for boundary_ordinal, paragraph_id in enumerate(
            edit.inserted_paragraph_ids
        ):
            preset = edit.inserted_property_presets[boundary_ordinal]
            upstream = current_paragraphs[upstream_index + boundary_ordinal]
            properties = upstream.properties if preset is None else preset
            current_paragraphs.insert(
                upstream_index + boundary_ordinal + 1,
                ParagraphV1(
                    paragraph_id=paragraph_id,
                    properties=properties,
                    provenance="chaptera_created",
                ),
            )

        current_text = (
            current_text[:start]
            + edit.replacement_text
            + current_text[end:]
        )
        cumulative_delta += len(edit.replacement_text) - (
            edit.base_end_scalar - edit.base_start_scalar
        )

        # Fail immediately if one edit made paragraph topology inconsistent.
        build_story_paragraph_state_v1(
            story_id=base_state.story_id,
            story_text=current_text,
            paragraphs=tuple(current_paragraphs),
            protected_terminal_cr=base_state.protected_terminal_cr,
        )

    after = build_story_paragraph_state_v1(
        story_id=base_state.story_id,
        story_text=current_text,
        paragraphs=tuple(current_paragraphs),
        protected_terminal_cr=base_state.protected_terminal_cr,
    )
    return ParagraphLifecycleReceiptV1(
        protocol_version="chaptera.paragraph-lifecycle-receipt.v1",
        before_state=base_state,
        after_state=after,
        normalized_edits=normalized,
        removed_paragraphs=tuple(removed),
        before_state_hash=state_hash_v1(base_state),
        after_state_hash=state_hash_v1(after),
    )


def undo_paragraph_lifecycle_v1(
    receipt: ParagraphLifecycleReceiptV1,
) -> StoryParagraphStateV1:
    if not isinstance(receipt, ParagraphLifecycleReceiptV1):
        _fail("ParagraphLifecycleReceiptV1 is required")
    return receipt.before_state


def replay_paragraph_lifecycle_v1(
    receipt: ParagraphLifecycleReceiptV1,
) -> StoryParagraphStateV1:
    if not isinstance(receipt, ParagraphLifecycleReceiptV1):
        _fail("ParagraphLifecycleReceiptV1 is required")
    replay = apply_paragraph_lifecycle_v1(
        base_state=receipt.before_state,
        edits=receipt.normalized_edits,
        expected_base_state_hash=receipt.before_state_hash,
    )
    if replay.after_state != receipt.after_state:
        _fail("paragraph lifecycle replay regenerated different state")
    if replay.removed_paragraphs != receipt.removed_paragraphs:
        _fail("paragraph lifecycle replay regenerated different inverse state")
    return replay.after_state
