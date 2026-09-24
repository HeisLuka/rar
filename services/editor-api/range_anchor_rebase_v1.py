#!/usr/bin/env python3
"""Deterministic Story anchored-range rebasing V1.

One committed ReplaceStoryRange changes canonical scalar coordinates exactly
once. Persistent semantic span owners consume this transform with explicit
boundary affinity and explicit full-cover persistence policy instead of doing
feature-local offset arithmetic.

Boundary affinity:
- left  => an anchor exactly at an edit boundary stays before replacement text.
- right => an anchor exactly at an edit boundary stays after replacement text.

For a fully covered span, the owner must choose one explicit policy:
- replacement: span becomes the replacement range;
- collapse_left / collapse_right: keep an empty anchor at replacement start/end;
- invalidate: semantic owner survives elsewhere but this range is invalid;
- delete: semantic owner/range is deleted.

No DOM/UTF-16 authority, OT/CRDT stale-command rebase, or native PUB write.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal


AnchorAffinityV1 = Literal["left", "right"]
FullCoverPolicyV1 = Literal[
    "replacement",
    "collapse_left",
    "collapse_right",
    "invalidate",
    "delete",
]
RangeRebaseStatusV1 = Literal["survives", "invalidated", "deleted"]


class RangeAnchorRebaseError(ValueError):
    pass


@dataclass(frozen=True)
class AnchoredRangeV1:
    start_scalar: int
    end_scalar: int
    allow_empty: bool = False


@dataclass(frozen=True)
class RangeAnchorPolicyV1:
    start_affinity: AnchorAffinityV1
    end_affinity: AnchorAffinityV1
    full_cover_policy: FullCoverPolicyV1


@dataclass(frozen=True)
class StoryRangeEditV1:
    start_scalar: int
    end_scalar: int
    replacement_length: int

    @property
    def removed_length(self) -> int:
        return self.end_scalar - self.start_scalar

    @property
    def delta(self) -> int:
        return self.replacement_length - self.removed_length

    @property
    def replacement_end_scalar(self) -> int:
        return self.start_scalar + self.replacement_length


@dataclass(frozen=True)
class RangeRebaseResultV1:
    protocol_version: Literal["chaptera.range-anchor-rebase.v1"]
    status: RangeRebaseStatusV1
    range: AnchoredRangeV1 | None


@dataclass(frozen=True)
class RangeRebaseReceiptV1:
    protocol_version: Literal["chaptera.range-anchor-rebase-receipt.v1"]
    before: AnchoredRangeV1
    policy: RangeAnchorPolicyV1
    edit: StoryRangeEditV1
    result: RangeRebaseResultV1


def _fail(message: str) -> None:
    raise RangeAnchorRebaseError(message)


def _validate_range(value: AnchoredRangeV1) -> None:
    if not isinstance(value, AnchoredRangeV1):
        _fail("range must be AnchoredRangeV1")
    for field in ("start_scalar", "end_scalar"):
        point = getattr(value, field)
        if not isinstance(point, int) or isinstance(point, bool) or point < 0:
            _fail(f"{field} must be a non-negative scalar index")
    if value.end_scalar < value.start_scalar:
        _fail("range end must not precede start")
    if value.start_scalar == value.end_scalar and not value.allow_empty:
        _fail("empty anchored range is not admitted by this owner")


def _validate_policy(policy: RangeAnchorPolicyV1, anchored: AnchoredRangeV1) -> None:
    if not isinstance(policy, RangeAnchorPolicyV1):
        _fail("policy must be RangeAnchorPolicyV1")
    if policy.start_affinity not in {"left", "right"}:
        _fail("start affinity must be left or right")
    if policy.end_affinity not in {"left", "right"}:
        _fail("end affinity must be left or right")
    if policy.full_cover_policy not in {
        "replacement",
        "collapse_left",
        "collapse_right",
        "invalidate",
        "delete",
    }:
        _fail("unsupported full-cover policy")
    if (
        anchored.start_scalar == anchored.end_scalar
        and policy.start_affinity != policy.end_affinity
    ):
        _fail("empty anchored range requires identical start/end affinity")


def _validate_edit(edit: StoryRangeEditV1) -> None:
    if not isinstance(edit, StoryRangeEditV1):
        _fail("edit must be StoryRangeEditV1")
    for field in ("start_scalar", "end_scalar", "replacement_length"):
        value = getattr(edit, field)
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            _fail(f"{field} must be a non-negative integer")
    if edit.end_scalar < edit.start_scalar:
        _fail("edit end must not precede start")


def _map_anchor(
    *,
    point: int,
    affinity: AnchorAffinityV1,
    edit: StoryRangeEditV1,
) -> int:
    a = edit.start_scalar
    b = edit.end_scalar
    r = edit.replacement_end_scalar

    if point < a:
        return point
    if point > b:
        return point + edit.delta

    # Insertion: a == b. An anchor at the insertion boundary chooses the side.
    if a == b and point == a:
        return a if affinity == "left" else r

    # Replace/delete boundaries and interior all choose which side of the
    # replacement owns the surviving anchor.
    return a if affinity == "left" else r


def _fully_covered(anchored: AnchoredRangeV1, edit: StoryRangeEditV1) -> bool:
    # Empty insertion covers no non-empty old content. For an empty anchored
    # range exactly at insertion, boundary affinity governs instead.
    if edit.start_scalar == edit.end_scalar:
        return False
    return (
        edit.start_scalar <= anchored.start_scalar
        and anchored.end_scalar <= edit.end_scalar
    )


def _full_cover_result(
    *,
    anchored: AnchoredRangeV1,
    policy: RangeAnchorPolicyV1,
    edit: StoryRangeEditV1,
) -> RangeRebaseResultV1:
    mode = policy.full_cover_policy
    if mode == "invalidate":
        return RangeRebaseResultV1(
            protocol_version="chaptera.range-anchor-rebase.v1",
            status="invalidated",
            range=None,
        )
    if mode == "delete":
        return RangeRebaseResultV1(
            protocol_version="chaptera.range-anchor-rebase.v1",
            status="deleted",
            range=None,
        )
    if mode == "replacement":
        start = edit.start_scalar
        end = edit.replacement_end_scalar
    elif mode == "collapse_left":
        start = end = edit.start_scalar
    elif mode == "collapse_right":
        start = end = edit.replacement_end_scalar
    else:
        _fail("unsupported full-cover policy")

    allow_empty = anchored.allow_empty or start != end
    if start == end and not allow_empty:
        # Explicit collapse/replacement-to-empty is only valid when the owner
        # admits an empty surviving range. Otherwise deletion is the only
        # semantically safe result.
        return RangeRebaseResultV1(
            protocol_version="chaptera.range-anchor-rebase.v1",
            status="deleted",
            range=None,
        )
    return RangeRebaseResultV1(
        protocol_version="chaptera.range-anchor-rebase.v1",
        status="survives",
        range=AnchoredRangeV1(start, end, allow_empty=allow_empty),
    )


def rebase_anchored_range_v1(
    *,
    anchored: AnchoredRangeV1,
    policy: RangeAnchorPolicyV1,
    edit: StoryRangeEditV1,
) -> RangeRebaseReceiptV1:
    _validate_range(anchored)
    _validate_policy(policy, anchored)
    _validate_edit(edit)

    if _fully_covered(anchored, edit):
        result = _full_cover_result(
            anchored=anchored,
            policy=policy,
            edit=edit,
        )
        return RangeRebaseReceiptV1(
            protocol_version="chaptera.range-anchor-rebase-receipt.v1",
            before=anchored,
            policy=policy,
            edit=edit,
            result=result,
        )

    start = _map_anchor(
        point=anchored.start_scalar,
        affinity=policy.start_affinity,
        edit=edit,
    )
    end = _map_anchor(
        point=anchored.end_scalar,
        affinity=policy.end_affinity,
        edit=edit,
    )
    if end < start:
        _fail("anchor affinities produced inverted surviving range")
    if start == end and not anchored.allow_empty:
        result = RangeRebaseResultV1(
            protocol_version="chaptera.range-anchor-rebase.v1",
            status="deleted",
            range=None,
        )
    else:
        result = RangeRebaseResultV1(
            protocol_version="chaptera.range-anchor-rebase.v1",
            status="survives",
            range=AnchoredRangeV1(
                start,
                end,
                allow_empty=anchored.allow_empty,
            ),
        )
    return RangeRebaseReceiptV1(
        protocol_version="chaptera.range-anchor-rebase-receipt.v1",
        before=anchored,
        policy=policy,
        edit=edit,
        result=result,
    )


def rebase_anchored_ranges_v1(
    *,
    anchored_ranges: tuple[tuple[str, AnchoredRangeV1, RangeAnchorPolicyV1], ...],
    edit: StoryRangeEditV1,
) -> tuple[tuple[str, RangeRebaseReceiptV1], ...]:
    if not isinstance(anchored_ranges, tuple):
        _fail("anchored_ranges must be an ordered tuple")
    ids = [item[0] for item in anchored_ranges if isinstance(item, tuple) and len(item) == 3]
    if len(ids) != len(anchored_ranges) or any(not isinstance(value, str) or not value for value in ids):
        _fail("each anchored range must have a non-empty semantic id")
    if len(set(ids)) != len(ids):
        _fail("semantic ids must be unique")
    receipts = []
    for semantic_id, anchored, policy in anchored_ranges:
        receipts.append(
            (
                semantic_id,
                rebase_anchored_range_v1(
                    anchored=anchored,
                    policy=policy,
                    edit=edit,
                ),
            )
        )
    return tuple(receipts)


def restore_range_from_rebase_receipt_v1(
    receipt: RangeRebaseReceiptV1,
) -> AnchoredRangeV1:
    """Exact undo when the owning Story operation persisted this before-state."""
    if not isinstance(receipt, RangeRebaseReceiptV1):
        _fail("RangeRebaseReceiptV1 is required")
    _validate_range(receipt.before)
    return receipt.before
