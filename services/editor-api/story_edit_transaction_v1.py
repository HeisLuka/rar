#!/usr/bin/env python3
"""Atomic core Story edit transaction V1.

This module composes already-independent Story authoring laws into one
source-neutral candidate state. It does not own revision persistence; the
existing RevisionKernel is the durable single-revision envelope.

Core V1 participant order:
1. authoritative capability inventory / unsupported anchored-semantics gate;
2. StoryEditDomainV1 protection + exact ReplaceStoryRange precondition;
3. ParagraphLifecycleV1;
4. generic document-owned anchored-range rebasing;
5. TextInsertFormatV1 + canonical TextFormatOverlay normalization;
6. whole candidate-state validation.

Only after all steps succeed may RevisionKernel advance the revision pointer.
Derived layout is downstream and is reported as layout_unknown in this bounded
semantic gate. Hyperlink and feature-specific paragraph lifecycle extensions are
fail-closed until separately admitted.
"""

from __future__ import annotations

import copy
from dataclasses import dataclass
import hashlib
import json
from typing import Any, Literal

from paragraph_lifecycle_v1 import (
    ParagraphEditV1,
    ParagraphLifecycleError,
    ParagraphPropertiesV1,
    ParagraphV1,
    StoryParagraphStateV1,
    apply_paragraph_lifecycle_v1,
    build_story_paragraph_state_v1,
    state_dict_v1 as paragraph_state_dict_v1,
    state_hash_v1 as paragraph_state_hash_v1,
)
from range_anchor_rebase_v1 import (
    AnchoredRangeV1,
    RangeAnchorPolicyV1,
    RangeAnchorRebaseError,
    StoryRangeEditV1,
    rebase_anchored_range_v1,
)
from story_edit_domain_v1 import (
    StoryEditDomainError,
    StoryProvenanceV1,
    replace_story_range_in_domain_v1,
)
from story_range_v1 import (
    story_text_hash_v1,
    validate_scalar_sequence_v1,
    validate_story_range_operation_v1,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    TextFormatOverlayStateV1,
    TextFormatOverrideRunV1,
    build_text_format_overlay_state_v1,
    state_dict_v1 as format_state_dict_v1,
    state_hash_v1 as format_state_hash_v1,
)
from text_insert_format_v1 import (
    RelativeFragmentFormatRunV1,
    TextInsertFormatError,
    TypingFormatSnapshotV1,
    plan_text_insert_format_v1,
)


class StoryEditTransactionError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class GenericAnchoredSemanticV1:
    semantic_id: str
    semantic_kind: str
    start_scalar: int
    end_scalar: int
    allow_empty: bool
    start_affinity: Literal["left", "right"]
    end_affinity: Literal["left", "right"]
    full_cover_policy: Literal[
        "replacement",
        "collapse_left",
        "collapse_right",
        "invalidate",
        "delete",
    ]


@dataclass(frozen=True)
class StoryEditCoreStateV1:
    protocol_version: Literal["chaptera.story-edit-core-state.v1"]
    story_id: str
    provenance: StoryProvenanceV1
    paragraph_state: StoryParagraphStateV1
    format_state: TextFormatOverlayStateV1
    generic_anchors: tuple[GenericAnchoredSemanticV1, ...]
    unsupported_anchored_semantics: tuple[str, ...]
    active_paragraph_features: tuple[str, ...]
    empty_story_preset_format: BaseCharacterFormatV1 | None


def _reject(code: str, message: str) -> None:
    raise StoryEditTransactionError(code, message)


def _json_scalar(value: Any) -> bool:
    return (
        value is None
        or isinstance(value, (str, bool))
        or (isinstance(value, int) and not isinstance(value, bool))
    )


def _canonical_string_tuple(
    values: tuple[str, ...] | list[str],
    *,
    label: str,
) -> tuple[str, ...]:
    if not isinstance(values, (tuple, list)):
        _reject("invalid_story_state", f"{label} must be a string sequence")
    out = []
    for value in values:
        if not isinstance(value, str) or not value:
            _reject("invalid_story_state", f"{label} entries must be non-empty strings")
        out.append(value)
    if len(set(out)) != len(out):
        _reject("invalid_story_state", f"{label} entries must be unique")
    return tuple(sorted(out))


def _base_format_to_dict(value: BaseCharacterFormatV1) -> dict:
    return {
        "font_resource_id": value.font_resource_id,
        "font_size_emu": value.font_size_emu,
        "bold": value.bold,
        "italic": value.italic,
        "text_color_rgb": value.text_color_rgb,
    }


