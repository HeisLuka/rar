#!/usr/bin/env python3
"""Source-neutral Chaptera authored overlay stack V1.

Lane order is explicit: index 0 is authored back/bottom and the last index is
authored front/top. Imported/source-backed stacking is a separate base lane and
is intentionally absent from this model.
"""

from __future__ import annotations

MODES = frozenset({"step_forward", "step_backward", "to_front", "to_back"})


def validate_authored_lane(lane):
    if not isinstance(lane, list):
        raise ValueError("AuthoredStackV1 lane must be a list")
    if any(not isinstance(node_id, str) or not node_id for node_id in lane):
        raise ValueError("AuthoredStackV1 members must be non-empty NodeIds")
    if len(set(lane)) != len(lane):
        raise ValueError("AuthoredStackV1 lane contains duplicate NodeIds")


def append_authored_member(lane, node_id):
    validate_authored_lane(lane)
    if not isinstance(node_id, str) or not node_id:
        raise ValueError("node_id is required")
    if node_id in lane:
        raise ValueError("authored member already exists")
    return list(lane) + [node_id]


def reorder_authored_lane(lane, node_id, mode):
    validate_authored_lane(lane)
    if mode not in MODES:
        raise ValueError("unsupported authored-stack reorder mode")
    if node_id not in lane:
        raise ValueError("authored-stack target is not a lane member")

    before = list(lane)
    index = before.index(node_id)
    if mode == "step_forward":
        target = min(index + 1, len(before) - 1)
    elif mode == "step_backward":
        target = max(index - 1, 0)
    elif mode == "to_front":
        target = len(before) - 1
    else:
        target = 0

    if target == index:
        raise ValueError("authored-stack reorder is a no-op")

    after = list(before)
    after.pop(index)
    after.insert(target, node_id)
    return after
