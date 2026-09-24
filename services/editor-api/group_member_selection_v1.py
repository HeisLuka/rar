#!/usr/bin/env python3
"""One-level transient authored Group-member selection scope V1.

Top-level selection and Group-member selection are mutually exclusive scopes.
A GroupMembers scope contains only direct children of one authored Group root.
The scope layer never promotes a member NodeId into top-level selection.

Graph provenance/capability admission is caller-owned. This module owns only
typed scope invariants, set composition, Escape/Clear/page-change transitions,
and explicit fail-closed reconciliation from a caller-supplied validity
receipt.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TypeAlias

from object_selection_compose_v1 import (
    ObjectSelectionComposeError,
    SelectionComposeModeV1,
    compose_object_selection_v1,
)
from object_selection_target_v1 import (
    DirectNodeSelectionV1,
    GroupMemberSelectionV1,
    ObjectSelectionTargetError,
    ObjectSelectionTargetV1,
    target_variant_v1,
)


class GroupMemberSelectionError(ValueError):
    pass


@dataclass(frozen=True)
class TopLevelSelectionScopeV1:
    selected: tuple[ObjectSelectionTargetV1, ...]
    primary: ObjectSelectionTargetV1 | None
    scope: str = "top_level"

    def __post_init__(self) -> None:
        _validate_top_level(self.selected, self.primary)


@dataclass(frozen=True)
class GroupMembersSelectionScopeV1:
    root_group: DirectNodeSelectionV1
    selected_members: tuple[GroupMemberSelectionV1, ...]
    primary: GroupMemberSelectionV1 | None
    scope: str = "group_members"

    def __post_init__(self) -> None:
        _validate_group_members(
            self.root_group,
            self.selected_members,
            self.primary,
        )


ObjectSelectionScopeV1: TypeAlias = (
    TopLevelSelectionScopeV1 | GroupMembersSelectionScopeV1
)


@dataclass(frozen=True)
class GroupScopeReconcileReceiptV1:
    root_survives: bool
    membership_valid: bool


def _fail(message: str) -> None:
    raise GroupMemberSelectionError(message)


def _validate_top_level(
    selected: tuple[ObjectSelectionTargetV1, ...],
    primary: ObjectSelectionTargetV1 | None,
) -> None:
    if not isinstance(selected, tuple):
        _fail("TopLevel selected must be a tuple")
    for index, target in enumerate(selected):
        try:
            target_variant_v1(target)
        except ObjectSelectionTargetError as exc:
            raise GroupMemberSelectionError(
                f"TopLevel selected[{index}] is invalid: {exc}"
            ) from exc
    if len(set(selected)) != len(selected):
        _fail("TopLevel selected must be duplicate-free")
    if primary is not None:
        try:
            target_variant_v1(primary)
        except ObjectSelectionTargetError as exc:
            raise GroupMemberSelectionError(f"TopLevel primary is invalid: {exc}") from exc
        if primary not in set(selected):
            _fail("TopLevel primary must be selected")


def _validate_group_members(
    root_group: DirectNodeSelectionV1,
    selected_members: tuple[GroupMemberSelectionV1, ...],
    primary: GroupMemberSelectionV1 | None,
) -> None:
    if not isinstance(root_group, DirectNodeSelectionV1):
        _fail("GroupMembers root_group must be a DirectNode target")
    if not isinstance(selected_members, tuple):
        _fail("GroupMembers selected_members must be a tuple")

    root_id = root_group.node_id
    page_id = root_group.page_id
    for index, member in enumerate(selected_members):
        if not isinstance(member, GroupMemberSelectionV1):
            _fail(f"GroupMembers selected_members[{index}] must be GroupMember")
        if member.page_id != page_id:
            _fail("GroupMembers members must share root page")
        if member.root_group_id != root_id:
            _fail("GroupMembers members must reference the same root Group")

    if len(set(selected_members)) != len(selected_members):
        _fail("GroupMembers selected_members must be duplicate-free")

    if primary is not None:
        if not isinstance(primary, GroupMemberSelectionV1):
            _fail("GroupMembers primary must be GroupMember")
        if primary not in set(selected_members):
            _fail("GroupMembers primary must be selected")
        if primary.page_id != page_id or primary.root_group_id != root_id:
            _fail("GroupMembers primary must belong to the same root Group")


def empty_top_level_scope_v1() -> TopLevelSelectionScopeV1:
    return TopLevelSelectionScopeV1(selected=(), primary=None)


def enter_group_member_v1(
    *,
    root_group: DirectNodeSelectionV1,
    member: GroupMemberSelectionV1,
) -> GroupMembersSelectionScopeV1:
    if member.page_id != root_group.page_id:
        _fail("member page must match root Group page")
    if member.root_group_id != root_group.node_id:
        _fail("member must reference the entered root Group")
    return GroupMembersSelectionScopeV1(
        root_group=root_group,
        selected_members=(member,),
        primary=member,
    )


def compose_group_members_v1(
    *,
    scope: GroupMembersSelectionScopeV1,
    candidates: tuple[GroupMemberSelectionV1, ...],
    mode: SelectionComposeModeV1,
) -> GroupMembersSelectionScopeV1:
    if not isinstance(scope, GroupMembersSelectionScopeV1):
        _fail("compose_group_members_v1 requires GroupMembers scope")

    for member in candidates:
        if not isinstance(member, GroupMemberSelectionV1):
            _fail("GroupMembers candidates must all be GroupMember targets")
        if member.page_id != scope.root_group.page_id:
            _fail("GroupMembers candidates must share root page")
        if member.root_group_id != scope.root_group.node_id:
            _fail("GroupMembers candidates must reference the same root Group")

    try:
        result = compose_object_selection_v1(
            base_set=scope.selected_members,
            base_primary=scope.primary,
            candidate_set=candidates,
            mode=mode,
        )
    except ObjectSelectionComposeError as exc:
        raise GroupMemberSelectionError(str(exc)) from exc

    if any(not isinstance(target, GroupMemberSelectionV1) for target in result.selected_set):
        _fail("selection compose returned a non-GroupMember identity")
    if result.primary is not None and not isinstance(
        result.primary, GroupMemberSelectionV1
    ):
        _fail("selection compose returned a non-GroupMember primary")

    return GroupMembersSelectionScopeV1(
        root_group=scope.root_group,
        selected_members=tuple(result.selected_set),
        primary=result.primary,
    )


def escape_parent_v1(
    scope: GroupMembersSelectionScopeV1,
) -> TopLevelSelectionScopeV1:
    if not isinstance(scope, GroupMembersSelectionScopeV1):
        _fail("EscapeParent V1 requires GroupMembers scope")
    return TopLevelSelectionScopeV1(
        selected=(scope.root_group,),
        primary=scope.root_group,
    )


def clear_selection_scope_v1(
    scope: ObjectSelectionScopeV1,
) -> TopLevelSelectionScopeV1:
    if not isinstance(
        scope,
        (TopLevelSelectionScopeV1, GroupMembersSelectionScopeV1),
    ):
        _fail("unsupported selection scope")
    return empty_top_level_scope_v1()


def selection_scope_on_page_change_v1(
    scope: ObjectSelectionScopeV1,
) -> TopLevelSelectionScopeV1:
    return clear_selection_scope_v1(scope)


def reconcile_group_scope_v1(
    *,
    scope: GroupMembersSelectionScopeV1,
    receipt: GroupScopeReconcileReceiptV1,
) -> ObjectSelectionScopeV1:
    """Fail closed from explicit caller truth; never promote member NodeIds."""
    if not isinstance(scope, GroupMembersSelectionScopeV1):
        _fail("reconcile_group_scope_v1 requires GroupMembers scope")
    if not isinstance(receipt, GroupScopeReconcileReceiptV1):
        _fail("reconciliation requires GroupScopeReconcileReceiptV1")
    if receipt.membership_valid:
        if not receipt.root_survives:
            _fail("membership cannot remain valid when root does not survive")
        return scope
    if receipt.root_survives:
        return TopLevelSelectionScopeV1(
            selected=(scope.root_group,),
            primary=scope.root_group,
        )
    return empty_top_level_scope_v1()
