#!/usr/bin/env python3
"""Atomic revision-bound same-Story literal Find/Replace V1."""

from __future__ import annotations

import copy
from dataclasses import dataclass
from typing import Any, Literal

from paragraph_lifecycle_v1 import (
    ParagraphEditV1,
    ParagraphLifecycleError,
    apply_paragraph_lifecycle_v1,
    state_hash_v1 as paragraph_state_hash_v1,
)
from range_anchor_rebase_v1 import (
    AnchoredRangeV1,
    RangeAnchorPolicyV1,
    StoryRangeEditV1,
    rebase_anchored_range_v1,
)
from story_edit_domain_v1 import (
    StoryEditDomainError,
    derive_story_edit_domain_v1,
    validate_ordinary_story_range_v1,
)
from story_edit_transaction_v1 import (
    GenericAnchoredSemanticV1,
    StoryEditTransactionError,
    build_story_edit_core_state_v1,
    story_edit_core_state_from_dict,
    story_edit_core_state_id_v1,
    story_edit_core_state_to_dict,
)
from text_find_snapshot_v1 import (
    TextFindMatchV1,
    TextFindSnapshotV1,
    TextFindError,
    validate_text_find_snapshot_current_v1,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    TextFormatOverlayStateV1,
    effective_property_segments_v1,
    state_hash_v1 as format_state_hash_v1,
)
from text_ingress_v1 import normalize_external_text_v1
from text_insert_format_v1 import (
    TextInsertFormatError,
    TypingFormatSnapshotV1,
    plan_text_insert_format_v1,
)


class StoryFindReplaceError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class NormalizedFindReplaceEditV1:
    edit_ordinal: int
    snapshot_match_ordinal: int
    base_start_scalar: int
    base_end_scalar: int
    expected_before: str
    replacement_text: str
    inserted_start_scalar: int
    inserted_end_scalar: int
    inserted_paragraph_ids: tuple[str, ...]


def _reject(code: str, message: str) -> None:
    raise StoryFindReplaceError(code, message)


def _snapshot_from_dict(value: dict) -> TextFindSnapshotV1:
    if not isinstance(value, dict):
        _reject("invalid_find_snapshot", "find_snapshot must be object")
    expected = {
        "protocol_version",
        "policy_version",
        "revision_id",
        "story_id",
        "query",
        "extent_start_scalar",
        "extent_end_scalar",
        "matches",
    }
    if set(value) != expected:
        _reject("invalid_find_snapshot", "find_snapshot fields are not exact V1")
    if value["protocol_version"] != "chaptera.text-find-snapshot.v1":
        _reject("invalid_find_snapshot", "find_snapshot protocol mismatch")
    if value["policy_version"] != "chaptera.text-find-policy.v1":
        _reject("invalid_find_snapshot", "find_snapshot policy mismatch")
    matches = []
    raw_matches = value["matches"]
    if not isinstance(raw_matches, list):
        _reject("invalid_find_snapshot", "find_snapshot.matches must be list")
    for item in raw_matches:
        if not isinstance(item, dict) or set(item) != {
            "ordinal",
            "start_scalar",
            "end_scalar",
            "matched_text",
            "matched_text_sha256",
        }:
            _reject("invalid_find_snapshot", "find_snapshot match shape invalid")
        matches.append(
            TextFindMatchV1(
                ordinal=item["ordinal"],
                start_scalar=item["start_scalar"],
                end_scalar=item["end_scalar"],
                matched_text=item["matched_text"],
                matched_text_sha256=item["matched_text_sha256"],
            )
        )
    return TextFindSnapshotV1(
        protocol_version=value["protocol_version"],
        policy_version=value["policy_version"],
        revision_id=value["revision_id"],
        story_id=value["story_id"],
        query=value["query"],
        extent_start_scalar=value["extent_start_scalar"],
        extent_end_scalar=value["extent_end_scalar"],
        matches=tuple(matches),
    )