def _base_format_from_dict(value: dict | None) -> BaseCharacterFormatV1 | None:
    if value is None:
        return None
    if not isinstance(value, dict) or set(value) != {
        "font_resource_id",
        "font_size_emu",
        "bold",
        "italic",
        "text_color_rgb",
    }:
        _reject("invalid_story_state", "base character format shape is invalid")
    return BaseCharacterFormatV1(
        font_resource_id=value["font_resource_id"],
        font_size_emu=value["font_size_emu"],
        bold=value["bold"],
        italic=value["italic"],
        text_color_rgb=value["text_color_rgb"],
    )


def _paragraph_state_from_dict(value: dict) -> StoryParagraphStateV1:
    if not isinstance(value, dict):
        _reject("invalid_story_state", "paragraph_state must be an object")
    expected = {
        "protocol_version",
        "story_id",
        "story_text",
        "protected_terminal_cr",
        "paragraphs",
    }
    if set(value) != expected or value.get("protocol_version") != "chaptera.story-paragraph-state.v1":
        _reject("invalid_story_state", "paragraph_state shape/protocol mismatch")
    paragraphs_raw = value.get("paragraphs")
    if not isinstance(paragraphs_raw, list):
        _reject("invalid_story_state", "paragraph_state.paragraphs must be a list")
    paragraphs = []
    for item in paragraphs_raw:
        if not isinstance(item, dict) or set(item) != {
            "paragraph_id",
            "properties",
            "provenance",
        }:
            _reject("invalid_story_state", "paragraph entry shape is invalid")
        raw_props = item.get("properties")
        if not isinstance(raw_props, list):
            _reject("invalid_story_state", "paragraph properties must be a list")
        props = []
        for pair in raw_props:
            if (
                not isinstance(pair, (list, tuple))
                or len(pair) != 2
                or not isinstance(pair[0], str)
                or not pair[0]
                or not _json_scalar(pair[1])
            ):
                _reject("invalid_story_state", "paragraph property entry is invalid")
            props.append((pair[0], pair[1]))
        paragraphs.append(
            ParagraphV1(
                paragraph_id=item["paragraph_id"],
                properties=ParagraphPropertiesV1(tuple(props)),
                provenance=item["provenance"],
            )
        )
    try:
        return build_story_paragraph_state_v1(
            story_id=value["story_id"],
            story_text=value["story_text"],
            paragraphs=tuple(paragraphs),
            protected_terminal_cr=value["protected_terminal_cr"],
        )
    except (ParagraphLifecycleError, ValueError) as exc:
        _reject("invalid_story_state", str(exc))


def _format_state_from_dict(value: dict) -> TextFormatOverlayStateV1:
    if not isinstance(value, dict):
        _reject("invalid_story_state", "format_state must be an object")
    expected = {
        "protocol_version",
        "story_id",
        "base_revision_id",
        "story_scalar_len",
        "base_runs",
        "overrides",
    }
    if set(value) != expected or value.get("protocol_version") != "chaptera.text-format-overlay.v1":
        _reject("invalid_story_state", "format_state shape/protocol mismatch")
    base_runs = []
    for item in value.get("base_runs", []):
        if not isinstance(item, dict) or set(item) != {
            "start_scalar",
            "end_scalar",
            "format",
        }:
            _reject("invalid_story_state", "base format run shape is invalid")
        base_format = _base_format_from_dict(item["format"])
        assert base_format is not None
        base_runs.append(
            BaseFormatRunV1(
                item["start_scalar"],
                item["end_scalar"],
                base_format,
            )
        )
    overrides = []
    for item in value.get("overrides", []):
        if not isinstance(item, dict) or set(item) != {
            "start_scalar",
            "end_scalar",
            "property",
            "value",
        }:
            _reject("invalid_story_state", "format override run shape is invalid")
        overrides.append(
            TextFormatOverrideRunV1(
                item["start_scalar"],
                item["end_scalar"],
                item["property"],
                item["value"],
            )
        )
    try:
        return build_text_format_overlay_state_v1(
            story_id=value["story_id"],
            base_revision_id=value["base_revision_id"],
            story_scalar_len=value["story_scalar_len"],
            base_runs=tuple(base_runs),
            overrides=tuple(overrides),
        )
    except ValueError as exc:
        _reject("invalid_story_state", str(exc))


def _anchor_to_dict(value: GenericAnchoredSemanticV1) -> dict:
    return {
        "semantic_id": value.semantic_id,
        "semantic_kind": value.semantic_kind,
        "start_scalar": value.start_scalar,
        "end_scalar": value.end_scalar,
        "allow_empty": value.allow_empty,
        "start_affinity": value.start_affinity,
        "end_affinity": value.end_affinity,
        "full_cover_policy": value.full_cover_policy,
    }


