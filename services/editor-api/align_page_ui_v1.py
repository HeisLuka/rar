#!/usr/bin/env python3
"""Single-object RelativeToPage align adapter over AlignDistributePagePlanV1."""

from __future__ import annotations

from dataclasses import dataclass

from align_distribute_page_v1 import plan_align_distribute_page_v1
from align_distribute_plan_v1 import GeometryMemberV1, RectEmu


class AlignPageUiError(ValueError):
    pass


@dataclass(frozen=True)
class AlignPageEligibilityV1:
    node_id: str | None
    enabled: bool
    reason: str | None


@dataclass(frozen=True)
class AlignPageUiResultV1:
    status: str
    node_id: str | None
    after_bounds: dict | None = None
    move_request: dict | None = None


def _rect_from_dict(value: dict, label: str) -> RectEmu:
    if not isinstance(value, dict) or set(value) != {"x","y","width","height"}:
        raise AlignPageUiError(f"{label} must be exact RectEMU")
    for field in ("x","y","width","height"):
        item=value[field]
        if not isinstance(item,int) or isinstance(item,bool):
            raise AlignPageUiError(f"{label}.{field} must be integer")
    if value["width"] <= 0 or value["height"] <= 0:
        raise AlignPageUiError(f"{label} must have positive size")
    return RectEmu(value["x"],value["y"],value["width"],value["height"])


def classify_align_page_target_v1(shape: dict) -> AlignPageEligibilityV1:
    if not isinstance(shape,dict):
        return AlignPageEligibilityV1(None,False,"missing_selection")
    node_id=shape.get("node_id")
    if not isinstance(node_id,str) or not node_id:
        return AlignPageEligibilityV1(None,False,"missing_node_id")
    if shape.get("provenance") not in ({"kind":"author_created"}, None) and shape.get("author_created") is not True:
        return AlignPageEligibilityV1(node_id,False,"source_backed_or_projected")
    if shape.get("author_created") is False:
        return AlignPageEligibilityV1(node_id,False,"source_backed_or_projected")
    if shape.get("parent_id") != shape.get("page_id"):
        return AlignPageEligibilityV1(node_id,False,"not_direct_page_owned")
    transform=shape.get("transform",{"kind":"identity"})
    if transform != {"kind":"identity"}:
        return AlignPageEligibilityV1(node_id,False,"unsupported_transform")
    try:
        _rect_from_dict(shape.get("bounds"),"shape.bounds")
    except AlignPageUiError:
        return AlignPageEligibilityV1(node_id,False,"invalid_bounds")
    return AlignPageEligibilityV1(node_id,True,None)


def build_align_page_move_request_v1(
    *,
    shape: dict,
    page_bounds: dict,
    mode: str,
    document_id: str,
    source_hash: str,
    base_revision_id: str,
    client_operation_id: str,
) -> AlignPageUiResultV1:
    eligibility=classify_align_page_target_v1(shape)
    if not eligibility.enabled:
        return AlignPageUiResultV1(
            status="disabled",
            node_id=eligibility.node_id,
        )
    node_id=eligibility.node_id
    assert node_id is not None
    before=_rect_from_dict(shape["bounds"],"shape.bounds")
    page=_rect_from_dict(page_bounds,"page_bounds")

    plan=plan_align_distribute_page_v1(
        page_bounds=page,
        members=(GeometryMemberV1(node_id=node_id,bounds=before),),
        mode=mode,
    )
    item=plan.members[0]
    after=item.after
    after_dict={"x":after.x,"y":after.y,"width":after.width,"height":after.height}
    if plan.status=="no_change":
        return AlignPageUiResultV1(
            status="no_change",
            node_id=node_id,
            after_bounds=after_dict,
        )

    request={
        "protocol_version":"chaptera.move-node-intent.v1",
        "document_id":document_id,
        "source_hash":source_hash,
        "base_revision_id":base_revision_id,
        "client_operation_id":client_operation_id,
        "command":{
            "kind":"move_node_to",
            "node_id":node_id,
            "x_emu":after.x,
            "y_emu":after.y,
        },
    }
    return AlignPageUiResultV1(
        status="submit_move",
        node_id=node_id,
        after_bounds=after_dict,
        move_request=request,
    )
