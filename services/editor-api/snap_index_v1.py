#!/usr/bin/env python3
"""Minimal source-neutral SnapIndex candidate resolver V1.

This reconstructs only the reusable policy proven by historical yab#209:
- exact EMU candidate positions;
- page/object target vocabulary;
- caller-supplied tolerance;
- explicit peer exclusion;
- minimal absolute correction;
- stable deterministic tie-break:
  distance, target kind, target node id, target anchor, moving anchor.

It deliberately does not enumerate page objects, inspect scene carriers, mutate
MoveTransaction, paint feedback, or add guide candidates. Those belong to their
own consumers/producers.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal


MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU

SnapAxisV1 = Literal["x", "y"]
SnapAnchorKindV1 = Literal["min", "center", "max"]
SnapTargetKindV1 = Literal[
    "page_edge",
    "page_center",
    "object_edge",
    "object_center",
]

_TARGET_KIND_RANK = {
    "page_edge": 0,
    "page_center": 1,
    "object_edge": 2,
    "object_center": 3,
}
_ANCHOR_RANK = {"min": 0, "center": 1, "max": 2}


class SnapIndexError(ValueError):
    pass


@dataclass(frozen=True)
class SnapCandidateV1:
    axis: SnapAxisV1
    position_emu: int
    target_anchor: SnapAnchorKindV1
    target_kind: SnapTargetKindV1
    target_node_id: str | None


@dataclass(frozen=True)
class SnapFeedbackV1:
    axis: SnapAxisV1
    position_emu: int
    moving_anchor: SnapAnchorKindV1
    target_anchor: SnapAnchorKindV1
    target_kind: SnapTargetKindV1
    target_node_id: str | None


@dataclass(frozen=True)
class SnapAxisMatchV1:
    correction_emu: int
    feedback: SnapFeedbackV1


@dataclass(frozen=True)
class SnapIndexV1:
    candidates: tuple[SnapCandidateV1, ...]

    @classmethod
    def build(cls, candidates: tuple[SnapCandidateV1, ...]) -> "SnapIndexV1":
        if not isinstance(candidates, tuple):
            raise SnapIndexError("candidates must be an ordered tuple")
        seen = set()
        validated = []
        for index, candidate in enumerate(candidates):
            _validate_candidate(candidate, index)
            identity = (
                candidate.axis,
                candidate.target_kind,
                candidate.target_node_id,
                candidate.target_anchor,
            )
            if identity in seen:
                raise SnapIndexError("duplicate snap candidate identity")
            seen.add(identity)
            validated.append(candidate)
        validated.sort(
            key=lambda candidate: (
                0 if candidate.axis == "x" else 1,
                _TARGET_KIND_RANK[candidate.target_kind],
                candidate.target_node_id or "",
                _ANCHOR_RANK[candidate.target_anchor],
                candidate.position_emu,
            )
        )
        return cls(candidates=tuple(validated))

    def best_axis_match_v1(
        self,
        *,
        axis: SnapAxisV1,
        moving_anchors: tuple[tuple[SnapAnchorKindV1, int], ...],
        tolerance_emu: int,
        excluded_node_ids: tuple[str, ...] = (),
    ) -> SnapAxisMatchV1 | None:
        if axis not in {"x", "y"}:
            raise SnapIndexError("axis must be x or y")
        _checked_nonnegative_emu(tolerance_emu, "tolerance_emu")
        if not isinstance(moving_anchors, tuple) or not moving_anchors:
            raise SnapIndexError("moving_anchors must be a non-empty tuple")
        if not isinstance(excluded_node_ids, tuple):
            raise SnapIndexError("excluded_node_ids must be a tuple")
        if any(not isinstance(value, str) or not value for value in excluded_node_ids):
            raise SnapIndexError("excluded_node_ids must contain non-empty strings")
        if len(set(excluded_node_ids)) != len(excluded_node_ids):
            raise SnapIndexError("excluded_node_ids must be unique")
        excluded = set(excluded_node_ids)

        normalized_anchors = []
        for index, anchor in enumerate(moving_anchors):
            if (
                not isinstance(anchor, tuple)
                or len(anchor) != 2
                or anchor[0] not in _ANCHOR_RANK
            ):
                raise SnapIndexError(f"moving_anchors[{index}] is invalid")
            _checked_emu(anchor[1], f"moving_anchors[{index}].position_emu")
            normalized_anchors.append(anchor)

        best_key = None
        best_match = None
        for moving_anchor, moving_position in normalized_anchors:
            for candidate in self.candidates:
                if candidate.axis != axis:
                    continue
                if (
                    candidate.target_node_id is not None
                    and candidate.target_node_id in excluded
                ):
                    continue
                correction = candidate.position_emu - moving_position
                distance = abs(correction)
                if distance > tolerance_emu:
                    continue

                key = (
                    distance,
                    _TARGET_KIND_RANK[candidate.target_kind],
                    candidate.target_node_id or "",
                    _ANCHOR_RANK[candidate.target_anchor],
                    _ANCHOR_RANK[moving_anchor],
                )
                if best_key is None or key < best_key:
                    best_key = key
                    best_match = SnapAxisMatchV1(
                        correction_emu=correction,
                        feedback=SnapFeedbackV1(
                            axis=axis,
                            position_emu=candidate.position_emu,
                            moving_anchor=moving_anchor,
                            target_anchor=candidate.target_anchor,
                            target_kind=candidate.target_kind,
                            target_node_id=candidate.target_node_id,
                        ),
                    )
        return best_match


def _checked_emu(value: int, label: str) -> int:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value < MIN_SAFE_EMU
        or value > MAX_SAFE_EMU
    ):
        raise SnapIndexError(f"{label} must be a JavaScript-safe EMU integer")
    return value


def _checked_nonnegative_emu(value: int, label: str) -> int:
    _checked_emu(value, label)
    if value < 0:
        raise SnapIndexError(f"{label} must be non-negative")
    return value


def _validate_candidate(candidate: SnapCandidateV1, index: int) -> None:
    if not isinstance(candidate, SnapCandidateV1):
        raise SnapIndexError(f"candidate[{index}] must be SnapCandidateV1")
    if candidate.axis not in {"x", "y"}:
        raise SnapIndexError(f"candidate[{index}].axis is invalid")
    _checked_emu(candidate.position_emu, f"candidate[{index}].position_emu")
    if candidate.target_anchor not in _ANCHOR_RANK:
        raise SnapIndexError(f"candidate[{index}].target_anchor is invalid")
    if candidate.target_kind not in _TARGET_KIND_RANK:
        raise SnapIndexError(f"candidate[{index}].target_kind is invalid")

    object_kind = candidate.target_kind in {"object_edge", "object_center"}
    if object_kind:
        if not isinstance(candidate.target_node_id, str) or not candidate.target_node_id:
            raise SnapIndexError(
                f"candidate[{index}] object target requires target_node_id"
            )
    elif candidate.target_node_id is not None:
        raise SnapIndexError(
            f"candidate[{index}] page target must not carry target_node_id"
        )

    if candidate.target_kind.endswith("_center") and candidate.target_anchor != "center":
        raise SnapIndexError(
            f"candidate[{index}] center target requires center anchor"
        )
    if candidate.target_kind.endswith("_edge") and candidate.target_anchor == "center":
        raise SnapIndexError(
            f"candidate[{index}] edge target cannot use center anchor"
        )
