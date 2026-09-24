#!/usr/bin/env python3
"""Interaction admissibility for placed vs unplaced canonical Story selections V1.

Canonical semantic ranges and physical caret placement are separate authorities.
This module never mutates Story text, selection, layout, viewport, or history.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from resolved_text_caret_map_v1 import (
    ResolvedTextCaretMapError,
    ResolvedTextCaretMapV1,
    SelectionGeometryV1,
    resolve_story_position_v1,
    selection_geometry_v1,
)
from text_selection_state_v1 import TextSelectionStateV1


SemanticRangeCommandV1 = Literal[
    "select_all",
    "copy",
    "cut",
    "range_format",
    "find_replace",
    "programmatic_inspection",
    "explicit_range_delete",
    "explicit_range_replace",
]
DirectCaretCommandV1 = Literal[
    "typing",
    "ime_start",
    "collapsed_format",
    "pointer_placement",
    "caret_navigation",
]
UnplacedCommandV1 = SemanticRangeCommandV1 | DirectCaretCommandV1


class TextUnplacedInteractionError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextInteractionAdmissibilityV1:
    protocol_version: Literal["chaptera.text-interaction-admissibility.v1"]
    story_id: str
    command_kind: str
    admitted: bool
    mode: Literal["semantic_range", "direct_caret"]
    reason: str | None
    geometry_state: Literal["complete", "partial", "unplaced", "unsupported"]
    selection_nonempty: bool
    focus_scalar: int
    focus_stop_id: str | None
    authoring_mutation_count: Literal[0]
    undo_history_entry_count: Literal[0]


_SEMANTIC = {
    "select_all",
    "copy",
    "cut",
    "range_format",
    "find_replace",
    "programmatic_inspection",
    "explicit_range_delete",
    "explicit_range_replace",
}
_DIRECT = {
    "typing",
    "ime_start",
    "collapsed_format",
    "pointer_placement",
    "caret_navigation",
}
_REQUIRES_NONEMPTY = {
    "copy",
    "cut",
    "range_format",
    "find_replace",
    "explicit_range_delete",
    "explicit_range_replace",
}


def _fail(code: str, message: str) -> None:
    raise TextUnplacedInteractionError(code, message)


def _validate_inputs(
    *,
    selection: TextSelectionStateV1,
    caret_map: ResolvedTextCaretMapV1,
    command_kind: str,
    expected_layout_revision_id: str,
) -> tuple[str, int, int]:
    if not isinstance(selection, TextSelectionStateV1):
        _fail("invalid_selection", "TextSelectionStateV1 is required")
    if not isinstance(caret_map, ResolvedTextCaretMapV1):
        _fail("invalid_caret_map", "ResolvedTextCaretMapV1 is required")
    if selection.story_id != caret_map.story_id:
        _fail("reconcile_required", "selection and caret map target different Stories")
    if (
        not isinstance(expected_layout_revision_id, str)
        or not expected_layout_revision_id
        or caret_map.layout_revision_id != expected_layout_revision_id
    ):
        _fail("reconcile_required", "caret map layout receipt is stale")
    if command_kind in _SEMANTIC:
        mode = "semantic_range"
    elif command_kind in _DIRECT:
        mode = "direct_caret"
    else:
        _fail("unsupported_interaction_command", "command is outside TextUnplacedInteractionV1")
    start, end = selection.normalized_range
    if end > caret_map.story_scalar_len:
        _fail("reconcile_required", "selection exceeds current caret-map Story extent")
    return mode, start, end


def _geometry(
    *,
    caret_map: ResolvedTextCaretMapV1,
    start: int,
    end: int,
    expected_layout_revision_id: str,
) -> SelectionGeometryV1:
    try:
        return selection_geometry_v1(
            caret_map=caret_map,
            start_scalar=start,
            end_scalar=end,
            expected_layout_revision_id=expected_layout_revision_id,
        )
    except ResolvedTextCaretMapError as exc:
        if exc.code == "stale_layout_map":
            _fail("reconcile_required", str(exc))
        _fail(exc.code, str(exc))


def decide_text_interaction_admissibility_v1(
    *,
    selection: TextSelectionStateV1,
    caret_map: ResolvedTextCaretMapV1,
    command_kind: UnplacedCommandV1,
    expected_layout_revision_id: str,
) -> TextInteractionAdmissibilityV1:
    mode, start, end = _validate_inputs(
        selection=selection,
        caret_map=caret_map,
        command_kind=command_kind,
        expected_layout_revision_id=expected_layout_revision_id,
    )
    geometry = _geometry(
        caret_map=caret_map,
        start=start,
        end=end,
        expected_layout_revision_id=expected_layout_revision_id,
    )
    nonempty = start != end

    if mode == "semantic_range":
        if command_kind in _REQUIRES_NONEMPTY and not nonempty:
            return TextInteractionAdmissibilityV1(
                protocol_version="chaptera.text-interaction-admissibility.v1",
                story_id=selection.story_id,
                command_kind=command_kind,
                admitted=False,
                mode=mode,
                reason="semantic_range_required",
                geometry_state=geometry.coverage_state,
                selection_nonempty=False,
                focus_scalar=selection.focus_scalar,
                focus_stop_id=None,
                authoring_mutation_count=0,
                undo_history_entry_count=0,
            )
        if geometry.coverage_state == "unsupported":
            # This is not an unplaced tail: layout materialized the range but
            # lacks authoritative internal caret/selection boundaries.
            return TextInteractionAdmissibilityV1(
                protocol_version="chaptera.text-interaction-admissibility.v1",
                story_id=selection.story_id,
                command_kind=command_kind,
                admitted=False,
                mode=mode,
                reason="selection_geometry_unsupported",
                geometry_state=geometry.coverage_state,
                selection_nonempty=nonempty,
                focus_scalar=selection.focus_scalar,
                focus_stop_id=None,
                authoring_mutation_count=0,
                undo_history_entry_count=0,
            )
        # Complete, partial, and wholly unplaced non-empty semantic ranges stay
        # canonical. Pixel coverage is not mutation authority.
        return TextInteractionAdmissibilityV1(
            protocol_version="chaptera.text-interaction-admissibility.v1",
            story_id=selection.story_id,
            command_kind=command_kind,
            admitted=True,
            mode=mode,
            reason=(
                None
                if geometry.coverage_state == "complete"
                else f"semantic_range_{geometry.coverage_state}"
            ),
            geometry_state=geometry.coverage_state,
            selection_nonempty=nonempty,
            focus_scalar=selection.focus_scalar,
            focus_stop_id=None,
            authoring_mutation_count=0,
            undo_history_entry_count=0,
        )

    # Direct caret interaction is always about the focus insertion point, even
    # when a non-empty semantic selection exists. Never substitute the last
    # visible line or nearest frame for an unplaced focus.
    stop_id = (
        selection.focus_visual_stop_id
        if selection.projection_state == "projected"
        and selection.layout_revision_id == caret_map.layout_revision_id
        else None
    )
    try:
        stop = resolve_story_position_v1(
            caret_map=caret_map,
            scalar_boundary=selection.focus_scalar,
            stop_id=stop_id,
            expected_layout_revision_id=expected_layout_revision_id,
        )
    except ResolvedTextCaretMapError as exc:
        if exc.code == "unplaced_story_position":
            reason = "direct_edit_unavailable:unplaced"
        elif exc.code in {
            "internal_cluster_unsupported",
            "caret_affinity_required",
            "invalid_caret_affinity",
        }:
            reason = "caret_geometry_unavailable"
        elif exc.code == "stale_layout_map":
            _fail("reconcile_required", str(exc))
        else:
            _fail(exc.code, str(exc))
        return TextInteractionAdmissibilityV1(
            protocol_version="chaptera.text-interaction-admissibility.v1",
            story_id=selection.story_id,
            command_kind=command_kind,
            admitted=False,
            mode=mode,
            reason=reason,
            geometry_state=geometry.coverage_state,
            selection_nonempty=nonempty,
            focus_scalar=selection.focus_scalar,
            focus_stop_id=None,
            authoring_mutation_count=0,
            undo_history_entry_count=0,
        )

    return TextInteractionAdmissibilityV1(
        protocol_version="chaptera.text-interaction-admissibility.v1",
        story_id=selection.story_id,
        command_kind=command_kind,
        admitted=True,
        mode=mode,
        reason=None,
        geometry_state=geometry.coverage_state,
        selection_nonempty=nonempty,
        focus_scalar=selection.focus_scalar,
        focus_stop_id=stop.stop_id,
        authoring_mutation_count=0,
        undo_history_entry_count=0,
    )