def _uniform_effective_format(
    state: TextFormatOverlayStateV1,
    start: int,
    end: int,
) -> BaseCharacterFormatV1:
    values: dict[str, Any] = {}
    for prop in ("font_size_emu", "bold", "italic", "text_color_rgb"):
        segments = effective_property_segments_v1(
            state=state,
            prop=prop,
            start_scalar=start,
            end_scalar=end,
        )
        unique = {segment.value for segment in segments}
        if len(unique) != 1:
            _reject(
                "mixed_format_match_unsupported",
                "selected match has mixed effective character formatting",
            )
        values[prop] = next(iter(unique))

    resource_ids = {
        run.format.font_resource_id
        for run in state.base_runs
        if start < run.end_scalar and run.start_scalar < end
    }
    if len(resource_ids) != 1:
        _reject(
            "mixed_format_match_unsupported",
            "selected match crosses resolved font resources",
        )
    return BaseCharacterFormatV1(
        font_resource_id=next(iter(resource_ids)),
        font_size_emu=values["font_size_emu"],
        bold=values["bold"],
        italic=values["italic"],
        text_color_rgb=values["text_color_rgb"],
    )


def _typing_snapshot(fmt: BaseCharacterFormatV1) -> TypingFormatSnapshotV1:
    return TypingFormatSnapshotV1(
        (
            ("bold", fmt.bold),
            ("font_size_emu", fmt.font_size_emu),
            ("italic", fmt.italic),
            ("text_color_rgb", fmt.text_color_rgb),
        )
    )


def _rebase_anchors_right_to_left(
    anchors: tuple[GenericAnchoredSemanticV1, ...],
    edits: tuple[NormalizedFindReplaceEditV1, ...],
) -> tuple[GenericAnchoredSemanticV1, ...]:
    current = list(anchors)
    for edit in reversed(edits):
        next_items = []
        range_edit = StoryRangeEditV1(
            edit.base_start_scalar,
            edit.base_end_scalar,
            len(edit.replacement_text),
        )
        for anchor in current:
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
                edit=range_edit,
            )
            if receipt.result.status != "survives" or receipt.result.range is None:
                continue
            rr = receipt.result.range
            next_items.append(
                GenericAnchoredSemanticV1(
                    semantic_id=anchor.semantic_id,
                    semantic_kind=anchor.semantic_kind,
                    start_scalar=rr.start_scalar,
                    end_scalar=rr.end_scalar,
                    allow_empty=anchor.allow_empty,
                    start_affinity=anchor.start_affinity,
                    end_affinity=anchor.end_affinity,
                    full_cover_policy=anchor.full_cover_policy,
                )
            )
        current = next_items
    return tuple(sorted(current, key=lambda x: (x.semantic_id, x.semantic_kind)))


def _final_text(
    base_text: str,
    edits: tuple[NormalizedFindReplaceEditV1, ...],
) -> str:
    text = base_text
    for edit in reversed(edits):
        text = (
            text[:edit.base_start_scalar]
            + edit.replacement_text
            + text[edit.base_end_scalar:]
        )
    return text


