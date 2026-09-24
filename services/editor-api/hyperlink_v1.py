#!/usr/bin/env python3
"""Canonical source-neutral HyperlinkSpan authoring V1.

V1 owns explicit author-created absolute http/https links over non-empty
canonical Story scalar ranges. Hyperlink identity is semantic, never inferred
from paint.

Broad automatic survival through text edits is intentionally NOT defined here:
EXP-TEXT-HYPERLINK-EDIT-POLICY-01 owns hyperlink-specific boundary affinity,
crossing edits, full-cover identity survival, and span-id lifetime. Until that
research closes, only edits strictly outside every affected hyperlink boundary
are admitted for automatic coordinate rebasing; ambiguous touches/intersections
fail closed.

EditorProject storage:
    project["hyperlinks"] = {span_id: serialized HyperlinkSpanV1, ...}

The source PUB remains immutable; these are Chaptera project semantics.
"""

from __future__ import annotations

import copy
from dataclasses import dataclass, replace
from typing import Literal
from urllib.parse import urlsplit, urlunsplit

from range_anchor_rebase_v1 import (
    AnchoredRangeV1,
    RangeAnchorPolicyV1,
    StoryRangeEditV1,
    rebase_anchored_range_v1,
)


HyperlinkProvenanceV1 = Literal["chaptera_created", "source_publisher"]


class HyperlinkAuthoringError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class HyperlinkTargetV1:
    protocol_version: Literal["chaptera.hyperlink-target.v1"]
    kind: Literal["absolute_url"]
    raw_url: str
    normalized_url: str


@dataclass(frozen=True)
class HyperlinkSpanV1:
    protocol_version: Literal["chaptera.hyperlink-span.v1"]
    span_id: str
    story_id: str
    start_scalar: int
    end_scalar: int
    target: HyperlinkTargetV1
    provenance: HyperlinkProvenanceV1


def _fail(code: str, message: str) -> None:
    raise HyperlinkAuthoringError(code, message)


def _require_string(value: object, label: str, *, min_len: int = 1) -> str:
    if not isinstance(value, str) or len(value) < min_len:
        _fail("invalid_hyperlink", f"{label} is required")
    return value


