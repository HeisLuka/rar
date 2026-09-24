#!/usr/bin/env python3
"""Final top-level canvas Escape selection fallback V1."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from editor_tool_routing_v1 import EditorToolRoutingResultV1
from group_member_selection_v1 import (
    TopLevelSelectionScopeV1,
    empty_top_level_scope_v1,
)


class CanvasEscapeSelectionError(ValueError):
    pass


@dataclass(frozen=True)
class CanvasEscapeSelectionResultV1:
    protocol_version: Literal["chaptera.canvas-escape-selection-result.v1"]
    status: Literal["cleared","not_applicable"]
    page_id: str
    selection: TopLevelSelectionScopeV1
    focus_owner_unchanged: Literal[True]
    tool_state_unchanged: Literal[True]
    viewport_unchanged: Literal[True]
    document_mutation_count: Literal[0]
    revision_created: Literal[False]


def apply_canvas_escape_selection_v1(
    *,
    routing: EditorToolRoutingResultV1,
    selection: TopLevelSelectionScopeV1,
) -> CanvasEscapeSelectionResultV1:
    if not isinstance(routing,EditorToolRoutingResultV1):
        raise CanvasEscapeSelectionError("EditorToolRoutingResultV1 is required")
    if not isinstance(selection,TopLevelSelectionScopeV1):
        raise CanvasEscapeSelectionError("top-level selection scope is required")

    if routing.action != "clear_top_level_selection":
        return CanvasEscapeSelectionResultV1(
            protocol_version="chaptera.canvas-escape-selection-result.v1",
            status="not_applicable",
            page_id=(
                selection.primary.page_id
                if selection.primary is not None
                else selection.selected[0].page_id
                if selection.selected
                else ""
            ),
            selection=selection,
            focus_owner_unchanged=True,
            tool_state_unchanged=True,
            viewport_unchanged=True,
            document_mutation_count=0,
            revision_created=False,
        )
    if not routing.selection_clear_requested:
        raise CanvasEscapeSelectionError(
            "routing clear action must carry selection_clear_requested"
        )
    if not selection.selected:
        raise CanvasEscapeSelectionError(
            "clear_top_level_selection requires nonempty top-level selection"
        )

    page_ids={target.page_id for target in selection.selected}
    if len(page_ids)!=1:
        raise CanvasEscapeSelectionError("top-level Escape selection must be one-page")
    page_id=next(iter(page_ids))

    return CanvasEscapeSelectionResultV1(
        protocol_version="chaptera.canvas-escape-selection-result.v1",
        status="cleared",
        page_id=page_id,
        selection=empty_top_level_scope_v1(),
        focus_owner_unchanged=True,
        tool_state_unchanged=True,
        viewport_unchanged=True,
        document_mutation_count=0,
        revision_created=False,
    )