def _normalize_selected_matches(
    *,
    snapshot: TextFindSnapshotV1,
    selected_ordinals: list[int],
    replacement_text: str,
    paragraph_ids_by_match: list[dict],
) -> tuple[NormalizedFindReplaceEditV1, ...]:
    if (
        not isinstance(selected_ordinals, list)
        or not selected_ordinals
        or any(not isinstance(v, int) or isinstance(v, bool) for v in selected_ordinals)
        or len(set(selected_ordinals)) != len(selected_ordinals)
    ):
        _reject("invalid_selected_matches", "selected_match_ordinals must be unique non-empty integers")

    by_ordinal = {m.ordinal: m for m in snapshot.matches}
    if len(by_ordinal) != len(snapshot.matches):
        _reject("invalid_find_snapshot", "find snapshot ordinals must be unique")
    selected = []
    for ordinal in selected_ordinals:
        match = by_ordinal.get(ordinal)
        if match is None:
            _reject("invalid_selected_matches", "selected match ordinal is absent from snapshot")
        selected.append(match)
    selected.sort(key=lambda m: (m.start_scalar, m.end_scalar, m.ordinal))

    id_map = {}
    if not isinstance(paragraph_ids_by_match, list):
        _reject("invalid_paragraph_ids", "paragraph_ids_by_match must be list")
    for item in paragraph_ids_by_match:
        if not isinstance(item, dict) or set(item) != {"match_ordinal", "paragraph_ids"}:
            _reject("invalid_paragraph_ids", "paragraph id entry shape invalid")
        ordinal = item["match_ordinal"]
        ids = item["paragraph_ids"]
        if (
            not isinstance(ordinal, int)
            or isinstance(ordinal, bool)
            or ordinal in id_map
            or not isinstance(ids, list)
            or any(not isinstance(v, str) or not v for v in ids)
            or len(set(ids)) != len(ids)
        ):
            _reject("invalid_paragraph_ids", "paragraph id entry invalid")
        id_map[ordinal] = tuple(ids)

    expected_count = replacement_text.count("\r")
    out = []
    cumulative_delta = 0
    for edit_ordinal, match in enumerate(selected):
        ids = id_map.get(match.ordinal, ())
        if len(ids) != expected_count:
            _reject(
                "invalid_paragraph_ids",
                "each selected match requires one preallocated ParagraphId per replacement U+000D",
            )
        inserted_start = match.start_scalar + cumulative_delta
        inserted_end = inserted_start + len(replacement_text)
        out.append(
            NormalizedFindReplaceEditV1(
                edit_ordinal=edit_ordinal,
                snapshot_match_ordinal=match.ordinal,
                base_start_scalar=match.start_scalar,
                base_end_scalar=match.end_scalar,
                expected_before=match.matched_text,
                replacement_text=replacement_text,
                inserted_start_scalar=inserted_start,
                inserted_end_scalar=inserted_end,
                inserted_paragraph_ids=ids,
            )
        )
        cumulative_delta += len(replacement_text) - (
            match.end_scalar - match.start_scalar
        )
    return tuple(out)