def _require_scalar(value: object, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        _fail("invalid_hyperlink_range", f"{label} must be non-negative integer")
    return value


def normalize_hyperlink_target_v1(raw_url: str) -> HyperlinkTargetV1:
    """Validate an absolute web URL while preserving the caller spelling."""
    _require_string(raw_url, "raw_url")
    if raw_url != raw_url.strip():
        _fail("invalid_hyperlink_target", "URL must not contain outer whitespace")
    if any(ord(ch) < 0x20 or ch == "\x7f" for ch in raw_url):
        _fail("invalid_hyperlink_target", "URL must not contain control characters")
    try:
        parts = urlsplit(raw_url)
        _ = parts.port
    except ValueError as exc:
        _fail("invalid_hyperlink_target", f"invalid URL: {exc}")
    scheme = parts.scheme.lower()
    if scheme not in {"http", "https"}:
        _fail("unsupported_hyperlink_target", "V1 supports absolute http/https URLs only")
    if not parts.netloc:
        _fail("invalid_hyperlink_target", "absolute URL requires authority/host")
    normalized = urlunsplit(
        (scheme, parts.netloc, parts.path, parts.query, parts.fragment)
    )
    return HyperlinkTargetV1(
        protocol_version="chaptera.hyperlink-target.v1",
        kind="absolute_url",
        raw_url=raw_url,
        normalized_url=normalized,
    )


def hyperlink_target_to_dict_v1(target: HyperlinkTargetV1) -> dict:
    if not isinstance(target, HyperlinkTargetV1):
        _fail("invalid_hyperlink_target", "HyperlinkTargetV1 is required")
    normalized = normalize_hyperlink_target_v1(target.raw_url)
    if normalized != target:
        _fail("invalid_hyperlink_target", "target normalization receipt is inconsistent")
    return {
        "protocol_version": target.protocol_version,
        "kind": target.kind,
        "raw_url": target.raw_url,
        "normalized_url": target.normalized_url,
    }


def hyperlink_target_from_dict_v1(value: dict) -> HyperlinkTargetV1:
    if not isinstance(value, dict) or set(value) != {
        "protocol_version",
        "kind",
        "raw_url",
        "normalized_url",
    }:
        _fail("invalid_hyperlink_target", "serialized hyperlink target shape is invalid")
    if value.get("protocol_version") != "chaptera.hyperlink-target.v1":
        _fail("invalid_hyperlink_target", "target protocol mismatch")
    if value.get("kind") != "absolute_url":
        _fail("unsupported_hyperlink_target", "target kind is unsupported")
    target = normalize_hyperlink_target_v1(value.get("raw_url"))
    if target.normalized_url != value.get("normalized_url"):
        _fail("invalid_hyperlink_target", "serialized normalized URL is inconsistent")
    return target


def _validate_span(span: HyperlinkSpanV1) -> None:
    if not isinstance(span, HyperlinkSpanV1):
        _fail("invalid_hyperlink", "HyperlinkSpanV1 is required")
    if span.protocol_version != "chaptera.hyperlink-span.v1":
        _fail("invalid_hyperlink", "hyperlink span protocol mismatch")
    _require_string(span.span_id, "span_id")
    _require_string(span.story_id, "story_id")
    start = _require_scalar(span.start_scalar, "start_scalar")
    end = _require_scalar(span.end_scalar, "end_scalar")
    if end <= start:
        _fail("invalid_hyperlink_range", "hyperlink range must be non-empty")
    hyperlink_target_to_dict_v1(span.target)
    if span.provenance not in {"chaptera_created", "source_publisher"}:
        _fail("invalid_hyperlink", "unsupported hyperlink provenance")


def hyperlink_span_to_dict_v1(span: HyperlinkSpanV1) -> dict:
    _validate_span(span)
    return {
        "protocol_version": span.protocol_version,
        "span_id": span.span_id,
        "story_id": span.story_id,
        "start_scalar": span.start_scalar,
        "end_scalar": span.end_scalar,
        "target": hyperlink_target_to_dict_v1(span.target),
        "provenance": span.provenance,
    }


def hyperlink_span_from_dict_v1(value: dict) -> HyperlinkSpanV1:
    if not isinstance(value, dict) or set(value) != {
        "protocol_version",
        "span_id",
        "story_id",
        "start_scalar",
        "end_scalar",
        "target",
        "provenance",
    }:
        _fail("invalid_hyperlink", "serialized hyperlink span shape is invalid")
    span = HyperlinkSpanV1(
        protocol_version=value.get("protocol_version"),
        span_id=value.get("span_id"),
        story_id=value.get("story_id"),
        start_scalar=value.get("start_scalar"),
        end_scalar=value.get("end_scalar"),
        target=hyperlink_target_from_dict_v1(value.get("target")),
        provenance=value.get("provenance"),
    )
    _validate_span(span)
    return span


def _validate_span_against_story(span: HyperlinkSpanV1, story_text: str) -> None:
    _validate_span(span)
    if not isinstance(story_text, str):
        _fail("invalid_story", "canonical Story text is required")
    if span.end_scalar > len(story_text):
        _fail("invalid_hyperlink_range", "hyperlink range exceeds canonical Story extent")


def _project_hyperlinks(project: dict) -> dict[str, dict]:
    raw = project.get("hyperlinks", {})
    if not isinstance(raw, dict):
        _fail("invalid_hyperlink_store", "project hyperlinks must be an object")
    out: dict[str, dict] = {}
    for key, value in raw.items():
        span = hyperlink_span_from_dict_v1(value)
        if key != span.span_id:
            _fail("invalid_hyperlink_store", "hyperlink map key differs from span_id")
        out[key] = hyperlink_span_to_dict_v1(span)
    return out


def _story_text(project: dict, story_id: str) -> str:
    stories = project.get("stories")
    if not isinstance(stories, dict) or story_id not in stories:
        _fail("story_not_found", "hyperlink Story is absent from EditorProject")
    text = stories[story_id]
    if not isinstance(text, str):
        _fail("invalid_story", "EditorProject Story text must be string")
    return text


def _overlaps(a: HyperlinkSpanV1, b: HyperlinkSpanV1) -> bool:
    return (
        a.story_id == b.story_id
        and max(a.start_scalar, b.start_scalar) < min(a.end_scalar, b.end_scalar)
    )


def _ensure_no_overlap(
    span: HyperlinkSpanV1,
    store: dict[str, dict],
    *,
    exclude_span_id: str | None = None,
) -> None:
    for span_id, raw in store.items():
        if span_id == exclude_span_id:
            continue
        other = hyperlink_span_from_dict_v1(raw)
        if _overlaps(span, other):
            _fail(
                "hyperlink_overlap",
                f"hyperlink {span.span_id} overlaps existing span {other.span_id}",
            )


def build_hyperlink_span_v1(
    *,
    span_id: str,
    story_id: str,
    start_scalar: int,
    end_scalar: int,
    raw_url: str,
    provenance: HyperlinkProvenanceV1 = "chaptera_created",
) -> HyperlinkSpanV1:
    span = HyperlinkSpanV1(
        protocol_version="chaptera.hyperlink-span.v1",
        span_id=span_id,
        story_id=story_id,
        start_scalar=start_scalar,
        end_scalar=end_scalar,
        target=normalize_hyperlink_target_v1(raw_url),
        provenance=provenance,
    )
    _validate_span(span)
    return span


def validate_hyperlink_request_v1(request: dict) -> None:
    if not isinstance(request, dict) or set(request) != {
        "protocol_version",
        "document_id",
        "source_hash",
        "base_revision_id",
        "client_operation_id",
        "command",
    }:
        _fail("invalid_command", "hyperlink request shape is invalid")
    if request.get("protocol_version") != "chaptera.hyperlink-intent.v1":
        _fail("invalid_command", "hyperlink request protocol mismatch")
    for label in (
        "document_id",
        "source_hash",
        "base_revision_id",
        "client_operation_id",
    ):
        _require_string(
            request.get(label),
            label,
            min_len=8 if label == "client_operation_id" else 1,
        )
    command = request.get("command")
    if not isinstance(command, dict):
        _fail("invalid_command", "hyperlink command is required")
    kind = command.get("kind")
    if kind == "hyperlink_create":
        if set(command) != {"kind", "span"}:
            _fail("invalid_command", "hyperlink_create shape is invalid")
        span = hyperlink_span_from_dict_v1(command["span"])
        if span.provenance != "chaptera_created":
            _fail(
                "invalid_hyperlink",
                "explicit authoring create may only create chaptera_created links",
            )
    elif kind == "hyperlink_update":
        if set(command) != {"kind", "expected_before", "raw_url"}:
            _fail("invalid_command", "hyperlink_update shape is invalid")
        before = hyperlink_span_from_dict_v1(command["expected_before"])
        if before.provenance != "chaptera_created":
            _fail("source_hyperlink_read_only", "source-projected hyperlink is not V1 editable")
        normalize_hyperlink_target_v1(command["raw_url"])
    elif kind == "hyperlink_remove":
        if set(command) != {"kind", "expected_before"}:
            _fail("invalid_command", "hyperlink_remove shape is invalid")
        before = hyperlink_span_from_dict_v1(command["expected_before"])
        if before.provenance != "chaptera_created":
            _fail("source_hyperlink_read_only", "source-projected hyperlink is not V1 editable")
    else:
        _fail("invalid_command", "unsupported hyperlink operation")


def _canonical_operation(
    *,
    kind: str,
    before: HyperlinkSpanV1 | None,
    after: HyperlinkSpanV1 | None,
) -> dict:
    span = after if after is not None else before
    assert span is not None
    return {
        "protocol_version": "chaptera.hyperlink-operation.v1",
        "kind": kind,
        "span_id": span.span_id,
        "story_id": span.story_id,
        "before_span": None if before is None else hyperlink_span_to_dict_v1(before),
        "after_span": None if after is None else hyperlink_span_to_dict_v1(after),
    }


def validate_hyperlink_operation_v1(command: dict, operation: dict) -> None:
    if not isinstance(operation, dict) or set(operation) != {
        "protocol_version",
        "kind",
        "span_id",
        "story_id",
        "before_span",
        "after_span",
    }:
        _fail("invalid_canonical_operation", "canonical hyperlink operation shape is invalid")
    if operation.get("protocol_version") != "chaptera.hyperlink-operation.v1":
        _fail("invalid_canonical_operation", "hyperlink operation protocol mismatch")
    kind = command.get("kind")
    if operation.get("kind") != kind:
        _fail("invalid_canonical_operation", "operation kind differs from request")
    before = (
        None
        if operation.get("before_span") is None
        else hyperlink_span_from_dict_v1(operation["before_span"])
    )
    after = (
        None
        if operation.get("after_span") is None
        else hyperlink_span_from_dict_v1(operation["after_span"])
    )
    if kind == "hyperlink_create":
        expected = hyperlink_span_from_dict_v1(command["span"])
        if before is not None or after != expected:
            _fail("invalid_canonical_operation", "create before/after receipt is invalid")
    elif kind == "hyperlink_update":
        expected_before = hyperlink_span_from_dict_v1(command["expected_before"])
        expected_after = replace(
            expected_before,
            target=normalize_hyperlink_target_v1(command["raw_url"]),
        )
        if before != expected_before or after != expected_after:
            _fail("invalid_canonical_operation", "update before/after receipt is invalid")
    elif kind == "hyperlink_remove":
        expected_before = hyperlink_span_from_dict_v1(command["expected_before"])
        if before != expected_before or after is not None:
            _fail("invalid_canonical_operation", "remove before/after receipt is invalid")
    else:
        _fail("invalid_canonical_operation", "operation kind is unsupported")
    span = after if after is not None else before
    assert span is not None
    if operation["span_id"] != span.span_id or operation["story_id"] != span.story_id:
        _fail("invalid_canonical_operation", "operation identity differs from span receipt")


def _apply_span_transition(
    project: dict,
    *,
    before: HyperlinkSpanV1 | None,
    after: HyperlinkSpanV1 | None,
) -> dict:
    resulting = copy.deepcopy(project)
    store = _project_hyperlinks(resulting)
    span = after if after is not None else before
    assert span is not None
    current_raw = store.get(span.span_id)
    current = None if current_raw is None else hyperlink_span_from_dict_v1(current_raw)
    if current != before:
        _fail("hyperlink_precondition_failed", "current hyperlink differs from exact before state")
    if after is None:
        store.pop(span.span_id, None)
    else:
        story_text = _story_text(resulting, after.story_id)
        _validate_span_against_story(after, story_text)
        _ensure_no_overlap(after, store, exclude_span_id=after.span_id)
        store[after.span_id] = hyperlink_span_to_dict_v1(after)
    resulting["hyperlinks"] = {key: store[key] for key in sorted(store)}
    return resulting


def replay_hyperlink_operation_v1(project: dict, operation: dict) -> dict:
    if not isinstance(operation, dict):
        _fail("invalid_canonical_operation", "canonical hyperlink operation is required")
    before = (
        None
        if operation.get("before_span") is None
        else hyperlink_span_from_dict_v1(operation["before_span"])
    )
    after = (
        None
        if operation.get("after_span") is None
        else hyperlink_span_from_dict_v1(operation["after_span"])
    )
    resulting = _apply_span_transition(project, before=before, after=after)
    operations = resulting.get("operations")
    if not isinstance(operations, list):
        _fail("invalid_project", "EditorProject operations must be a list")
    operations.append(copy.deepcopy(operation))
    return resulting


def undo_hyperlink_operation_v1(project: dict, operation: dict) -> dict:
    if not isinstance(operation, dict):
        _fail("invalid_canonical_operation", "canonical hyperlink operation is required")
    before = (
        None
        if operation.get("before_span") is None
        else hyperlink_span_from_dict_v1(operation["before_span"])
    )
    after = (
        None
        if operation.get("after_span") is None
        else hyperlink_span_from_dict_v1(operation["after_span"])
    )
    resulting = _apply_span_transition(project, before=after, after=before)
    operations = resulting.get("operations")
    if not isinstance(operations, list) or not operations or operations[-1] != operation:
        _fail(
            "hyperlink_undo_precondition_failed",
            "exact hyperlink operation must be the current replay tail",
        )
    operations.pop()
    return resulting


def execute_hyperlink_operation_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list]:
    if not isinstance(base_project, dict):
        _fail("invalid_project", "EditorProject object is required")
    validate_hyperlink_request_v1(
        {
            "protocol_version": "chaptera.hyperlink-intent.v1",
            "document_id": "executor",
            "source_hash": "executor",
            "base_revision_id": "executor",
            "client_operation_id": "executor-id",
            "command": copy.deepcopy(command),
        }
    )
    store = _project_hyperlinks(base_project)
    kind = command["kind"]

    if kind == "hyperlink_create":
        before = None
        after = hyperlink_span_from_dict_v1(command["span"])
        if after.span_id in store:
            _fail("hyperlink_already_exists", "span_id already exists")
        _validate_span_against_story(after, _story_text(base_project, after.story_id))
        _ensure_no_overlap(after, store)
    elif kind == "hyperlink_update":
        before = hyperlink_span_from_dict_v1(command["expected_before"])
        raw = store.get(before.span_id)
        current = None if raw is None else hyperlink_span_from_dict_v1(raw)
        if current != before:
            _fail("hyperlink_precondition_failed", "current hyperlink differs from expected_before")
        after = replace(
            before,
            target=normalize_hyperlink_target_v1(command["raw_url"]),
        )
        _ensure_no_overlap(after, store, exclude_span_id=after.span_id)
    elif kind == "hyperlink_remove":
        before = hyperlink_span_from_dict_v1(command["expected_before"])
        raw = store.get(before.span_id)
        current = None if raw is None else hyperlink_span_from_dict_v1(raw)
        if current != before:
            _fail("hyperlink_precondition_failed", "current hyperlink differs from expected_before")
        after = None
    else:
        _fail("invalid_command", "unsupported hyperlink operation")

    operation = _canonical_operation(kind=kind, before=before, after=after)
    resulting = replay_hyperlink_operation_v1(base_project, operation)
    consequences = [
        {
            "kind": "hyperlink_semantics_changed",
            "story_id": operation["story_id"],
            "span_id": operation["span_id"],
            "layout_invalidation": False,
        }
    ]
    return operation, resulting, consequences


