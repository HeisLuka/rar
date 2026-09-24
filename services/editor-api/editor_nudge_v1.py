#!/usr/bin/env python3
"""Single-selection canvas nudge adapter V1.

This is a narrow product adapter:
canvas focus + one admitted direct authored selection + arrow command
-> NudgePlanV1 -> one RevisionKernel MoveNode commit.

It deliberately does not implement multi-selection, nested group-member nudge,
source-backed/projected mutation, snapping, key-repeat coalescing, or Story
caret navigation.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from nudge_plan_v1 import NudgePlanError, plan_nudge_v1
from object_selection_target_v1 import DirectNodeSelectionV1, ObjectSelectionTargetV1
from revision_store import MAX_SAFE_EMU, MIN_SAFE_EMU, AuthoritativeExecutor, RevisionKernel


FocusOwnerV1 = Literal["canvas", "story", "other"]
NudgeOutcomeV1 = Literal["commit", "route_story", "ignored"]
AUTHORED_DIRECT_MOVABLE_V1 = "chaptera-authored-direct-movable-v1"


class EditorNudgeError(ValueError):
    pass


@dataclass(frozen=True)
class SelectionGeometryV1:
    x_emu: int
    y_emu: int
    width_emu: int
    height_emu: int


@dataclass(frozen=True)
class NudgeDispatchV1:
    protocol_version: Literal["chaptera.editor-nudge-dispatch.v1"]
    outcome: NudgeOutcomeV1
    reason: str
    request: dict | None


def _checked_int(value: int, label: str) -> int:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value < MIN_SAFE_EMU
        or value > MAX_SAFE_EMU
    ):
        raise EditorNudgeError(f"{label} must be a JavaScript-safe EMU integer")
    return value


def _validate_geometry(geometry: SelectionGeometryV1) -> None:
    if not isinstance(geometry, SelectionGeometryV1):
        raise EditorNudgeError("selected geometry is required")
    _checked_int(geometry.x_emu, "geometry.x_emu")
    _checked_int(geometry.y_emu, "geometry.y_emu")
    _checked_int(geometry.width_emu, "geometry.width_emu")
    _checked_int(geometry.height_emu, "geometry.height_emu")
    if geometry.width_emu <= 0 or geometry.height_emu <= 0:
        raise EditorNudgeError("selected geometry must have positive size")
    _checked_int(geometry.x_emu + geometry.width_emu, "geometry.right_emu")
    _checked_int(geometry.y_emu + geometry.height_emu, "geometry.bottom_emu")


def _ignored(reason: str) -> NudgeDispatchV1:
    return NudgeDispatchV1(
        protocol_version="chaptera.editor-nudge-dispatch.v1",
        outcome="ignored",
        reason=reason,
        request=None,
    )


def plan_editor_nudge_v1(
    *,
    document_id: str,
    source_hash: str,
    base_revision_id: str,
    client_operation_id: str,
    focus_owner: FocusOwnerV1,
    selections: tuple[ObjectSelectionTargetV1, ...],
    selected_geometry: SelectionGeometryV1 | None,
    selected_mutation_class: str | None,
    direction: str,
    modifier_state: str,
) -> NudgeDispatchV1:
    """Plan one keyboard nudge without mutating document state."""

    for value, label in (
        (document_id, "document_id"),
        (source_hash, "source_hash"),
        (base_revision_id, "base_revision_id"),
        (client_operation_id, "client_operation_id"),
    ):
        if not isinstance(value, str) or not value:
            raise EditorNudgeError(f"{label} is required")

    if focus_owner == "story":
        return NudgeDispatchV1(
            protocol_version="chaptera.editor-nudge-dispatch.v1",
            outcome="route_story",
            reason="active_story_text_editing_owns_arrows",
            request=None,
        )
    if focus_owner != "canvas":
        return _ignored("canvas_does_not_own_keyboard")
    if not isinstance(selections, tuple) or len(selections) != 1:
        return _ignored("single_selection_required")

    target = selections[0]
    if not isinstance(target, DirectNodeSelectionV1):
        return _ignored("direct_node_selection_required")
    if selected_mutation_class != AUTHORED_DIRECT_MOVABLE_V1:
        return _ignored("authored_direct_movable_target_required")
    if selected_geometry is None:
        raise EditorNudgeError("selected_geometry is required for admitted target")

    _validate_geometry(selected_geometry)

    try:
        plan = plan_nudge_v1(
            direction=direction,
            modifier_state=modifier_state,
        )
    except NudgePlanError:
        return _ignored("unsupported_nudge_command")

    x_emu = _checked_int(
        selected_geometry.x_emu + plan.dx_emu,
        "nudge.target_x_emu",
    )
    y_emu = _checked_int(
        selected_geometry.y_emu + plan.dy_emu,
        "nudge.target_y_emu",
    )
    _checked_int(x_emu + selected_geometry.width_emu, "nudge.target_right_emu")
    _checked_int(y_emu + selected_geometry.height_emu, "nudge.target_bottom_emu")

    request = {
        "protocol_version": "chaptera.commit-request.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": base_revision_id,
        "client_operation_id": client_operation_id,
        "command": {
            "kind": "move_node_to",
            "node_id": target.node_id,
            "x_emu": x_emu,
            "y_emu": y_emu,
        },
    }
    return NudgeDispatchV1(
        protocol_version="chaptera.editor-nudge-dispatch.v1",
        outcome="commit",
        reason="single_canvas_nudge",
        request=request,
    )


def commit_editor_nudge_v1(
    *,
    kernel: RevisionKernel,
    executor: AuthoritativeExecutor,
    **plan_args,
) -> NudgeDispatchV1 | dict:
    """Commit exactly one MoveNode when the adapter admits the key command."""

    dispatch = plan_editor_nudge_v1(**plan_args)
    if dispatch.outcome != "commit":
        return dispatch
    assert dispatch.request is not None
    return kernel.commit_move(dispatch.request, executor)