def execute_story_find_replace_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list[dict]]:
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
    base_text = before.paragraph_state.story_text

    if before.unsupported_anchored_semantics:
        _reject(
            "anchored_semantics_unsupported",
            "find/replace excludes unresolved feature-specific anchored semantics",
        )

    snapshot = _snapshot_from_dict(command["find_snapshot"])
    if snapshot.story_id != story_id:
        _reject("invalid_find_snapshot", "snapshot StoryId differs from command")
    if snapshot.revision_id != command["base_story_revision_id"]:
        _reject("find_snapshot_stale", "snapshot revision differs from command base revision")

    domain = derive_story_edit_domain_v1(
        story_id=story_id,
        story_text=base_text,
        provenance=before.provenance,
    )
    try:
        validate_text_find_snapshot_current_v1(
            snapshot=snapshot,
            revision_id=command["base_story_revision_id"],
            story_id=story_id,
            story_text=base_text,
            domain=domain,
        )
    except TextFindError as exc:
        _reject(exc.code, str(exc))

    replacement = normalize_external_text_v1(
        command["external_replacement_text"]
    ).text
    edits = _normalize_selected_matches(
        snapshot=snapshot,
        selected_ordinals=command["selected_match_ordinals"],
        replacement_text=replacement,
        paragraph_ids_by_match=command["paragraph_ids_by_match"],
    )

    # Validate every selected range against the same base edit-domain.
    for edit in edits:
        try:
            validate_ordinary_story_range_v1(
                domain=domain,
                start_scalar=edit.base_start_scalar,
                end_scalar=edit.base_end_scalar,
            )
        except StoryEditDomainError as exc:
            _reject(exc.code, str(exc))
        if base_text[edit.base_start_scalar:edit.base_end_scalar] != edit.expected_before:
            _reject("find_snapshot_stale", "selected match text changed since snapshot")

    if before.active_paragraph_features and any(
        "\r" in edit.expected_before or "\r" in edit.replacement_text
        for edit in edits
    ):
        _reject(
            "paragraph_feature_lifecycle_unsupported",
            "paragraph boundary replacement requires feature lifecycle participant",
        )

    # Freeze each match's effective format from the immutable base state before
    # applying any edit.
    frozen_formats = {
        edit.snapshot_match_ordinal: _uniform_effective_format(
            before.format_state,
            edit.base_start_scalar,
            edit.base_end_scalar,
        )
        for edit in edits
    }

    paragraph_edits = tuple(
        ParagraphEditV1(
            base_start_scalar=e.base_start_scalar,
            base_end_scalar=e.base_end_scalar,
            expected_before=e.expected_before,
            replacement_text=e.replacement_text,
            inserted_paragraph_ids=e.inserted_paragraph_ids,
            inserted_property_presets=tuple(None for _ in e.inserted_paragraph_ids),
        )
        for e in edits
    )
    try:
        paragraph_receipt = apply_paragraph_lifecycle_v1(
            base_state=before.paragraph_state,
            edits=paragraph_edits,
            expected_base_state_hash=paragraph_state_hash_v1(before.paragraph_state),
        )
    except ParagraphLifecycleError as exc:
        _reject("paragraph_lifecycle_rejected", str(exc))

    # Internal optimization: disjoint edits are applied right-to-left so every
    # coordinate remains a base coordinate. Frozen per-match format comes only
    # from the original base state.
    format_state = before.format_state
    for e in reversed(edits):
        fmt = frozen_formats[e.snapshot_match_ordinal]
        try:
            receipt = plan_text_insert_format_v1(
                before_state=format_state,
                edit_start_scalar=e.base_start_scalar,
                edit_end_scalar=e.base_end_scalar,
                replacement_text=e.replacement_text,
                post_edit_revision_id=command["format_generation_id"],
                typing_snapshot=(
                    None if not e.replacement_text else _typing_snapshot(fmt)
                ),
                empty_story_preset_format=before.empty_story_preset_format,
            )
        except TextInsertFormatError as exc:
            _reject("text_format_rejected", str(exc))
        format_state = receipt.after_state

    after_anchors = _rebase_anchors_right_to_left(before.generic_anchors, edits)
    final_text = _final_text(base_text, edits)
    if paragraph_receipt.after_state.story_text != final_text:
        _reject("candidate_invariant_failed", "paragraph lifecycle text differs from composite Story")
    if format_state.story_scalar_len != len(final_text):
        _reject("candidate_invariant_failed", "format extent differs from composite Story")

    after = build_story_edit_core_state_v1(
        story_id=story_id,
        provenance=before.provenance,
        paragraph_state=paragraph_receipt.after_state,
        format_state=format_state,
        generic_anchors=after_anchors,
        unsupported_anchored_semantics=(),
        active_paragraph_features=before.active_paragraph_features,
        empty_story_preset_format=before.empty_story_preset_format,
    )

    operation = {
        "protocol_version": "chaptera.story-find-replace.v1",
        "kind": "story_find_replace",
        "story_id": story_id,
        "base_story_revision_id": command["base_story_revision_id"],
        "query": snapshot.query,
        "selected_match_ordinals": [e.snapshot_match_ordinal for e in edits],
        "replacement_text": replacement,
        "normalized_edits": [
            {
                "edit_ordinal": e.edit_ordinal,
                "snapshot_match_ordinal": e.snapshot_match_ordinal,
                "base_start_scalar": e.base_start_scalar,
                "base_end_scalar": e.base_end_scalar,
                "inserted_start_scalar": e.inserted_start_scalar,
                "inserted_end_scalar": e.inserted_end_scalar,
                "inserted_paragraph_ids": list(e.inserted_paragraph_ids),
            }
            for e in edits
        ],
        "before_state_id": story_edit_core_state_id_v1(before),
        "after_state_id": story_edit_core_state_id_v1(after),
        "paragraph_before_hash": paragraph_state_hash_v1(before.paragraph_state),
        "paragraph_after_hash": paragraph_state_hash_v1(after.paragraph_state),
        "format_before_hash": format_state_hash_v1(before.format_state),
        "format_after_hash": format_state_hash_v1(after.format_state),
        "inverse_state": story_edit_core_state_to_dict(before),
        "after_state": story_edit_core_state_to_dict(after),
        "snapshot_staled": True,
        "layout_status": "layout_unknown",
    }

    project = copy.deepcopy(base_project)
    project["story_models"] = copy.deepcopy(story_models)
    project["story_models"][story_id] = story_edit_core_state_to_dict(after)
    stories = project.get("stories", {})
    if not isinstance(stories, dict):
        _reject("invalid_story_state", "project stories mirror must be object")
    project["stories"] = copy.deepcopy(stories)
    project["stories"][story_id] = final_text
    operations = project.get("operations", [])
    if not isinstance(operations, list):
        _reject("invalid_story_state", "project operations must be list")
    project["operations"] = list(operations) + [copy.deepcopy(operation)]

    consequences = [
        {"key": "story.text", "state": "supported", "note": None},
        {"key": "story.paragraphs", "state": "supported", "note": None},
        {"key": "story.character_format", "state": "supported", "note": None},
        {"key": "story.generic_anchors", "state": "supported", "note": None},
        {"key": "find.snapshot", "state": "stale", "note": "regenerate_after_commit"},
        {"key": "layout.reflow", "state": "unknown", "note": "derived_after_semantic_commit"},
    ]
    return operation, project, consequences


