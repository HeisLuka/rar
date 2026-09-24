#!/usr/bin/env python3
"""Transient typed selection identity vocabulary V1.

The three V1 variants deliberately remain distinct identities even when they
mention the same canonical NodeId. There is intentionally no generic NodeId
collapse helper: mutation consumers must pattern-match the variant and prove
operation-specific admission independently.

These values are session/transient interaction state only. This module has no
EditorProject serialization or persistence hook.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from typing import Literal, TypeAlias


SelectionTargetVariantV1 = Literal[
    "direct_node",
    "projected_instance",
    "group_member",
]


class ObjectSelectionTargetError(ValueError):
    pass


def _identity(value: str, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise ObjectSelectionTargetError(f"{label} is required")
    return value


@dataclass(frozen=True)
class DirectNodeSelectionV1:
    page_id: str
    node_id: str
    variant: Literal["direct_node"] = "direct_node"

    def __post_init__(self) -> None:
        _identity(self.page_id, "page_id")
        _identity(self.node_id, "node_id")


@dataclass(frozen=True)
class ProjectedInstanceSelectionV1:
    page_id: str
    instance_id: str
    origin_node_id: str
    projection_kind: str
    mutation_class: str
    variant: Literal["projected_instance"] = "projected_instance"

    def __post_init__(self) -> None:
        _identity(self.page_id, "page_id")
        _identity(self.instance_id, "instance_id")
        _identity(self.origin_node_id, "origin_node_id")
        _identity(self.projection_kind, "projection_kind")
        _identity(self.mutation_class, "mutation_class")


@dataclass(frozen=True)
class GroupMemberSelectionV1:
    page_id: str
    root_group_id: str
    member_node_id: str
    variant: Literal["group_member"] = "group_member"

    def __post_init__(self) -> None:
        _identity(self.page_id, "page_id")
        _identity(self.root_group_id, "root_group_id")
        _identity(self.member_node_id, "member_node_id")
        if self.root_group_id == self.member_node_id:
            raise ObjectSelectionTargetError(
                "root_group_id must differ from member_node_id"
            )


ObjectSelectionTargetV1: TypeAlias = (
    DirectNodeSelectionV1
    | ProjectedInstanceSelectionV1
    | GroupMemberSelectionV1
)


def target_variant_v1(target: ObjectSelectionTargetV1) -> SelectionTargetVariantV1:
    if isinstance(target, DirectNodeSelectionV1):
        return "direct_node"
    if isinstance(target, ProjectedInstanceSelectionV1):
        return "projected_instance"
    if isinstance(target, GroupMemberSelectionV1):
        return "group_member"
    raise ObjectSelectionTargetError("unsupported ObjectSelectionTargetV1")


def target_page_id_v1(target: ObjectSelectionTargetV1) -> str:
    target_variant_v1(target)
    return target.page_id


def target_provenance_v1(target: ObjectSelectionTargetV1) -> dict:
    """Expose explicit identity provenance without collapsing to NodeId."""
    if isinstance(target, DirectNodeSelectionV1):
        return {
            "variant": "direct_node",
            "page_id": target.page_id,
            "node_id": target.node_id,
        }
    if isinstance(target, ProjectedInstanceSelectionV1):
        return {
            "variant": "projected_instance",
            "page_id": target.page_id,
            "instance_id": target.instance_id,
            "origin_node_id": target.origin_node_id,
            "projection_kind": target.projection_kind,
            "mutation_class": target.mutation_class,
        }
    if isinstance(target, GroupMemberSelectionV1):
        return {
            "variant": "group_member",
            "page_id": target.page_id,
            "root_group_id": target.root_group_id,
            "member_node_id": target.member_node_id,
        }
    raise ObjectSelectionTargetError("unsupported ObjectSelectionTargetV1")


def selection_target_to_dict_v1(target: ObjectSelectionTargetV1) -> dict:
    provenance = target_provenance_v1(target)
    return {"protocol_version": "chaptera.object-selection-target.v1", **provenance}


def selection_target_to_json_v1(target: ObjectSelectionTargetV1) -> str:
    return json.dumps(
        selection_target_to_dict_v1(target),
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )


def selection_target_fingerprint_v1(target: ObjectSelectionTargetV1) -> str:
    return hashlib.sha256(
        selection_target_to_json_v1(target).encode("utf-8")
    ).hexdigest()


def selection_target_from_dict_v1(payload: dict) -> ObjectSelectionTargetV1:
    if not isinstance(payload, dict):
        raise ObjectSelectionTargetError("selection target payload must be an object")
    if payload.get("protocol_version") != "chaptera.object-selection-target.v1":
        raise ObjectSelectionTargetError("V1 protocol_version is required")

    variant = payload.get("variant")
    if variant == "direct_node":
        expected = {"protocol_version", "variant", "page_id", "node_id"}
        if set(payload) != expected:
            raise ObjectSelectionTargetError("DirectNode V1 contains unexpected fields")
        return DirectNodeSelectionV1(
            page_id=payload.get("page_id"),
            node_id=payload.get("node_id"),
        )

    if variant == "projected_instance":
        expected = {
            "protocol_version",
            "variant",
            "page_id",
            "instance_id",
            "origin_node_id",
            "projection_kind",
            "mutation_class",
        }
        if set(payload) != expected:
            raise ObjectSelectionTargetError(
                "ProjectedInstance V1 contains unexpected fields"
            )
        return ProjectedInstanceSelectionV1(
            page_id=payload.get("page_id"),
            instance_id=payload.get("instance_id"),
            origin_node_id=payload.get("origin_node_id"),
            projection_kind=payload.get("projection_kind"),
            mutation_class=payload.get("mutation_class"),
        )

    if variant == "group_member":
        expected = {
            "protocol_version",
            "variant",
            "page_id",
            "root_group_id",
            "member_node_id",
        }
        if set(payload) != expected:
            raise ObjectSelectionTargetError(
                "GroupMember V1 contains unexpected fields"
            )
        return GroupMemberSelectionV1(
            page_id=payload.get("page_id"),
            root_group_id=payload.get("root_group_id"),
            member_node_id=payload.get("member_node_id"),
        )

    raise ObjectSelectionTargetError("unknown ObjectSelectionTargetV1 variant")
