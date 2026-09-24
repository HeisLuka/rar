#!/usr/bin/env python3
"""Lower public browser StoryRangeIntentV1 into canonical StoryEditTransactionV1.

The browser owns only factual user intent:
- base revision identity;
- StoryId;
- canonical scalar [start,end) range;
- external replacement text;
- client operation identity / causal dependency.

It does NOT supply canonical before-state, paragraph identities, formatting
participants, or durable Story semantics.

This adapter runs on the server against one already-authoritative base project.
It:
1. reads the canonical StoryEditCoreStateV1 from base_project.story_models;
2. derives expected_before from that canonical Story, never from browser input;
3. normalizes external replacement text once through TextIngressV1;
4. preallocates one ParagraphId for each inserted canonical U+000D;
5. emits one StoryEditTransactionV1 request for RevisionKernel.

No DOM range, UTF-16 position, browser text metric, or native key semantic enters
the canonical transaction.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
from typing import Callable

from story_edit_transaction_v1 import (
    StoryEditTransactionError,
    story_edit_core_state_from_dict,
)
from text_ingress_v1 import TextIngressError, normalize_external_text_v1


class WebTextTransactionAdapterError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


ParagraphIdFactory = Callable[[int], str]


@dataclass(frozen=True)
class BrowserStoryRangeLoweringReceiptV1:
    protocol_version: str
    story_id: str
    start_scalar: int
    end_scalar: int
    before_text_sha256: str
    canonical_replacement_sha256: str
    inserted_paragraph_count: int
    browser_before_state_accepted: bool

    def to_dict(self) -> dict:
        return {
            "protocol_version": self.protocol_version,
            "story_id": self.story_id,
            "start_scalar": self.start_scalar,
            "end_scalar": self.end_scalar,
            "before_text_sha256": self.before_text_sha256,
            "canonical_replacement_sha256": self.canonical_replacement_sha256,
            "inserted_paragraph_count": self.inserted_paragraph_count,
            "browser_before_state_accepted": self.browser_before_state_accepted,
        }


def _fail(code: str, message: str) -> None:
    raise WebTextTransactionAdapterError(code, message)


def _validate_browser_request_shape(request: dict) -> dict:
    if not isinstance(request, dict):
        _fail("invalid_browser_text_intent", "browser StoryRangeIntentV1 must be an object")
    required_outer = {
        "protocol_version",
        "document_id",
        "source_hash",
        "base_revision_id",
        "client_operation_id",
        "command",
    }
    optional_outer = {"depends_on_client_operation_id"}
    fields = set(request)
    if not required_outer.issubset(fields) or fields - required_outer - optional_outer:
        _fail(
            "invalid_browser_text_intent",
            "browser StoryRangeIntentV1 fields are not valid V1",
        )
    if request.get("protocol_version") != "chaptera.story-range-intent.v1":
        _fail("invalid_browser_text_intent", "browser text protocol_version mismatch")

    for field in ("document_id", "source_hash", "base_revision_id", "client_operation_id"):
        value = request.get(field)
        if not isinstance(value, str) or not value:
            _fail("invalid_browser_text_intent", f"{field} is required")
    source_hash = request["source_hash"]
    if len(source_hash) != 64 or any(ch not in "0123456789abcdef" for ch in source_hash):
        _fail("invalid_browser_text_intent", "source_hash must be lowercase SHA-256")
    base_revision = request["base_revision_id"]
    if (
        not base_revision.startswith("sha256:")
        or len(base_revision) != 71
        or any(ch not in "0123456789abcdef" for ch in base_revision[7:])
    ):
        _fail("invalid_browser_text_intent", "base_revision_id must be sha256 identity")
    if not 8 <= len(request["client_operation_id"]) <= 160:
        _fail("invalid_browser_text_intent", "client_operation_id length is invalid")

    depends = request.get("depends_on_client_operation_id")
    if depends is not None and (
        not isinstance(depends, str) or not 8 <= len(depends) <= 160
    ):
        _fail(
            "invalid_browser_text_intent",
            "depends_on_client_operation_id must be bounded or null",
        )

    command = request.get("command")
    expected_command = {
        "kind",
        "story_id",
        "start_scalar",
        "end_scalar",
        "replacement_text",
    }
    if (
        not isinstance(command, dict)
        or set(command) != expected_command
        or command.get("kind") != "replace_story_range"
    ):
        _fail(
            "invalid_browser_text_intent",
            "browser command must be exact replace_story_range intent",
        )
    if not isinstance(command["story_id"], str) or not command["story_id"]:
        _fail("invalid_browser_text_intent", "story_id is required")
    for field in ("start_scalar", "end_scalar"):
        value = command[field]
        if not isinstance(value, int) or isinstance(value, bool):
            _fail("invalid_browser_text_intent", f"{field} must be integer")
    if (
        command["start_scalar"] < 0
        or command["end_scalar"] < command["start_scalar"]
        or command["end_scalar"] > 0xFFFFFFFF
    ):
        _fail("invalid_browser_text_intent", "browser Story scalar range is invalid")
    if not isinstance(command["replacement_text"], str):
        _fail("invalid_browser_text_intent", "replacement_text must be string")
    return command


def _authoritative_story(base_project: dict, story_id: str):
    if not isinstance(base_project, dict):
        _fail("invalid_base_project", "base project must be an object")
    story_models = base_project.get("story_models")
    if not isinstance(story_models, dict):
        _fail(
            "canonical_story_model_missing",
            "base project has no authoritative story_models",
        )
    raw = story_models.get(story_id)
    if not isinstance(raw, dict):
        _fail(
            "canonical_story_model_missing",
            "requested Story has no canonical StoryEditCoreStateV1",
        )
    try:
        core = story_edit_core_state_from_dict(raw)
    except (StoryEditTransactionError, ValueError) as exc:
        _fail("invalid_canonical_story_model", str(exc))

    story_text = core.paragraph_state.story_text
    stories = base_project.get("stories")
    if stories is not None:
        if not isinstance(stories, dict) or stories.get(story_id) != story_text:
            _fail(
                "canonical_story_mirror_mismatch",
                "project Story text mirror differs from authoritative Story model",
            )
    return core


def lower_browser_story_range_intent_v1(
    *,
    browser_request: dict,
    base_project: dict,
    paragraph_id_factory: ParagraphIdFactory | None = None,
) -> tuple[dict, BrowserStoryRangeLoweringReceiptV1]:
    command = _validate_browser_request_shape(browser_request)
    core = _authoritative_story(base_project, command["story_id"])
    story_text = core.paragraph_state.story_text

    start = command["start_scalar"]
    end = command["end_scalar"]
    if end > len(story_text):
        _fail(
            "story_range_outside_canonical_story",
            "browser range exceeds canonical Story scalar length",
        )

    expected_before = story_text[start:end]
    try:
        canonical = normalize_external_text_v1(command["replacement_text"])
    except TextIngressError as exc:
        _fail("text_ingress_rejected", str(exc))

    paragraph_count = canonical.paragraph_boundary_count
    paragraph_ids = []
    if paragraph_count:
        if paragraph_id_factory is None:
            _fail(
                "paragraph_id_allocator_required",
                "paragraph insertion requires server-side ParagraphId preallocation",
            )
        for ordinal in range(paragraph_count):
            paragraph_id = paragraph_id_factory(ordinal)
            if not isinstance(paragraph_id, str) or not paragraph_id:
                _fail(
                    "invalid_paragraph_id",
                    "paragraph_id_factory returned invalid identity",
                )
            paragraph_ids.append(paragraph_id)
        if len(set(paragraph_ids)) != len(paragraph_ids):
            _fail("invalid_paragraph_id", "preallocated ParagraphIds must be unique")

    transaction_request = {
        "protocol_version": "chaptera.story-edit-transaction-intent.v1",
        "document_id": browser_request["document_id"],
        "source_hash": browser_request["source_hash"],
        "base_revision_id": browser_request["base_revision_id"],
        "client_operation_id": browser_request["client_operation_id"],
        "depends_on_client_operation_id": browser_request.get("depends_on_client_operation_id"),
        "command": {
            "kind": "story_edit_transaction",
            "story_id": command["story_id"],
            "start_scalar": start,
            "end_scalar": end,
            "expected_before": expected_before,
            "replacement_text": canonical.text,
            "paragraph_inserted_ids": paragraph_ids,
            "paragraph_inserted_property_presets": [],
            "typing_format": None,
            "fragment_format_runs": [],
            "incoming_semantic_kinds": [],
        },
    }

    receipt = BrowserStoryRangeLoweringReceiptV1(
        protocol_version="chaptera.browser-story-range-lowering-receipt.v1",
        story_id=command["story_id"],
        start_scalar=start,
        end_scalar=end,
        before_text_sha256=hashlib.sha256(expected_before.encode("utf-8")).hexdigest(),
        canonical_replacement_sha256=canonical.text_sha256,
        inserted_paragraph_count=paragraph_count,
        browser_before_state_accepted=False,
    )
    return transaction_request, receipt