def _anchor_from_dict(value: dict) -> GenericAnchoredSemanticV1:
    expected = {
        "semantic_id",
        "semantic_kind",
        "start_scalar",
        "end_scalar",
        "allow_empty",
        "start_affinity",
        "end_affinity",
        "full_cover_policy",
    }
    if not isinstance(value, dict) or set(value) != expected:
        _reject("invalid_story_state", "generic anchor shape is invalid")
    anchor = GenericAnchoredSemanticV1(
        semantic_id=value["semantic_id"],
        semantic_kind=value["semantic_kind"],
        start_scalar=value["start_scalar"],
        end_scalar=value["end_scalar"],
        allow_empty=value["allow_empty"],
        start_affinity=value["start_affinity"],
        end_affinity=value["end_affinity"],
        full_cover_policy=value["full_cover_policy"],
    )
    _validate_anchor(anchor, story_len=None)
    return anchor


def _validate_anchor(
    anchor: GenericAnchoredSemanticV1,
    *,
    story_len: int | None,
) -> None:
    if not isinstance(anchor.semantic_id, str) or not anchor.semantic_id:
        _reject("invalid_story_state", "generic anchor semantic_id is required")
    if not isinstance(anchor.semantic_kind, str) or not anchor.semantic_kind:
        _reject("invalid_story_state", "generic anchor semantic_kind is required")
    if not isinstance(anchor.allow_empty, bool):
        _reject("invalid_story_state", "generic anchor allow_empty must be boolean")
    if (
        not isinstance(anchor.start_scalar, int)
        or isinstance(anchor.start_scalar, bool)
        or not isinstance(anchor.end_scalar, int)
        or isinstance(anchor.end_scalar, bool)
        or anchor.start_scalar < 0
        or anchor.end_scalar < anchor.start_scalar
    ):
        _reject("invalid_story_state", "generic anchor scalar range is invalid")
    if story_len is not None and anchor.end_scalar > story_len:
        _reject("invalid_story_state", "generic anchor lies outside Story")
    if anchor.start_scalar == anchor.end_scalar and not anchor.allow_empty:
        _reject("invalid_story_state", "non-empty generic anchor collapsed to zero length")
    if anchor.start_affinity not in {"left", "right"}:
        _reject("invalid_story_state", "generic anchor start affinity is invalid")
    if anchor.end_affinity not in {"left", "right"}:
        _reject("invalid_story_state", "generic anchor end affinity is invalid")
    if anchor.full_cover_policy not in {
        "replacement",
        "collapse_left",
        "collapse_right",
        "invalidate",
        "delete",
    }:
        _reject("invalid_story_state", "generic anchor full-cover policy is invalid")


def build_story_edit_core_state_v1(
    *,
    story_id: str,
    provenance: StoryProvenanceV1,
    paragraph_state: StoryParagraphStateV1,
    format_state: TextFormatOverlayStateV1,
    generic_anchors: tuple[GenericAnchoredSemanticV1, ...] = (),
    unsupported_anchored_semantics: tuple[str, ...] = (),
    active_paragraph_features: tuple[str, ...] = (),
    empty_story_preset_format: BaseCharacterFormatV1 | None = None,
) -> StoryEditCoreStateV1:
    if not isinstance(story_id, str) or not story_id:
        _reject("invalid_story_state", "story_id is required")
    if provenance not in {
        "chaptera_created",
        "imported_mature_quill_terminal_cr",
        "imported_unknown",
    }:
        _reject("invalid_story_state", "Story provenance is unsupported")
    if paragraph_state.story_id != story_id or format_state.story_id != story_id:
        _reject("invalid_story_state", "participant StoryIds disagree")
    story_text = paragraph_state.story_text
    if format_state.story_scalar_len != len(story_text):
        _reject("invalid_story_state", "Story text length and formatting extent disagree")
    expected_protected = provenance == "imported_mature_quill_terminal_cr"
    if paragraph_state.protected_terminal_cr != expected_protected:
        _reject(
            "invalid_story_state",
            "paragraph terminal protection disagrees with Story provenance",
        )

    if not isinstance(generic_anchors, tuple):
        _reject("invalid_story_state", "generic_anchors must be an ordered tuple")
    ids = set()
    canonical_anchors = []
    for anchor in generic_anchors:
        if not isinstance(anchor, GenericAnchoredSemanticV1):
            _reject("invalid_story_state", "generic_anchors contains invalid entry")
        _validate_anchor(anchor, story_len=len(story_text))
        if anchor.semantic_id in ids:
            _reject("invalid_story_state", "generic anchor semantic IDs must be unique")
        ids.add(anchor.semantic_id)
        canonical_anchors.append(anchor)
    canonical_anchors.sort(key=lambda a: (a.semantic_id, a.semantic_kind))

    unsupported = _canonical_string_tuple(
        unsupported_anchored_semantics,
        label="unsupported_anchored_semantics",
    )
    active_features = _canonical_string_tuple(
        active_paragraph_features,
        label="active_paragraph_features",
    )

    if empty_story_preset_format is not None:
        # Overlay constructor is the format validator. Validate the preset in a
        # one-scalar synthetic base without retaining that synthetic state.
        try:
            build_text_format_overlay_state_v1(
                story_id=story_id,
                base_revision_id="preset-validation",
                story_scalar_len=1,
                base_runs=(BaseFormatRunV1(0, 1, empty_story_preset_format),),
            )
        except ValueError as exc:
            _reject("invalid_story_state", str(exc))

    return StoryEditCoreStateV1(
        protocol_version="chaptera.story-edit-core-state.v1",
        story_id=story_id,
        provenance=provenance,
        paragraph_state=paragraph_state,
        format_state=format_state,
        generic_anchors=tuple(canonical_anchors),
        unsupported_anchored_semantics=unsupported,
        active_paragraph_features=active_features,
        empty_story_preset_format=empty_story_preset_format,
    )