def _edit_relation_to_span(
    *,
    span: HyperlinkSpanV1,
    edit: StoryRangeEditV1,
) -> Literal["strictly_before", "strictly_after", "ambiguous"]:
    if edit.end_scalar < span.start_scalar:
        return "strictly_before"
    if edit.start_scalar > span.end_scalar:
        return "strictly_after"
    return "ambiguous"


def rebase_hyperlinks_for_unambiguous_story_edit_v1(
    *,
    spans: tuple[HyperlinkSpanV1, ...],
    story_id: str,
    edit_start_scalar: int,
    edit_end_scalar: int,
    replacement_length: int,
) -> tuple[HyperlinkSpanV1, ...]:
    """Rebase only edits whose hyperlink outcome cannot depend on policy."""
    _require_string(story_id, "story_id")
    edit = StoryRangeEditV1(
        start_scalar=edit_start_scalar,
        end_scalar=edit_end_scalar,
        replacement_length=replacement_length,
    )
    probe = AnchoredRangeV1(0, 0, allow_empty=True)
    rebase_anchored_range_v1(
        anchored=probe,
        policy=RangeAnchorPolicyV1("left", "left", "invalidate"),
        edit=edit,
    )

    out = []
    for span in spans:
        _validate_span(span)
        if span.story_id != story_id:
            out.append(span)
            continue
        relation = _edit_relation_to_span(span=span, edit=edit)
        if relation == "ambiguous":
            _fail(
                "hyperlink_edit_policy_required",
                f"text edit touches hyperlink {span.span_id}; policy experiment is required",
            )
        receipt = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(
                span.start_scalar,
                span.end_scalar,
                allow_empty=False,
            ),
            policy=RangeAnchorPolicyV1(
                start_affinity="left",
                end_affinity="right",
                full_cover_policy="invalidate",
            ),
            edit=edit,
        )
        if receipt.result.status != "survives" or receipt.result.range is None:
            _fail(
                "hyperlink_edit_policy_required",
                "bounded hyperlink rebase unexpectedly required semantic survival policy",
            )
        out.append(
            replace(
                span,
                start_scalar=receipt.result.range.start_scalar,
                end_scalar=receipt.result.range.end_scalar,
            )
        )
    return tuple(
        sorted(
            out,
            key=lambda item: (
                item.story_id,
                item.start_scalar,
                item.end_scalar,
                item.span_id,
            ),
        )
    )
