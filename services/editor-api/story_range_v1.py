#!/usr/bin/env python3
"""Canonical bounded Story range replacement V1.

Coordinates are Unicode scalar indices in canonical Chaptera Story text.
Python str indices map Unicode code points; this module rejects surrogate code
points so accepted text is a Unicode scalar sequence.

This primitive owns only low-level exact range replacement:
- exact expected-before precondition;
- scalar-coordinate splice;
- optional authoritative terminal-CR preservation policy;
- exact inverse data;
- deterministic serialization/replay;
- Story state identity derived from StoryId + final canonical text only.

External newline normalization, NFC/NFD policy, edit-domain authorization,
formatting, hyperlink/range rebasing, IME and browser selection are separate
layers.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from typing import Any


PROTOCOL_VERSION_V1 = "chaptera.replace-story-range.v1"
MAX_STORY_SCALARS_V1 = 0xFFFFFFFF


class StoryRangeError(ValueError):
    pass


@dataclass(frozen=True)
class StoryRangeInverseV1:
    start_scalar: int
    end_scalar: int
    expected_before: str
    replacement_text: str


@dataclass(frozen=True)
class StoryRangeResultV1:
    story_id: str
    before_text: str
    after_text: str
    operation: dict
    inverse: StoryRangeInverseV1


def _fail(message: str) -> None:
    raise StoryRangeError(message)


def validate_scalar_sequence_v1(text: str, label: str) -> int:
    if not isinstance(text, str):
        _fail(f"{label} must be a string")
    for index, ch in enumerate(text):
        cp = ord(ch)
        if 0xD800 <= cp <= 0xDFFF:
            _fail(f"{label}[{index}] is a surrogate, not a Unicode scalar")
    scalar_len = len(text)
    if scalar_len > MAX_STORY_SCALARS_V1:
        _fail(f"{label} exceeds V1 scalar bound")
    return scalar_len


def story_text_hash_v1(text: str) -> str:
    validate_scalar_sequence_v1(text, "story_text")
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def story_state_id_v1(story_id: str, text: str) -> str:
    if not isinstance(story_id, str) or not story_id:
        _fail("story_id is required")
    validate_scalar_sequence_v1(text, "story_text")
    payload = json.dumps(
        {
            "protocol_version": "chaptera.story-state.v1",
            "story_id": story_id,
            "text": text,
        },
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def _validate_range(start_scalar: int, end_scalar: int, scalar_len: int) -> None:
    if (
        not isinstance(start_scalar, int)
        or isinstance(start_scalar, bool)
        or not isinstance(end_scalar, int)
        or isinstance(end_scalar, bool)
        or start_scalar < 0
        or end_scalar < start_scalar
        or end_scalar > scalar_len
    ):
        _fail("Story scalar range is outside canonical text")


def _inverse_for(
    *,
    start_scalar: int,
    expected_before: str,
    replacement_text: str,
) -> StoryRangeInverseV1:
    return StoryRangeInverseV1(
        start_scalar=start_scalar,
        end_scalar=start_scalar + len(replacement_text),
        expected_before=replacement_text,
        replacement_text=expected_before,
    )


def replace_story_range_v1(
    *,
    story_id: str,
    story_text: str,
    start_scalar: int,
    end_scalar: int,
    expected_before: str,
    replacement_text: str,
    requires_terminal_cr: bool = False,
) -> StoryRangeResultV1:
    if not isinstance(story_id, str) or not story_id:
        _fail("story_id is required")
    scalar_len = validate_scalar_sequence_v1(story_text, "story_text")
    validate_scalar_sequence_v1(expected_before, "expected_before")
    replacement_len = validate_scalar_sequence_v1(
        replacement_text,
        "replacement_text",
    )
    _validate_range(start_scalar, end_scalar, scalar_len)

    actual_before = story_text[start_scalar:end_scalar]
    if actual_before != expected_before:
        _fail("expected_before does not match canonical Story range")

    if not isinstance(requires_terminal_cr, bool):
        _fail("requires_terminal_cr must be boolean")
    if requires_terminal_cr and not story_text.endswith("\r"):
        _fail("source-backed terminal CR invariant is already violated")

    after_text = (
        story_text[:start_scalar]
        + replacement_text
        + story_text[end_scalar:]
    )
    validate_scalar_sequence_v1(after_text, "after_text")
    if len(after_text) > MAX_STORY_SCALARS_V1:
        _fail("resulting Story exceeds V1 scalar bound")
    if requires_terminal_cr and not after_text.endswith("\r"):
        _fail("replacement would violate source-backed terminal CR invariant")

    inverse = _inverse_for(
        start_scalar=start_scalar,
        expected_before=expected_before,
        replacement_text=replacement_text,
    )
    operation = {
        "protocol_version": PROTOCOL_VERSION_V1,
        "kind": "replace_story_range",
        "story_id": story_id,
        "start_scalar": start_scalar,
        "end_scalar": end_scalar,
        "expected_before": expected_before,
        "replacement_text": replacement_text,
        "inverse": {
            "start_scalar": inverse.start_scalar,
            "end_scalar": inverse.end_scalar,
            "expected_before": inverse.expected_before,
            "replacement_text": inverse.replacement_text,
        },
        "before_text_hash": story_text_hash_v1(story_text),
        "after_text_hash": story_text_hash_v1(after_text),
        "before_story_state_id": story_state_id_v1(story_id, story_text),
        "after_story_state_id": story_state_id_v1(story_id, after_text),
    }
    return StoryRangeResultV1(
        story_id=story_id,
        before_text=story_text,
        after_text=after_text,
        operation=operation,
        inverse=inverse,
    )


def validate_story_range_operation_v1(command: dict, operation: dict) -> None:
    if not isinstance(command, dict):
        _fail("Story range command must be an object")
    expected_command_fields = {
        "kind",
        "story_id",
        "start_scalar",
        "end_scalar",
        "expected_before",
        "replacement_text",
    }
    if set(command) != expected_command_fields:
        _fail("replace_story_range intent fields are not exact V1")
    if command.get("kind") != "replace_story_range":
        _fail("replace_story_range intent kind is required")

    validate_scalar_sequence_v1(command.get("expected_before"), "expected_before")
    validate_scalar_sequence_v1(command.get("replacement_text"), "replacement_text")
    start = command.get("start_scalar")
    end = command.get("end_scalar")
    if (
        not isinstance(start, int)
        or isinstance(start, bool)
        or not isinstance(end, int)
        or isinstance(end, bool)
        or start < 0
        or end < start
        or end > MAX_STORY_SCALARS_V1
    ):
        _fail("Story scalar range is invalid")

    if not isinstance(operation, dict):
        _fail("canonical Story range operation must be an object")
    expected_operation_fields = {
        "protocol_version",
        "kind",
        "story_id",
        "start_scalar",
        "end_scalar",
        "expected_before",
        "replacement_text",
        "inverse",
        "before_text_hash",
        "after_text_hash",
        "before_story_state_id",
        "after_story_state_id",
    }
    if set(operation) != expected_operation_fields:
        _fail("canonical Story range operation fields are not exact V1")
    if operation.get("protocol_version") != PROTOCOL_VERSION_V1:
        _fail("canonical Story range protocol_version mismatch")

    for field in (
        "kind",
        "story_id",
        "start_scalar",
        "end_scalar",
        "expected_before",
        "replacement_text",
    ):
        if operation.get(field) != command.get(field):
            _fail(f"canonical Story range {field} differs from accepted intent")

    inverse = operation.get("inverse")
    expected_inverse = _inverse_for(
        start_scalar=start,
        expected_before=command["expected_before"],
        replacement_text=command["replacement_text"],
    )
    if inverse != {
        "start_scalar": expected_inverse.start_scalar,
        "end_scalar": expected_inverse.end_scalar,
        "expected_before": expected_inverse.expected_before,
        "replacement_text": expected_inverse.replacement_text,
    }:
        _fail("canonical Story range inverse violates V1 law")

    for field in ("before_text_hash", "after_text_hash"):
        value = operation.get(field)
        if (
            not isinstance(value, str)
            or len(value) != 64
            or any(ch not in "0123456789abcdef" for ch in value)
        ):
            _fail(f"{field} must be lowercase SHA-256")
    for field in ("before_story_state_id", "after_story_state_id"):
        value = operation.get(field)
        if (
            not isinstance(value, str)
            or not value.startswith("sha256:")
            or len(value) != 71
            or any(ch not in "0123456789abcdef" for ch in value[7:])
        ):
            _fail(f"{field} must be sha256: identity")


def replay_story_range_operation_v1(
    *,
    story_text: str,
    operation: dict,
    requires_terminal_cr: bool = False,
) -> str:
    if not isinstance(operation, dict) or operation.get("protocol_version") != PROTOCOL_VERSION_V1:
        _fail("serialized ReplaceStoryRangeV1 operation is required")
    command = {
        "kind": operation.get("kind"),
        "story_id": operation.get("story_id"),
        "start_scalar": operation.get("start_scalar"),
        "end_scalar": operation.get("end_scalar"),
        "expected_before": operation.get("expected_before"),
        "replacement_text": operation.get("replacement_text"),
    }
    validate_story_range_operation_v1(command, operation)
    result = replace_story_range_v1(
        story_id=command["story_id"],
        story_text=story_text,
        start_scalar=command["start_scalar"],
        end_scalar=command["end_scalar"],
        expected_before=command["expected_before"],
        replacement_text=command["replacement_text"],
        requires_terminal_cr=requires_terminal_cr,
    )
    if result.operation != operation:
        _fail("serialized operation does not replay to identical canonical operation")
    return result.after_text


def apply_story_range_inverse_v1(
    *,
    story_id: str,
    story_text: str,
    operation: dict,
    requires_terminal_cr: bool = False,
) -> str:
    if operation.get("story_id") != story_id:
        _fail("inverse StoryId mismatch")
    inverse = operation.get("inverse")
    if not isinstance(inverse, dict):
        _fail("canonical Story range inverse is required")
    result = replace_story_range_v1(
        story_id=story_id,
        story_text=story_text,
        start_scalar=inverse.get("start_scalar"),
        end_scalar=inverse.get("end_scalar"),
        expected_before=inverse.get("expected_before"),
        replacement_text=inverse.get("replacement_text"),
        requires_terminal_cr=requires_terminal_cr,
    )
    return result.after_text