def story_edit_core_state_to_dict(state: StoryEditCoreStateV1) -> dict:
    return {
        "protocol_version": state.protocol_version,
        "story_id": state.story_id,
        "provenance": state.provenance,
        "paragraph_state": paragraph_state_dict_v1(state.paragraph_state),
        "format_state": format_state_dict_v1(state.format_state),
        "generic_anchors": [_anchor_to_dict(a) for a in state.generic_anchors],
        "unsupported_anchored_semantics": list(state.unsupported_anchored_semantics),
        "active_paragraph_features": list(state.active_paragraph_features),
        "empty_story_preset_format": (
            None
            if state.empty_story_preset_format is None
            else _base_format_to_dict(state.empty_story_preset_format)
        ),
    }


def story_edit_core_state_from_dict(value: dict) -> StoryEditCoreStateV1:
    expected = {
        "protocol_version",
        "story_id",
        "provenance",
        "paragraph_state",
        "format_state",
        "generic_anchors",
        "unsupported_anchored_semantics",
        "active_paragraph_features",
        "empty_story_preset_format",
    }
    if not isinstance(value, dict) or set(value) != expected:
        _reject("invalid_story_state", "Story edit core-state shape is invalid")
    if value.get("protocol_version") != "chaptera.story-edit-core-state.v1":
        _reject("invalid_story_state", "Story edit core-state protocol mismatch")
    anchors_raw = value.get("generic_anchors")
    if not isinstance(anchors_raw, list):
        _reject("invalid_story_state", "generic_anchors must serialize as a list")
    return build_story_edit_core_state_v1(
        story_id=value["story_id"],
        provenance=value["provenance"],
        paragraph_state=_paragraph_state_from_dict(value["paragraph_state"]),
        format_state=_format_state_from_dict(value["format_state"]),
        generic_anchors=tuple(_anchor_from_dict(item) for item in anchors_raw),
        unsupported_anchored_semantics=tuple(value["unsupported_anchored_semantics"]),
        active_paragraph_features=tuple(value["active_paragraph_features"]),
        empty_story_preset_format=_base_format_from_dict(
            value["empty_story_preset_format"]
        ),
    )


