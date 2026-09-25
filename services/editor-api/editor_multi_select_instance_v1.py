#!/usr/bin/env python3
"""Top-level instance-aware transient multi-selection V1."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from object_selection_compose_v1 import compose_object_selection_v1
from object_selection_target_v1 import (
    DirectNodeSelectionV1,
    GroupMemberSelectionV1,
    ObjectSelectionTargetV1,
    ProjectedInstanceSelectionV1,
    selection_target_to_json_v1,
    target_page_id_v1,
)


class EditorMultiSelectInstanceError(ValueError):
    pass


TopLevelInstanceTargetV1 = DirectNodeSelectionV1 | ProjectedInstanceSelectionV1


@dataclass(frozen=True)
class InstanceAwareMultiSelectionV1:
    protocol_version: Literal["chaptera.instance-aware-multi-selection.v1"]
    page_id: str
    selected: tuple[TopLevelInstanceTargetV1, ...]
    primary: TopLevelInstanceTargetV1 | None

    def __post_init__(self) -> None:
        if not isinstance(self.page_id, str) or not self.page_id:
            raise EditorMultiSelectInstanceError("page_id is required")
        if len(set(self.selected)) != len(self.selected):
            raise EditorMultiSelectInstanceError("selected targets must be duplicate-free")
        ordered=tuple(sorted(self.selected,key=selection_target_to_json_v1))
        if ordered != self.selected:
            raise EditorMultiSelectInstanceError("selected targets must use deterministic canonical order")
        for target in self.selected:
            if isinstance(target, GroupMemberSelectionV1):
                raise EditorMultiSelectInstanceError("GroupMember belongs to nested selection scope")
            if not isinstance(target,(DirectNodeSelectionV1,ProjectedInstanceSelectionV1)):
                raise EditorMultiSelectInstanceError("unsupported top-level target variant")
            if target_page_id_v1(target) != self.page_id:
                raise EditorMultiSelectInstanceError("cross-page top-level selection is not admitted")
        if self.primary is not None and self.primary not in self.selected:
            raise EditorMultiSelectInstanceError("primary must be selected")


def empty_instance_selection_v1(*,page_id:str) -> InstanceAwareMultiSelectionV1:
    return InstanceAwareMultiSelectionV1(
        protocol_version="chaptera.instance-aware-multi-selection.v1",
        page_id=page_id,
        selected=(),
        primary=None,
    )


def compose_instance_selection_v1(
    state: InstanceAwareMultiSelectionV1,
    *,
    candidate: ObjectSelectionTargetV1,
    mode: Literal["replace","toggle"],
) -> InstanceAwareMultiSelectionV1:
    if isinstance(candidate,GroupMemberSelectionV1):
        raise EditorMultiSelectInstanceError("GroupMember selection is owned by nested scope")
    if not isinstance(candidate,(DirectNodeSelectionV1,ProjectedInstanceSelectionV1)):
        raise EditorMultiSelectInstanceError("unsupported top-level target")
    if target_page_id_v1(candidate) != state.page_id:
        raise EditorMultiSelectInstanceError("cross-page top-level selection is not admitted")
    result=compose_object_selection_v1(
        base_set=state.selected,
        base_primary=state.primary,
        candidate_set=(candidate,),
        mode=mode,
    )
    return InstanceAwareMultiSelectionV1(
        protocol_version=state.protocol_version,
        page_id=state.page_id,
        selected=tuple(result.selected_set),
        primary=result.primary,
    )


def click_instance_target_v1(
    state: InstanceAwareMultiSelectionV1,
    *,
    candidate: ObjectSelectionTargetV1,
) -> InstanceAwareMultiSelectionV1:
    return compose_instance_selection_v1(state,candidate=candidate,mode="replace")


def shift_click_instance_target_v1(
    state: InstanceAwareMultiSelectionV1,
    *,
    candidate: ObjectSelectionTargetV1,
) -> InstanceAwareMultiSelectionV1:
    return compose_instance_selection_v1(state,candidate=candidate,mode="toggle")


def change_instance_selection_page_v1(
    state: InstanceAwareMultiSelectionV1,
    *,
    page_id:str,
) -> InstanceAwareMultiSelectionV1:
    return empty_instance_selection_v1(page_id=page_id)


def selected_mutation_node_ids_v1(
    state: InstanceAwareMultiSelectionV1,
    *,
    operation_safe_direct_node_ids: frozenset[str],
) -> tuple[str,...]:
    """Fail closed unless every target is DirectNode and explicitly admitted by caller."""
    if not isinstance(operation_safe_direct_node_ids,frozenset):
        raise EditorMultiSelectInstanceError("operation_safe_direct_node_ids must be frozenset")
    node_ids=[]
    for target in state.selected:
        if not isinstance(target,DirectNodeSelectionV1):
            raise EditorMultiSelectInstanceError(
                "projected/inherited instance is inspectable but not admitted to page-local mutation"
            )
        if target.node_id not in operation_safe_direct_node_ids:
            raise EditorMultiSelectInstanceError(
                "direct target lacks explicit operation-safe admission"
            )
        node_ids.append(target.node_id)
    return tuple(sorted(node_ids))


def inspector_instance_selection_v1(state: InstanceAwareMultiSelectionV1) -> tuple[dict,...]:
    out=[]
    for target in state.selected:
        if isinstance(target,DirectNodeSelectionV1):
            out.append({
                "variant":"direct_node",
                "page_id":target.page_id,
                "node_id":target.node_id,
                "read_only":False,
            })
        else:
            out.append({
                "variant":"projected_instance",
                "page_id":target.page_id,
                "instance_id":target.instance_id,
                "origin_node_id":target.origin_node_id,
                "projection_kind":target.projection_kind,
                "mutation_class":target.mutation_class,
                "read_only":True,
            })
    return tuple(out)
