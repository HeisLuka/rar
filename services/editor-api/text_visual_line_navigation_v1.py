#!/usr/bin/env python3
"""Deterministic horizontal-LTR visual-line text navigation V1.

Canonical Story scalar positions remain identity. Authoritative resolved caret
geometry is used only to select among already-admitted physical caret stops.
Browser/toolkit Home/End/Up/Down behavior is never an input.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from resolved_text_caret_map_v1 import (
    CaretStopV1,
    ResolvedLineFragmentV1,
    ResolvedTextCaretMapError,
    ResolvedTextCaretMapV1,
    resolve_story_position_v1,
)
from story_edit_domain_v1 import StoryEditDomainV1
from text_selection_state_v1 import (
    TextSelectionStateError,
    TextSelectionStateV1,
    build_text_selection_state_v1,
    project_selection_state_v1,
    validate_selection_state_v1,
)


TextVisualLineNavigationCommandV1 = Literal[
    "move_line_start",
    "move_line_end",
    "extend_line_start",
    "extend_line_end",
    "move_visual_line_up",
    "move_visual_line_down",
    "extend_visual_line_up",
    "extend_visual_line_down",
]


class TextVisualLineNavigationError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextVisualLineNavigationResultV1:
    protocol_version: Literal["chaptera.text-visual-line-navigation-result.v1"]
    command: TextVisualLineNavigationCommandV1
    status: Literal[
        "moved",
        "boundary_no_op",
        "navigation_unsupported",
        "reconcile_required",
    ]
    selection: TextSelectionStateV1
    source_line_id: str | None
    target_line_id: str | None
    target_stop_id: str | None
    preferred_inline_x_emu: int | None
    reason: str | None
    authoring_mutation_count: Literal[0]
    undo_history_entry_count: Literal[0]


_HOME_END = {
    "move_line_start",
    "move_line_end",
    "extend_line_start",
    "extend_line_end",
}
_VERTICAL = {
    "move_visual_line_up",
    "move_visual_line_down",
    "extend_visual_line_up",
    "extend_visual_line_down",
}
_EXTEND = {
    "extend_line_start",
    "extend_line_end",
    "extend_visual_line_up",
    "extend_visual_line_down",
}


def _fail(code: str, message: str) -> None:
    raise TextVisualLineNavigationError(code, message)


def _result(
    *,
    command: TextVisualLineNavigationCommandV1,
    status: Literal[
        "moved",
        "boundary_no_op",
        "navigation_unsupported",
        "reconcile_required",
    ],
    selection: TextSelectionStateV1,
    source_line_id: str | None,
    target_line_id: str | None,
    target_stop_id: str | None,
    preferred_inline_x_emu: int | None,
    reason: str | None,
) -> TextVisualLineNavigationResultV1:
    return TextVisualLineNavigationResultV1(
        protocol_version="chaptera.text-visual-line-navigation-result.v1",
        command=command,
        status=status,
        selection=selection,
        source_line_id=source_line_id,
        target_line_id=target_line_id,
        target_stop_id=target_stop_id,
        preferred_inline_x_emu=preferred_inline_x_emu,
        reason=reason,
        authoring_mutation_count=0,
        undo_history_entry_count=0,
    )


def _validate_command(command: str) -> TextVisualLineNavigationCommandV1:
    if command not in _HOME_END | _VERTICAL:
        _fail(
            "unsupported_navigation_command",
            "V1 supports only visual-line Home/End/Up/Down movement and extension",
        )
    return command


def _validate_context(
    *,
    state: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    expected_layout_revision_id: str,
) -> None:
    if not isinstance(state, TextSelectionStateV1):
        _fail("invalid_selection_state", "TextSelectionStateV1 is required")
    if not isinstance(domain, StoryEditDomainV1):
        _fail("invalid_edit_domain", "StoryEditDomainV1 is required")
    if not isinstance(caret_map, ResolvedTextCaretMapV1):
        _fail("invalid_caret_map", "ResolvedTextCaretMapV1 is required")
    if not isinstance(expected_layout_revision_id, str) or not expected_layout_revision_id:
        _fail("invalid_layout_revision", "expected_layout_revision_id is required")
    try:
        validate_selection_state_v1(
            state=state,
            domain=domain,
            expected_revision_id=state.revision_id,
        )
    except TextSelectionStateError as exc:
        if exc.code in {
            "selection_reconcile_required",
            "stale_selection_revision",
            "edit_domain_unknown",
        }:
            _fail("reconcile_required", str(exc))
        _fail(exc.code, str(exc))

    if domain.story_id != caret_map.story_id or state.story_id != caret_map.story_id:
        _fail("reconcile_required", "selection/domain/caret map target different Stories")
    if caret_map.story_scalar_len != domain.raw_scalar_len:
        _fail("reconcile_required", "caret map Story extent differs from edit domain")
    if caret_map.layout_revision_id != expected_layout_revision_id:
        _fail("reconcile_required", "caret map layout receipt is stale")
    if (
        state.projection_state == "projected"
        and state.layout_revision_id != caret_map.layout_revision_id
    ):
        _fail(
            "reconcile_required",
            "projected selection belongs to an incompatible layout receipt",
        )
    if domain.caret_start_boundary is None or domain.caret_end_boundary is None:
        _fail("reconcile_required", "ordinary Story caret domain is unavailable")


def _admitted_stops(
    *,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    line_id: str | None = None,
) -> tuple[CaretStopV1, ...]:
    assert domain.caret_start_boundary is not None
    assert domain.caret_end_boundary is not None
    stops = [
        stop
        for stop in caret_map.caret_stops
        if domain.caret_start_boundary
        <= stop.scalar_boundary
        <= domain.caret_end_boundary
        and (line_id is None or stop.line_id == line_id)
    ]
    return tuple(
        sorted(
            stops,
            key=lambda stop: (
                stop.flow_ordinal,
                stop.frame_x_emu,
                stop.scalar_boundary,
                stop.stop_id,
            ),
        )
    )


def _line_by_id(
    caret_map: ResolvedTextCaretMapV1,
    line_id: str,
) -> ResolvedLineFragmentV1 | None:
    matches = [line for line in caret_map.lines if line.line_id == line_id]
    if len(matches) > 1:
        _fail("invalid_caret_map", "line_id must be unique")
    return None if not matches else matches[0]


def _focus_stop(
    *,
    state: TextSelectionStateV1,
    caret_map: ResolvedTextCaretMapV1,
) -> CaretStopV1:
    stop_id = (
        state.focus_visual_stop_id
        if state.projection_state == "projected"
        and state.layout_revision_id == caret_map.layout_revision_id
        else None
    )
    try:
        return resolve_story_position_v1(
            caret_map=caret_map,
            scalar_boundary=state.focus_scalar,
            stop_id=stop_id,
            expected_layout_revision_id=caret_map.layout_revision_id,
        )
    except ResolvedTextCaretMapError as exc:
        if exc.code in {
            "caret_affinity_required",
            "invalid_caret_affinity",
            "internal_cluster_unsupported",
            "unplaced_story_position",
        }:
            _fail("navigation_unsupported", f"{exc.code}:{exc}")
        if exc.code == "stale_layout_map":
            _fail("reconcile_required", str(exc))
        _fail(exc.code, str(exc))


def _home_end_target(
    *,
    command: TextVisualLineNavigationCommandV1,
    current_line: ResolvedLineFragmentV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
) -> CaretStopV1:
    stops = _admitted_stops(
        domain=domain,
        caret_map=caret_map,
        line_id=current_line.line_id,
    )
    if not stops:
        _fail(
            "navigation_unsupported",
            "current visual line has no admitted ordinary caret stop",
        )
    by_inline = sorted(
        stops,
        key=lambda stop: (
            stop.frame_x_emu,
            stop.scalar_boundary,
            stop.stop_id,
        ),
    )
    if command in {"move_line_start", "extend_line_start"}:
        return by_inline[0]
    return by_inline[-1]


def _vertical_target(
    *,
    command: TextVisualLineNavigationCommandV1,
    current_line: ResolvedLineFragmentV1,
    preferred_inline_x_emu: int,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
) -> tuple[ResolvedLineFragmentV1 | None, CaretStopV1 | None]:
    target_id = (
        current_line.previous_line_id
        if command in {"move_visual_line_up", "extend_visual_line_up"}
        else current_line.next_line_id
    )
    if target_id is None:
        return None, None
    target_line = _line_by_id(caret_map, target_id)
    if target_line is None:
        _fail(
            "reconcile_required",
            "Story-flow line adjacency references a missing target line",
        )
    stops = _admitted_stops(
        domain=domain,
        caret_map=caret_map,
        line_id=target_line.line_id,
    )
    if not stops:
        _fail(
            "navigation_unsupported",
            "target visual line has no admitted ordinary caret stop",
        )
    target = min(
        stops,
        key=lambda stop: (
            abs(stop.frame_x_emu - preferred_inline_x_emu),
            stop.frame_x_emu,
            stop.scalar_boundary,
            stop.stop_id,
        ),
    )
    return target_line, target


def _project_target(
    *,
    state: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    target: CaretStopV1,
    extend: bool,
    preferred_inline_x_emu: int | None,
) -> TextSelectionStateV1:
    if extend:
        anchor_scalar = state.anchor_scalar
        anchor_stop_id = (
            state.anchor_visual_stop_id
            if state.projection_state == "projected"
            and state.layout_revision_id == caret_map.layout_revision_id
            else None
        )
    else:
        anchor_scalar = target.scalar_boundary
        anchor_stop_id = target.stop_id

    try:
        semantic = build_text_selection_state_v1(
            domain=domain,
            revision_id=state.revision_id,
            anchor_scalar=anchor_scalar,
            focus_scalar=target.scalar_boundary,
            preferred_inline_x_emu=preferred_inline_x_emu,
        )
        projection = project_selection_state_v1(
            state=semantic,
            domain=domain,
            caret_map=caret_map,
            anchor_stop_id=anchor_stop_id,
            focus_stop_id=target.stop_id,
        )
    except TextSelectionStateError as exc:
        if exc.code in {
            "caret_affinity_required",
            "invalid_caret_affinity",
            "internal_cluster_unsupported",
            "unplaced_story_position",
        }:
            _fail("navigation_unsupported", f"{exc.code}:{exc}")
        if exc.code in {
            "selection_reconcile_required",
            "stale_selection_revision",
        }:
            _fail("reconcile_required", str(exc))
        _fail(exc.code, str(exc))
    return projection.state


def navigate_visual_line_v1(
    *,
    command: TextVisualLineNavigationCommandV1,
    state: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    expected_layout_revision_id: str,
) -> TextVisualLineNavigationResultV1:
    command = _validate_command(command)
    try:
        _validate_context(
            state=state,
            domain=domain,
            caret_map=caret_map,
            expected_layout_revision_id=expected_layout_revision_id,
        )
        current_stop = _focus_stop(state=state, caret_map=caret_map)
        current_line = _line_by_id(caret_map, current_stop.line_id)
        if current_line is None:
            _fail(
                "reconcile_required",
                "focus caret stop references a missing visual line",
            )
        admitted_focus = {
            stop.stop_id
            for stop in _admitted_stops(
                domain=domain,
                caret_map=caret_map,
                line_id=current_line.line_id,
            )
        }
        if current_stop.stop_id not in admitted_focus:
            _fail(
                "navigation_unsupported",
                "focus caret stop is outside ordinary editable domain",
            )

        if command in _HOME_END:
            target = _home_end_target(
                command=command,
                current_line=current_line,
                domain=domain,
                caret_map=caret_map,
            )
            selection = _project_target(
                state=state,
                domain=domain,
                caret_map=caret_map,
                target=target,
                extend=command in _EXTEND,
                preferred_inline_x_emu=None,
            )
            return _result(
                command=command,
                status="moved",
                selection=selection,
                source_line_id=current_line.line_id,
                target_line_id=current_line.line_id,
                target_stop_id=target.stop_id,
                preferred_inline_x_emu=None,
                reason=None,
            )

        preferred = (
            state.preferred_inline_x_emu
            if state.preferred_inline_x_emu is not None
            else current_stop.frame_x_emu
        )
        target_line, target = _vertical_target(
            command=command,
            current_line=current_line,
            preferred_inline_x_emu=preferred,
            domain=domain,
            caret_map=caret_map,
        )
        if target_line is None or target is None:
            no_op = replace(state, preferred_inline_x_emu=preferred)
            return _result(
                command=command,
                status="boundary_no_op",
                selection=no_op,
                source_line_id=current_line.line_id,
                target_line_id=None,
                target_stop_id=None,
                preferred_inline_x_emu=preferred,
                reason="no adjacent Story-flow visual line",
            )
        selection = _project_target(
            state=state,
            domain=domain,
            caret_map=caret_map,
            target=target,
            extend=command in _EXTEND,
            preferred_inline_x_emu=preferred,
        )
        return _result(
            command=command,
            status="moved",
            selection=selection,
            source_line_id=current_line.line_id,
            target_line_id=target_line.line_id,
            target_stop_id=target.stop_id,
            preferred_inline_x_emu=preferred,
            reason=None,
        )
    except TextVisualLineNavigationError as exc:
        if exc.code == "reconcile_required":
            return _result(
                command=command,
                status="reconcile_required",
                selection=state,
                source_line_id=None,
                target_line_id=None,
                target_stop_id=None,
                preferred_inline_x_emu=None,
                reason=str(exc),
            )
        if exc.code == "navigation_unsupported":
            return _result(
                command=command,
                status="navigation_unsupported",
                selection=state,
                source_line_id=None,
                target_line_id=None,
                target_stop_id=None,
                preferred_inline_x_emu=state.preferred_inline_x_emu,
                reason=str(exc),
            )
        raise