def story_edit_core_state_id_v1(state: StoryEditCoreStateV1) -> str:
    payload = json.dumps(
        story_edit_core_state_to_dict(state),
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def _paragraph_properties_from_command(value: dict | None) -> ParagraphPropertiesV1 | None:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError("paragraph property preset must be object or null")
    items = []
    for key, item in value.items():
        if not isinstance(key, str) or not key or not _json_scalar(item):
            raise ValueError("paragraph property preset contains invalid JSON scalar")
        items.append((key, item))
    items.sort(key=lambda pair: pair[0])
    return ParagraphPropertiesV1(tuple(items))


def _typing_snapshot_from_command(value: dict | None) -> TypingFormatSnapshotV1 | None:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError("typing_format must be object or null")
    return TypingFormatSnapshotV1(tuple(sorted(value.items(), key=lambda pair: pair[0])))


def _fragment_runs_from_command(values: list) -> tuple[RelativeFragmentFormatRunV1, ...]:
    if not isinstance(values, list):
        raise ValueError("fragment_format_runs must be a list")
    out = []
    for item in values:
        if not isinstance(item, dict) or set(item) != {
            "start_offset",
            "end_offset",
            "property",
            "value",
        }:
            raise ValueError("fragment format run shape is invalid")
        out.append(
            RelativeFragmentFormatRunV1(
                item["start_offset"],
                item["end_offset"],
                item["property"],
                item["value"],
            )
        )
    return tuple(out)


def _format_generation_id(
    *,
    before_state: StoryEditCoreStateV1,
    command: dict,
) -> str:
    payload = {
        "protocol_version": "chaptera.story-edit-format-generation.v1",
        "before_format_state_hash": format_state_hash_v1(before_state.format_state),
        "story_id": command["story_id"],
        "start_scalar": command["start_scalar"],
        "end_scalar": command["end_scalar"],
        "expected_before": command["expected_before"],
        "replacement_text": command["replacement_text"],
        "paragraph_inserted_ids": command["paragraph_inserted_ids"],
        "typing_format": command["typing_format"],
        "fragment_format_runs": command["fragment_format_runs"],
    }
    digest = hashlib.sha256(
        json.dumps(
            payload,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
    ).hexdigest()
    return "sha256:" + digest


def _rebase_generic_anchors(
    *,
    before: tuple[GenericAnchoredSemanticV1, ...],
    edit: StoryRangeEditV1,
) -> tuple[tuple[GenericAnchoredSemanticV1, ...], tuple[dict, ...]]:
    after = []
    changes = []
    for anchor in before:
        try:
            receipt = rebase_anchored_range_v1(
                anchored=AnchoredRangeV1(
                    anchor.start_scalar,
                    anchor.end_scalar,
                    allow_empty=anchor.allow_empty,
                ),
                policy=RangeAnchorPolicyV1(
                    anchor.start_affinity,
                    anchor.end_affinity,
                    anchor.full_cover_policy,
                ),
                edit=edit,
            )
        except RangeAnchorRebaseError as exc:
            _reject("anchored_range_rejected", str(exc))
        result = receipt.result
        after_anchor = None
        if result.status == "survives":
            assert result.range is not None
            after_anchor = GenericAnchoredSemanticV1(
                semantic_id=anchor.semantic_id,
                semantic_kind=anchor.semantic_kind,
                start_scalar=result.range.start_scalar,
                end_scalar=result.range.end_scalar,
                allow_empty=result.range.allow_empty,
                start_affinity=anchor.start_affinity,
                end_affinity=anchor.end_affinity,
                full_cover_policy=anchor.full_cover_policy,
            )
            after.append(after_anchor)
        changes.append(
            {
                "semantic_id": anchor.semantic_id,
                "semantic_kind": anchor.semantic_kind,
                "status": result.status,
                "before": _anchor_to_dict(anchor),
                "after": None if after_anchor is None else _anchor_to_dict(after_anchor),
            }
        )
    after.sort(key=lambda a: (a.semantic_id, a.semantic_kind))
    changes.sort(key=lambda item: (item["semantic_id"], item["semantic_kind"]))
    return tuple(after), tuple(changes)


def _boundary_lifecycle_required(command: dict) -> bool:
    return (
        "\r" in command["expected_before"]
        or "\r" in command["replacement_text"]
    )


def execute_story_edit_transaction_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list[dict]]:
    """Authoritative executor for RevisionKernel._commit_command."""
    if not isinstance(base_project, dict):
        _reject("invalid_story_state", "base project must be object")
    story_models = base_project.get("story_models")
    if not isinstance(story_models, dict):
        _reject("invalid_story_state", "project lacks authoritative story_models")
    story_id = command["story_id"]
    raw_state = story_models.get(story_id)
    if not isinstance(raw_state, dict):
        _reject("invalid_story_state", "requested Story model is missing")
    before = story_edit_core_state_from_dict(raw_state)

    if before.unsupported_anchored_semantics or command["incoming_semantic_kinds"]:
        _reject(
            "anchored_semantics_unsupported",
            "core Story transaction excludes unresolved feature-specific anchored semantics",
        )
    if before.active_paragraph_features and _boundary_lifecycle_required(command):
        _reject(
            "paragraph_feature_lifecycle_unsupported",
            "paragraph boundary edit requires feature lifecycle participant",
        )

    try:
        text_result = replace_story_range_in_domain_v1(
            story_id=story_id,
            story_text=before.paragraph_state.story_text,
            provenance=before.provenance,
            start_scalar=command["start_scalar"],
            end_scalar=command["end_scalar"],
            expected_before=command["expected_before"],
            replacement_text=command["replacement_text"],
        )
    except StoryEditDomainError as exc:
        _reject(exc.code, str(exc))

    paragraph_presets = tuple(
        _paragraph_properties_from_command(value)
        for value in command["paragraph_inserted_property_presets"]
    )
    paragraph_edit = ParagraphEditV1(
        base_start_scalar=command["start_scalar"],
        base_end_scalar=command["end_scalar"],
        expected_before=command["expected_before"],
        replacement_text=command["replacement_text"],
        inserted_paragraph_ids=tuple(command["paragraph_inserted_ids"]),
        inserted_property_presets=paragraph_presets,
    )
    try:
        paragraph_receipt = apply_paragraph_lifecycle_v1(
            base_state=before.paragraph_state,
            edits=(paragraph_edit,),
            expected_base_state_hash=paragraph_state_hash_v1(before.paragraph_state),
        )
    except ParagraphLifecycleError as exc:
        _reject("paragraph_lifecycle_rejected", str(exc))

    generation_id = _format_generation_id(before_state=before, command=command)
    try:
        format_receipt = plan_text_insert_format_v1(
            before_state=before.format_state,
            edit_start_scalar=command["start_scalar"],
            edit_end_scalar=command["end_scalar"],
            replacement_text=command["replacement_text"],
            post_edit_revision_id=generation_id,
            typing_snapshot=_typing_snapshot_from_command(command["typing_format"]),
            fragment_runs=_fragment_runs_from_command(command["fragment_format_runs"]),
            empty_story_preset_format=before.empty_story_preset_format,
        )
    except (TextInsertFormatError, ValueError) as exc:
        _reject("text_format_rejected", str(exc))

    edit = StoryRangeEditV1(
        command["start_scalar"],
        command["end_scalar"],
        len(command["replacement_text"]),
    )
    after_anchors, anchor_changes = _rebase_generic_anchors(
        before=before.generic_anchors,
        edit=edit,
    )

    if paragraph_receipt.after_state.story_text != text_result.after_text:
        _reject("candidate_invariant_failed", "paragraph participant text differs from Story result")
    if format_receipt.after_state.story_scalar_len != len(text_result.after_text):
        _reject("candidate_invariant_failed", "format extent differs from resulting Story length")

    after = build_story_edit_core_state_v1(
        story_id=story_id,
        provenance=before.provenance,
        paragraph_state=paragraph_receipt.after_state,
        format_state=format_receipt.after_state,
        generic_anchors=after_anchors,
        unsupported_anchored_semantics=(),
        active_paragraph_features=before.active_paragraph_features,
        empty_story_preset_format=before.empty_story_preset_format,
    )

    before_id = story_edit_core_state_id_v1(before)
    after_id = story_edit_core_state_id_v1(after)
    operation = {
        "protocol_version": "chaptera.story-edit-transaction.v1",
        "kind": "story_edit_transaction",
        "story_id": story_id,
        "start_scalar": command["start_scalar"],
        "end_scalar": command["end_scalar"],
        "expected_before": command["expected_before"],
        "replacement_text": command["replacement_text"],
        "before_state_id": before_id,
        "after_state_id": after_id,
        "story_range_operation": text_result.operation,
        "paragraph_lifecycle": {
            "before_state_hash": paragraph_receipt.before_state_hash,
            "after_state_hash": paragraph_receipt.after_state_hash,
            "removed_paragraph_ids": [
                item.paragraph.paragraph_id
                for item in paragraph_receipt.removed_paragraphs
            ],
            "inserted_paragraph_ids": list(command["paragraph_inserted_ids"]),
        },
        "format_assignment": {
            "source": format_receipt.source,
            "before_state_hash": format_state_hash_v1(before.format_state),
            "after_state_hash": format_state_hash_v1(after.format_state),
            "inserted_start_scalar": format_receipt.inserted_start_scalar,
            "inserted_end_scalar": format_receipt.inserted_end_scalar,
            "format_generation_id": generation_id,
        },
        "generic_anchor_changes": list(anchor_changes),
        "inverse_state": story_edit_core_state_to_dict(before),
        "after_state": story_edit_core_state_to_dict(after),
        "layout_status": "layout_unknown",
    }

    project = copy.deepcopy(base_project)
    project["story_models"] = copy.deepcopy(story_models)
    project["story_models"][story_id] = story_edit_core_state_to_dict(after)
    stories = project.get("stories")
    if stories is None:
        stories = {}
    if not isinstance(stories, dict):
        _reject("invalid_story_state", "project stories mirror must be object when present")
    project["stories"] = copy.deepcopy(stories)
    project["stories"][story_id] = text_result.after_text
    operations = project.get("operations", [])
    if not isinstance(operations, list):
        _reject("invalid_story_state", "project operations must be a list")
    project["operations"] = list(operations) + [copy.deepcopy(operation)]

    consequences = [
        {"key": "story.text", "state": "supported", "note": None},
        {"key": "story.paragraphs", "state": "supported", "note": None},
        {"key": "story.character_format", "state": "supported", "note": None},
        {"key": "story.generic_anchors", "state": "supported", "note": None},
        {
            "key": "layout.reflow",
            "state": "unknown",
            "note": "derived_after_semantic_commit",
        },
    ]
    return operation, project, consequences


def restore_story_edit_transaction_before_v1(operation: dict) -> dict:
    if not isinstance(operation, dict) or operation.get("protocol_version") != "chaptera.story-edit-transaction.v1":
        raise ValueError("StoryEditTransactionV1 operation is required")
    before = story_edit_core_state_from_dict(operation.get("inverse_state"))
    return story_edit_core_state_to_dict(before)


def validate_story_edit_transaction_request_v1(request: dict) -> None:
    if request.get("protocol_version") != "chaptera.story-edit-transaction-intent.v1":
        raise ValueError("V1 Story edit transaction protocol_version is required")
    command = request.get("command")
    allowed = {
        "kind",
        "story_id",
        "start_scalar",
        "end_scalar",
        "expected_before",
        "replacement_text",
        "paragraph_inserted_ids",
        "paragraph_inserted_property_presets",
        "typing_format",
        "fragment_format_runs",
        "incoming_semantic_kinds",
    }
    if (
        not isinstance(command, dict)
        or command.get("kind") != "story_edit_transaction"
        or set(command) != allowed
    ):
        raise ValueError("StoryEditTransactionV1 contains non-intent/authoritative fields")
    if not isinstance(command["story_id"], str) or not command["story_id"]:
        raise ValueError("StoryEditTransactionV1 story_id is required")
    start = command["start_scalar"]
    end = command["end_scalar"]
    if (
        not isinstance(start, int)
        or isinstance(start, bool)
        or not isinstance(end, int)
        or isinstance(end, bool)
        or start < 0
        or end < start
        or end > 0xFFFFFFFF
    ):
        raise ValueError("StoryEditTransactionV1 scalar range is invalid")
    validate_scalar_sequence_v1(command["expected_before"], "expected_before")
    validate_scalar_sequence_v1(command["replacement_text"], "replacement_text")
    if "\n" in command["expected_before"] or "\n" in command["replacement_text"]:
        raise ValueError("StoryEditTransactionV1 consumes canonical U+000D text only")

    ids = command["paragraph_inserted_ids"]
    if (
        not isinstance(ids, list)
        or any(not isinstance(value, str) or not value for value in ids)
        or len(set(ids)) != len(ids)
        or len(ids) != command["replacement_text"].count("\r")
    ):
        raise ValueError(
            "StoryEditTransactionV1 requires one unique preallocated ParagraphId per inserted U+000D"
        )
    presets = command["paragraph_inserted_property_presets"]
    if not isinstance(presets, list) or len(presets) not in {0, len(ids)}:
        raise ValueError("paragraph_inserted_property_presets must be empty or match inserted IDs")
    for preset in presets:
        if preset is not None:
            _paragraph_properties_from_command(preset)

    typing = command["typing_format"]
    if typing is not None and not isinstance(typing, dict):
        raise ValueError("typing_format must be object or null")
    if isinstance(typing, dict):
        if any(not isinstance(key, str) or not key for key in typing):
            raise ValueError("typing_format property names must be strings")

    _fragment_runs_from_command(command["fragment_format_runs"])
    if typing is not None and command["fragment_format_runs"]:
        raise ValueError("typing_format and fragment_format_runs are mutually exclusive")

    incoming = command["incoming_semantic_kinds"]
    if (
        not isinstance(incoming, list)
        or any(not isinstance(value, str) or not value for value in incoming)
        or len(set(incoming)) != len(incoming)
    ):
        raise ValueError("incoming_semantic_kinds must be a unique string list")

    depends = request.get("depends_on_client_operation_id")
    if depends is not None and (not isinstance(depends, str) or len(depends) < 8):
        raise ValueError("depends_on_client_operation_id is invalid")


def validate_story_edit_transaction_operation_v1(
    command: dict,
    operation: dict,
) -> None:
    expected_fields = {
        "protocol_version",
        "kind",
        "story_id",
        "start_scalar",
        "end_scalar",
        "expected_before",
        "replacement_text",
        "before_state_id",
        "after_state_id",
        "story_range_operation",
        "paragraph_lifecycle",
        "format_assignment",
        "generic_anchor_changes",
        "inverse_state",
        "after_state",
        "layout_status",
    }
    if not isinstance(operation, dict) or set(operation) != expected_fields:
        raise ValueError("canonical StoryEditTransactionV1 fields are not exact")
    if operation.get("protocol_version") != "chaptera.story-edit-transaction.v1":
        raise ValueError("canonical StoryEditTransactionV1 protocol mismatch")
    for field in (
        "kind",
        "story_id",
        "start_scalar",
        "end_scalar",
        "expected_before",
        "replacement_text",
    ):
        if operation.get(field) != command.get(field):
            raise ValueError(f"canonical StoryEditTransactionV1 {field} differs from intent")

    range_command = {
        "kind": "replace_story_range",
        "story_id": command["story_id"],
        "start_scalar": command["start_scalar"],
        "end_scalar": command["end_scalar"],
        "expected_before": command["expected_before"],
        "replacement_text": command["replacement_text"],
    }
    validate_story_range_operation_v1(
        range_command,
        operation["story_range_operation"],
    )

    try:
        before = story_edit_core_state_from_dict(operation["inverse_state"])
        after = story_edit_core_state_from_dict(operation["after_state"])
    except StoryEditTransactionError as exc:
        raise ValueError(str(exc)) from exc

    before_id = story_edit_core_state_id_v1(before)
    after_id = story_edit_core_state_id_v1(after)
    if operation["before_state_id"] != before_id:
        raise ValueError("canonical Story transaction before_state_id mismatch")
    if operation["after_state_id"] != after_id:
        raise ValueError("canonical Story transaction after_state_id mismatch")
    if before.story_id != command["story_id"] or after.story_id != command["story_id"]:
        raise ValueError("canonical Story transaction state targets wrong Story")

    embedded = operation["story_range_operation"]
    if story_text_hash_v1(before.paragraph_state.story_text) != embedded["before_text_hash"]:
        raise ValueError("canonical Story transaction before text hash mismatch")
    if story_text_hash_v1(after.paragraph_state.story_text) != embedded["after_text_hash"]:
        raise ValueError("canonical Story transaction after text hash mismatch")

    paragraph_meta = operation["paragraph_lifecycle"]
    if not isinstance(paragraph_meta, dict) or set(paragraph_meta) != {
        "before_state_hash",
        "after_state_hash",
        "removed_paragraph_ids",
        "inserted_paragraph_ids",
    }:
        raise ValueError("canonical paragraph lifecycle receipt shape mismatch")
    if paragraph_meta["before_state_hash"] != paragraph_state_hash_v1(before.paragraph_state):
        raise ValueError("canonical paragraph before-state hash mismatch")
    if paragraph_meta["after_state_hash"] != paragraph_state_hash_v1(after.paragraph_state):
        raise ValueError("canonical paragraph after-state hash mismatch")
    if paragraph_meta["inserted_paragraph_ids"] != command["paragraph_inserted_ids"]:
        raise ValueError("canonical inserted ParagraphIds differ from intent")

    format_meta = operation["format_assignment"]
    if not isinstance(format_meta, dict) or set(format_meta) != {
        "source",
        "before_state_hash",
        "after_state_hash",
        "inserted_start_scalar",
        "inserted_end_scalar",
        "format_generation_id",
    }:
        raise ValueError("canonical format assignment receipt shape mismatch")
    if format_meta["before_state_hash"] != format_state_hash_v1(before.format_state):
        raise ValueError("canonical format before-state hash mismatch")
    if format_meta["after_state_hash"] != format_state_hash_v1(after.format_state):
        raise ValueError("canonical format after-state hash mismatch")
    expected_insert_end = command["start_scalar"] + len(command["replacement_text"])
    if (
        format_meta["inserted_start_scalar"] != command["start_scalar"]
        or format_meta["inserted_end_scalar"] != expected_insert_end
    ):
        raise ValueError("canonical inserted formatting range mismatch")

    edit = StoryRangeEditV1(
        command["start_scalar"],
        command["end_scalar"],
        len(command["replacement_text"]),
    )
    try:
        expected_anchors, expected_changes = _rebase_generic_anchors(
            before=before.generic_anchors,
            edit=edit,
        )
    except StoryEditTransactionError as exc:
        raise ValueError(str(exc)) from exc
    if tuple(after.generic_anchors) != tuple(expected_anchors):
        raise ValueError("canonical generic anchor state violates shared transform")
    if operation["generic_anchor_changes"] != list(expected_changes):
        raise ValueError("canonical generic anchor change receipt mismatch")

    if operation["layout_status"] != "layout_unknown":
        raise ValueError("core Story transaction layout status must be layout_unknown")
