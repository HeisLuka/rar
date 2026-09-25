#!/usr/bin/env python3
"""Scope-routed Select All for Story, top-level canvas, and nested Group scope V1."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from canvas_tool_state_v1 import SELECT_TOOL_V1
from editor_multi_select_v1 import AuthoredMultiSelectionStateV1, AuthoredSelectableNodeV1
from editor_object_select_all_v1 import (
    EditorObjectSelectAllResultV1,
    route_editor_select_all_v1,
)
from editor_tool_routing_v1 import EditorToolRoutingStateV1
from nested_group_selection_v1 import (
    NestedGroupPathSnapshotV1,
    NestedGroupSelectionError,
    NestedGroupSelectionScopeV1,
    NestedGroupSelectionTargetV1,
    compose_nested_group_selection_v1,
)
from story_edit_domain_v1 import StoryEditDomainV1
from text_selection_state_v1 import TextSelectionStateV1
from text_typing_format_state_v1 import TextTypingFormatStateV1


@dataclass(frozen=True)
class EditorNestedSelectAllResultV1:
    protocol_version: Literal["chaptera.editor-nested-select-all-result.v1"]
    status: Literal[
        "delegated",
        "nested_selected",
        "nested_cleared",
        "suppressed",
        "nested_scope_invalid",
    ]
    delegated_result: EditorObjectSelectAllResultV1 | None
    nested_scope: NestedGroupSelectionScopeV1 | None
    selected_direct_child_count: int
    document_mutation_count: Literal[0]
    revision_created: Literal[False]
    group_children_order_changed: Literal[False]
    scroll_changed: Literal[False]
    zoom_changed: Literal[False]
    reason: str | None


def _delegated(
    result: EditorObjectSelectAllResultV1,
    nested_scope: NestedGroupSelectionScopeV1 | None,
) -> EditorNestedSelectAllResultV1:
    return EditorNestedSelectAllResultV1(
        protocol_version="chaptera.editor-nested-select-all-result.v1",
        status="delegated",
        delegated_result=result,
        nested_scope=nested_scope,
        selected_direct_child_count=0,
        document_mutation_count=0,
        revision_created=False,
        group_children_order_changed=False,
        scroll_changed=False,
        zoom_changed=False,
        reason=None,
    )


def route_editor_nested_select_all_v1(
    *,
    routing_state: EditorToolRoutingStateV1,
    current_page_id: str,
    current_object_selection: AuthoredMultiSelectionStateV1,
    page_candidates: tuple[AuthoredSelectableNodeV1, ...],
    nested_scope: NestedGroupSelectionScopeV1 | None = None,
    nested_snapshot: NestedGroupPathSnapshotV1 | None = None,
    eligible_direct_child_ids: tuple[str, ...] = (),
    current_text_selection: TextSelectionStateV1 | None = None,
    text_domain: StoryEditDomainV1 | None = None,
    typing_state: TextTypingFormatStateV1 | None = None,
) -> EditorNestedSelectAllResultV1:
    """Route Ctrl/Cmd+A without crossing the active semantic scope."""

    # Story text always wins, even if a nested canvas scope remains transiently
    # remembered by the surrounding product shell.
    if routing_state.focus_owner == "story_text":
        return _delegated(
            route_editor_select_all_v1(
                routing_state=routing_state,
                current_page_id=current_page_id,
                current_object_selection=current_object_selection,
                candidates=page_candidates,
                current_text_selection=current_text_selection,
                text_domain=text_domain,
                typing_state=typing_state,
            ),
            nested_scope,
        )

    # Without an active nested path, reuse the already-defined top-level law.
    if nested_scope is None:
        return _delegated(
            route_editor_select_all_v1(
                routing_state=routing_state,
                current_page_id=current_page_id,
                current_object_selection=current_object_selection,
                candidates=page_candidates,
                current_text_selection=current_text_selection,
                text_domain=text_domain,
                typing_state=typing_state,
            ),
            None,
        )

    # Nested scope is an explicit fence: never fall back to page Select All.
    if nested_scope.page_id != current_page_id:
        return EditorNestedSelectAllResultV1(
            protocol_version="chaptera.editor-nested-select-all-result.v1",
            status="nested_scope_invalid",
            delegated_result=None,
            nested_scope=nested_scope,
            selected_direct_child_count=0,
            document_mutation_count=0,
            revision_created=False,
            group_children_order_changed=False,
            scroll_changed=False,
            zoom_changed=False,
            reason="nested scope belongs to a different current page",
        )

    if routing_state.focus_owner in {"modal", "ime", "inspector_input", "find_replace"}:
        return EditorNestedSelectAllResultV1(
            protocol_version="chaptera.editor-nested-select-all-result.v1",
            status="suppressed",
            delegated_result=None,
            nested_scope=nested_scope,
            selected_direct_child_count=0,
            document_mutation_count=0,
            revision_created=False,
            group_children_order_changed=False,
            scroll_changed=False,
            zoom_changed=False,
            reason=f"{routing_state.focus_owner} owns Select All input",
        )

    if routing_state.focus_owner != "canvas":
        return EditorNestedSelectAllResultV1(
            protocol_version="chaptera.editor-nested-select-all-result.v1",
            status="suppressed",
            delegated_result=None,
            nested_scope=nested_scope,
            selected_direct_child_count=0,
            document_mutation_count=0,
            revision_created=False,
            group_children_order_changed=False,
            scroll_changed=False,
            zoom_changed=False,
            reason="canvas does not own nested Select All input",
        )

    if routing_state.canvas.active_gesture is not None:
        return EditorNestedSelectAllResultV1(
            protocol_version="chaptera.editor-nested-select-all-result.v1",
            status="suppressed",
            delegated_result=None,
            nested_scope=nested_scope,
            selected_direct_child_count=0,
            document_mutation_count=0,
            revision_created=False,
            group_children_order_changed=False,
            scroll_changed=False,
            zoom_changed=False,
            reason="active canvas gesture owns keyboard routing",
        )

    if routing_state.canvas.active_tool != SELECT_TOOL_V1:
        return EditorNestedSelectAllResultV1(
            protocol_version="chaptera.editor-nested-select-all-result.v1",
            status="suppressed",
            delegated_result=None,
            nested_scope=nested_scope,
            selected_direct_child_count=0,
            document_mutation_count=0,
            revision_created=False,
            group_children_order_changed=False,
            scroll_changed=False,
            zoom_changed=False,
            reason="temporary canvas tool owns keyboard routing",
        )

    if nested_snapshot is None:
        return EditorNestedSelectAllResultV1(
            protocol_version="chaptera.editor-nested-select-all-result.v1",
            status="nested_scope_invalid",
            delegated_result=None,
            nested_scope=nested_scope,
            selected_direct_child_count=0,
            document_mutation_count=0,
            revision_created=False,
            group_children_order_changed=False,
            scroll_changed=False,
            zoom_changed=False,
            reason="nested path snapshot is required",
        )

    if len(set(eligible_direct_child_ids)) != len(eligible_direct_child_ids):
        return EditorNestedSelectAllResultV1(
            protocol_version="chaptera.editor-nested-select-all-result.v1",
            status="nested_scope_invalid",
            delegated_result=None,
            nested_scope=nested_scope,
            selected_direct_child_count=0,
            document_mutation_count=0,
            revision_created=False,
            group_children_order_changed=False,
            scroll_changed=False,
            zoom_changed=False,
            reason="eligible direct-child domain contains duplicates",
        )

    direct_children = set(nested_snapshot.edges[-1].children)
    if any(child_id not in direct_children for child_id in eligible_direct_child_ids):
        return EditorNestedSelectAllResultV1(
            protocol_version="chaptera.editor-nested-select-all-result.v1",
            status="nested_scope_invalid",
            delegated_result=None,
            nested_scope=nested_scope,
            selected_direct_child_count=0,
            document_mutation_count=0,
            revision_created=False,
            group_children_order_changed=False,
            scroll_changed=False,
            zoom_changed=False,
            reason="eligible child is not a current direct child of active Group",
        )

    candidates = tuple(
        NestedGroupSelectionTargetV1(
            page_id=nested_scope.page_id,
            group_path=nested_scope.container_path,
            node_id=child_id,
        )
        for child_id in sorted(eligible_direct_child_ids)
    )

    try:
        next_scope = compose_nested_group_selection_v1(
            scope=nested_scope,
            snapshot=nested_snapshot,
            candidates=candidates,
            mode="replace",
        )
    except NestedGroupSelectionError as exc:
        return EditorNestedSelectAllResultV1(
            protocol_version="chaptera.editor-nested-select-all-result.v1",
            status="nested_scope_invalid",
            delegated_result=None,
            nested_scope=nested_scope,
            selected_direct_child_count=0,
            document_mutation_count=0,
            revision_created=False,
            group_children_order_changed=False,
            scroll_changed=False,
            zoom_changed=False,
            reason=str(exc),
        )

    return EditorNestedSelectAllResultV1(
        protocol_version="chaptera.editor-nested-select-all-result.v1",
        status="nested_selected" if candidates else "nested_cleared",
        delegated_result=None,
        nested_scope=next_scope,
        selected_direct_child_count=len(candidates),
        document_mutation_count=0,
        revision_created=False,
        group_children_order_changed=False,
        scroll_changed=False,
        zoom_changed=False,
        reason=None,
    )
