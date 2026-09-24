#!/usr/bin/env python3
"""Bounded one-page authored direct-object multi-selection V1."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from object_selection_compose_v1 import compose_object_selection_v1
from object_selection_target_v1 import DirectNodeSelectionV1


class EditorMultiSelectError(ValueError):
    pass


@dataclass(frozen=True)
class AuthoredSelectableNodeV1:
    page_id: str
    node_id: str
    authored_direct: bool
    x_emu: int
    y_emu: int
    width_emu: int
    height_emu: int

    def __post_init__(self) -> None:
        if not isinstance(self.page_id,str) or not self.page_id:
            raise EditorMultiSelectError("page_id is required")
        if not isinstance(self.node_id,str) or not self.node_id:
            raise EditorMultiSelectError("node_id is required")
        if not isinstance(self.authored_direct,bool):
            raise EditorMultiSelectError("authored_direct must be boolean")
        for name in ("x_emu","y_emu","width_emu","height_emu"):
            value=getattr(self,name)
            if not isinstance(value,int) or isinstance(value,bool):
                raise EditorMultiSelectError(f"{name} must be integer")
        if self.width_emu < 0 or self.height_emu < 0:
            raise EditorMultiSelectError("bounds extent must be non-negative")


@dataclass(frozen=True)
class AuthoredMultiSelectionStateV1:
    protocol_version: Literal["chaptera.authored-multi-selection.v1"]
    page_id: str
    selected_node_ids: tuple[str,...]
    primary_node_id: str | None

    def __post_init__(self) -> None:
        if not isinstance(self.page_id,str) or not self.page_id:
            raise EditorMultiSelectError("page_id is required")
        if tuple(sorted(set(self.selected_node_ids))) != self.selected_node_ids:
            raise EditorMultiSelectError("selected_node_ids must be sorted and duplicate-free")
        if self.primary_node_id is not None and self.primary_node_id not in self.selected_node_ids:
            raise EditorMultiSelectError("primary_node_id must be selected")


@dataclass(frozen=True)
class AggregateSelectionBoundsV1:
    x_emu: int
    y_emu: int
    width_emu: int
    height_emu: int


@dataclass(frozen=True)
class MultiSelectionInspectorV1:
    count: int
    primary_node_id: str | None


def empty_multi_selection_v1(*,page_id:str) -> AuthoredMultiSelectionStateV1:
    return AuthoredMultiSelectionStateV1(
        protocol_version="chaptera.authored-multi-selection.v1",
        page_id=page_id,
        selected_node_ids=(),
        primary_node_id=None,
    )


def _require_candidate(
    state: AuthoredMultiSelectionStateV1,
    candidate: AuthoredSelectableNodeV1,
) -> None:
    if candidate.page_id != state.page_id:
        raise EditorMultiSelectError("cross-page selection is not admitted")
    if not candidate.authored_direct:
        raise EditorMultiSelectError(
            "source-backed/inherited/projected instance is not admitted to authored NodeId multi-set"
        )


def _compose(
    state: AuthoredMultiSelectionStateV1,
    candidate: AuthoredSelectableNodeV1,
    *,
    mode: Literal["replace","toggle"],
) -> AuthoredMultiSelectionStateV1:
    _require_candidate(state,candidate)
    base=tuple(DirectNodeSelectionV1(state.page_id,n) for n in state.selected_node_ids)
    primary=(
        None
        if state.primary_node_id is None
        else DirectNodeSelectionV1(state.page_id,state.primary_node_id)
    )
    result=compose_object_selection_v1(
        base_set=base,
        base_primary=primary,
        candidate_set=(DirectNodeSelectionV1(candidate.page_id,candidate.node_id),),
        mode=mode,
    )
    ids=tuple(sorted(t.node_id for t in result.selected_set))
    primary_id=None if result.primary is None else result.primary.node_id
    return AuthoredMultiSelectionStateV1(
        protocol_version=state.protocol_version,
        page_id=state.page_id,
        selected_node_ids=ids,
        primary_node_id=primary_id,
    )


def click_authored_node_v1(
    state: AuthoredMultiSelectionStateV1,
    *,
    candidate: AuthoredSelectableNodeV1,
) -> AuthoredMultiSelectionStateV1:
    return _compose(state,candidate,mode="replace")


def shift_click_authored_node_v1(
    state: AuthoredMultiSelectionStateV1,
    *,
    candidate: AuthoredSelectableNodeV1,
) -> AuthoredMultiSelectionStateV1:
    return _compose(state,candidate,mode="toggle")


def click_empty_canvas_v1(
    state: AuthoredMultiSelectionStateV1,
) -> AuthoredMultiSelectionStateV1:
    return empty_multi_selection_v1(page_id=state.page_id)


def change_selection_page_v1(
    state: AuthoredMultiSelectionStateV1,
    *,
    page_id: str,
) -> AuthoredMultiSelectionStateV1:
    if not isinstance(page_id,str) or not page_id:
        raise EditorMultiSelectError("page_id is required")
    return empty_multi_selection_v1(page_id=page_id)


def aggregate_selection_bounds_v1(
    *,
    state: AuthoredMultiSelectionStateV1,
    candidates: tuple[AuthoredSelectableNodeV1,...],
) -> AggregateSelectionBoundsV1 | None:
    by_id={}
    for item in candidates:
        if item.page_id != state.page_id or not item.authored_direct:
            continue
        by_id[item.node_id]=item
    if not state.selected_node_ids:
        return None
    missing=[node_id for node_id in state.selected_node_ids if node_id not in by_id]
    if missing:
        raise EditorMultiSelectError("selection geometry missing for selected authored node")
    items=[by_id[node_id] for node_id in state.selected_node_ids]
    x0=min(i.x_emu for i in items)
    y0=min(i.y_emu for i in items)
    x1=max(i.x_emu+i.width_emu for i in items)
    y1=max(i.y_emu+i.height_emu for i in items)
    return AggregateSelectionBoundsV1(x0,y0,x1-x0,y1-y0)


def inspector_selection_summary_v1(
    state: AuthoredMultiSelectionStateV1,
) -> MultiSelectionInspectorV1:
    return MultiSelectionInspectorV1(
        count=len(state.selected_node_ids),
        primary_node_id=state.primary_node_id,
    )