def validate_story_find_replace_request_v1(request: dict) -> None:
    if request.get("protocol_version") != "chaptera.story-find-replace-intent.v1":
        raise ValueError("StoryFindReplaceV1 protocol_version is required")
    command = request.get("command")
    allowed = {
        "kind",
        "story_id",
        "base_story_revision_id",
        "find_snapshot",
        "selected_match_ordinals",
        "external_replacement_text",
        "paragraph_ids_by_match",
        "format_generation_id",
    }
    if (
        not isinstance(command, dict)
        or command.get("kind") != "story_find_replace"
        or set(command) != allowed
    ):
        raise ValueError("StoryFindReplaceV1 intent fields are not exact")
    for field in ("story_id", "base_story_revision_id", "format_generation_id"):
        if not isinstance(command[field], str) or not command[field]:
            raise ValueError(f"{field} is required")
    if not isinstance(command["external_replacement_text"], str):
        raise ValueError("external_replacement_text must be string")
    _snapshot_from_dict(command["find_snapshot"])
    _normalize_selected_matches(
        snapshot=_snapshot_from_dict(command["find_snapshot"]),
        selected_ordinals=command["selected_match_ordinals"],
        replacement_text=normalize_external_text_v1(command["external_replacement_text"]).text,
        paragraph_ids_by_match=command["paragraph_ids_by_match"],
    )


def validate_story_find_replace_operation_v1(command: dict, operation: dict) -> None:
    expected = {
        "protocol_version",
        "kind",
        "story_id",
        "base_story_revision_id",
        "query",
        "selected_match_ordinals",
        "replacement_text",
        "normalized_edits",
        "before_state_id",
        "after_state_id",
        "paragraph_before_hash",
        "paragraph_after_hash",
        "format_before_hash",
        "format_after_hash",
        "inverse_state",
        "after_state",
        "snapshot_staled",
        "layout_status",
    }
    if not isinstance(operation, dict) or set(operation) != expected:
        raise ValueError("StoryFindReplaceV1 canonical fields are not exact")
    if operation["protocol_version"] != "chaptera.story-find-replace.v1":
        raise ValueError("StoryFindReplaceV1 protocol mismatch")
    if operation["kind"] != command["kind"] or operation["story_id"] != command["story_id"]:
        raise ValueError("StoryFindReplaceV1 canonical target differs from intent")
    if operation["base_story_revision_id"] != command["base_story_revision_id"]:
        raise ValueError("StoryFindReplaceV1 base revision differs from intent")
    before = story_edit_core_state_from_dict(operation["inverse_state"])
    after = story_edit_core_state_from_dict(operation["after_state"])
    if operation["before_state_id"] != story_edit_core_state_id_v1(before):
        raise ValueError("StoryFindReplaceV1 before_state_id mismatch")
    if operation["after_state_id"] != story_edit_core_state_id_v1(after):
        raise ValueError("StoryFindReplaceV1 after_state_id mismatch")
    if operation["paragraph_before_hash"] != paragraph_state_hash_v1(before.paragraph_state):
        raise ValueError("StoryFindReplaceV1 paragraph before hash mismatch")
    if operation["paragraph_after_hash"] != paragraph_state_hash_v1(after.paragraph_state):
        raise ValueError("StoryFindReplaceV1 paragraph after hash mismatch")
    if operation["format_before_hash"] != format_state_hash_v1(before.format_state):
        raise ValueError("StoryFindReplaceV1 format before hash mismatch")
    if operation["format_after_hash"] != format_state_hash_v1(after.format_state):
        raise ValueError("StoryFindReplaceV1 format after hash mismatch")
    if operation["snapshot_staled"] is not True or operation["layout_status"] != "layout_unknown":
        raise ValueError("StoryFindReplaceV1 downstream status mismatch")
