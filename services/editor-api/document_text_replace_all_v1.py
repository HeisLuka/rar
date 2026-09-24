#!/usr/bin/env python3
"""Atomic exhaustive publication-wide exact literal Replace All V1."""

from __future__ import annotations

import copy
import hashlib
import json

from document_text_find_v1 import (
    DocumentStorySearchInputV1,
    DocumentTextFindError,
    build_document_text_find_snapshot_v1,
)
from multi_story_text_transaction_v1 import (
    MultiStoryTextTransactionError,
    execute_multi_story_text_transaction_v1,
    validate_multi_story_text_transaction_operation_v1,
)
from story_edit_transaction_v1 import (
    StoryEditTransactionError,
    story_edit_core_state_from_dict,
)
from text_ingress_v1 import TextIngressError, normalize_external_text_v1


class DocumentTextReplaceAllError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


class DocumentTextReplaceAllNoOp(Exception):
    def __init__(self, *, query: str, replacement_text: str):
        super().__init__("document Replace All has zero matches")
        self.query = query
        self.replacement_text = replacement_text


def _reject(code: str, message: str) -> None:
    raise DocumentTextReplaceAllError(code, message)


def _snapshot_story_results(snapshot: dict) -> list[dict]:
    if not isinstance(snapshot, dict):
        _reject("invalid_document_find_snapshot", "document_find_snapshot must be object")
    expected = {
        "protocol_version",
        "policy_version",
        "revision_id",
        "query",
        "story_results",
    }
    if set(snapshot) != expected:
        _reject(
            "invalid_document_find_snapshot",
            "document_find_snapshot fields are not exact V1",
        )
    if (
        snapshot["protocol_version"] != "chaptera.document-text-find-snapshot.v1"
        or snapshot["policy_version"] != "chaptera.document-text-find-policy.v1"
    ):
        _reject("invalid_document_find_snapshot", "document find snapshot protocol mismatch")
    if not isinstance(snapshot["revision_id"], str) or not snapshot["revision_id"]:
        _reject("invalid_document_find_snapshot", "snapshot revision_id is required")
    if not isinstance(snapshot["query"], str) or not snapshot["query"]:
        _reject("invalid_document_find_snapshot", "snapshot query must be non-empty")
    results = snapshot["story_results"]
    if not isinstance(results, list):
        _reject("invalid_document_find_snapshot", "story_results must be list")

    story_ids = []
    for item in results:
        if not isinstance(item, dict) or set(item) != {
            "story_id",
            "status",
            "snapshot",
            "reason",
        }:
            _reject("invalid_document_find_snapshot", "story result shape is invalid")
        story_id = item["story_id"]
        if not isinstance(story_id, str) or not story_id:
            _reject("invalid_document_find_snapshot", "story result StoryId is required")
        if item["status"] not in {"searched", "unsupported"}:
            _reject("invalid_document_find_snapshot", "story result status is invalid")
        if item["status"] == "searched":
            local = item["snapshot"]
            if not isinstance(local, dict):
                _reject("invalid_document_find_snapshot", "searched Story requires local snapshot")
            if item["reason"] is not None:
                _reject("invalid_document_find_snapshot", "searched Story must not carry reason")
            if local.get("story_id") != story_id:
                _reject("invalid_document_find_snapshot", "local snapshot StoryId mismatch")
        else:
            if item["snapshot"] is not None:
                _reject("invalid_document_find_snapshot", "unsupported Story must not carry snapshot")
            if not isinstance(item["reason"], str) or not item["reason"]:
                _reject("invalid_document_find_snapshot", "unsupported Story requires reason")
        story_ids.append(story_id)

    if len(set(story_ids)) != len(story_ids):
        _reject("invalid_document_find_snapshot", "document snapshot StoryIds must be unique")
    if story_ids != sorted(story_ids):
        _reject("invalid_document_find_snapshot", "document snapshot StoryIds must be canonical")
    return results


def _local_matches(local_snapshot: dict) -> list[dict]:
    expected = {
        "protocol_version",
        "policy_version",
        "revision_id",
        "story_id",
        "query",
        "extent_start_scalar",
        "extent_end_scalar",
        "matches",
    }
    if not isinstance(local_snapshot, dict) or set(local_snapshot) != expected:
        _reject("invalid_document_find_snapshot", "local TextFindSnapshotV1 fields are invalid")
    if (
        local_snapshot["protocol_version"] != "chaptera.text-find-snapshot.v1"
        or local_snapshot["policy_version"] != "chaptera.text-find-policy.v1"
    ):
        _reject("invalid_document_find_snapshot", "local find snapshot protocol mismatch")
    matches = local_snapshot["matches"]
    if not isinstance(matches, list):
        _reject("invalid_document_find_snapshot", "local snapshot matches must be list")
    ordinals = []
    last_end = -1
    for item in matches:
        if not isinstance(item, dict) or set(item) != {
            "ordinal",
            "start_scalar",
            "end_scalar",
            "matched_text",
            "matched_text_sha256",
        }:
            _reject("invalid_document_find_snapshot", "local find match shape is invalid")
        ordinal = item["ordinal"]
        start = item["start_scalar"]
        end = item["end_scalar"]
        if (
            not isinstance(ordinal, int)
            or isinstance(ordinal, bool)
            or not isinstance(start, int)
            or isinstance(start, bool)
            or not isinstance(end, int)
            or isinstance(end, bool)
            or start < 0
            or end <= start
        ):
            _reject("invalid_document_find_snapshot", "local match coordinates are invalid")
        if start < last_end:
            _reject("invalid_document_find_snapshot", "local matches must be non-overlapping/orderly")
        last_end = end
        ordinals.append(ordinal)
    if ordinals != list(range(len(matches))):
        _reject("invalid_document_find_snapshot", "local match ordinals must be snapshot-local canonical")
    return matches


def _canonical_replacement(external_text: str) -> str:
    try:
        return normalize_external_text_v1(external_text).text
    except TextIngressError as exc:
        _reject("invalid_replacement", str(exc))


def _story_registry_from_project(base_project: dict) -> tuple[DocumentStorySearchInputV1, ...]:
    story_models = base_project.get("story_models")
    stories = base_project.get("stories")
    if not isinstance(story_models, dict) or not isinstance(stories, dict):
        _reject("invalid_story_state", "project Story registries must be objects")
    if set(story_models) != set(stories):
        _reject(
            "invalid_story_state",
            "Story model registry and Story text mirror must have identical scope",
        )
    out = []
    for story_id in sorted(story_models):
        raw = story_models[story_id]
        try:
            state = story_edit_core_state_from_dict(raw)
        except StoryEditTransactionError as exc:
            _reject("invalid_story_state", str(exc))
        text = state.paragraph_state.story_text
        if stories[story_id] != text:
            _reject("invalid_story_state", "Story text mirror differs from canonical Story model")
        out.append(
            DocumentStorySearchInputV1(
                story_id=story_id,
                story_text=text,
                provenance=state.provenance,
            )
        )
    return tuple(out)


def _paragraph_id_map(
    entries: list[dict],
) -> dict[tuple[str, int], tuple[str, ...]]:
    out = {}
    all_ids = set()
    for item in entries:
        key = (item["story_id"], item["match_ordinal"])
        if key in out:
            _reject("invalid_paragraph_ids", "paragraph id assignment key is duplicated")
        ids = tuple(item["paragraph_ids"])
        for paragraph_id in ids:
            if paragraph_id in all_ids:
                _reject(
                    "invalid_paragraph_ids",
                    "preallocated ParagraphIds must be unique across document command",
                )
            all_ids.add(paragraph_id)
        out[key] = ids
    return out


def _format_generation_id(
    *,
    base_revision_id: str,
    story_id: str,
    query: str,
    replacement_text: str,
) -> str:
    payload = json.dumps(
        {
            "protocol_version": "chaptera.document-replace-format-generation.v1",
            "base_revision_id": base_revision_id,
            "story_id": story_id,
            "query": query,
            "replacement_text": replacement_text,
        },
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def _build_multi_story_command(
    *,
    base_revision_id: str,
    snapshot: dict,
    replacement_text: str,
    paragraph_ids_by_match: list[dict],
) -> tuple[dict, int]:
    results = _snapshot_story_results(snapshot)
    id_map = _paragraph_id_map(paragraph_ids_by_match)
    replacement_boundary_count = replacement_text.count("\r")
    matched_keys = set()
    entries = []
    total_match_count = 0

    for item in results:
        if item["status"] != "searched":
            continue
        local_snapshot = item["snapshot"]
        assert isinstance(local_snapshot, dict)
        matches = _local_matches(local_snapshot)
        if not matches:
            continue
        story_id = item["story_id"]
        total_match_count += len(matches)
        paragraph_entries = []
        for match in matches:
            ordinal = match["ordinal"]
            key = (story_id, ordinal)
            matched_keys.add(key)
            ids = id_map.get(key, ())
            if len(ids) != replacement_boundary_count:
                _reject(
                    "invalid_paragraph_ids",
                    "every replaced match requires one preallocated ParagraphId per replacement U+000D",
                )
            paragraph_entries.append(
                {
                    "match_ordinal": ordinal,
                    "paragraph_ids": list(ids),
                }
            )
        local_command = {
            "kind": "story_find_replace",
            "story_id": story_id,
            "base_story_revision_id": base_revision_id,
            "find_snapshot": copy.deepcopy(local_snapshot),
            "selected_match_ordinals": [m["ordinal"] for m in matches],
            # Document ingress already normalized the spelling. Story-local
            # producer receives that canonical spelling; its ingress call is
            # idempotent validation of the same canonical scalar sequence.
            "external_replacement_text": replacement_text,
            "paragraph_ids_by_match": paragraph_entries,
            "format_generation_id": _format_generation_id(
                base_revision_id=base_revision_id,
                story_id=story_id,
                query=snapshot["query"],
                replacement_text=replacement_text,
            ),
        }
        entries.append(
            {
                "story_id": story_id,
                "producer_kind": "story_find_replace",
                "producer_command": local_command,
            }
        )

    extra_keys = set(id_map) - matched_keys
    if extra_keys:
        _reject(
            "invalid_paragraph_ids",
            "paragraph id assignments may target only selected document snapshot matches",
        )

    return (
        {
            "kind": "multi_story_text_transaction",
            "base_revision_id": base_revision_id,
            "entries": entries,
        },
        total_match_count,
    )


def execute_document_text_replace_all_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list[dict]]:
    snapshot = command["document_find_snapshot"]
    results = _snapshot_story_results(snapshot)
    base_revision_id = command["base_revision_id"]
    if snapshot["revision_id"] != base_revision_id:
        _reject(
            "document_find_snapshot_stale",
            "document find snapshot revision differs from Replace All base revision",
        )

    story_registry = _story_registry_from_project(base_project)
    current_story_ids = {item.story_id for item in story_registry}
    snapshot_story_ids = {item["story_id"] for item in results}
    if snapshot_story_ids != current_story_ids:
        _reject(
            "document_search_incomplete",
            "document snapshot does not account for every canonical Story",
        )

    if any(item["status"] != "searched" for item in results):
        _reject(
            "document_search_incomplete",
            "exhaustive document Replace All requires every Story search domain",
        )

    current = build_document_text_find_snapshot_v1(
        revision_id=base_revision_id,
        stories=story_registry,
        external_query=snapshot["query"],
    )
    if not current.exhaustive_searchable:
        _reject(
            "document_search_incomplete",
            "current document contains unsupported/unknown Story search domain",
        )
    if current.to_dict() != snapshot:
        _reject(
            "document_find_snapshot_stale",
            "document find snapshot differs from current canonical revision",
        )

    replacement = _canonical_replacement(command["external_replacement_text"])
    multi_command, total_match_count = _build_multi_story_command(
        base_revision_id=base_revision_id,
        snapshot=snapshot,
        replacement_text=replacement,
        paragraph_ids_by_match=command["paragraph_ids_by_match"],
    )
    if total_match_count == 0:
        raise DocumentTextReplaceAllNoOp(
            query=snapshot["query"],
            replacement_text=replacement,
        )

    try:
        multi_operation, project, consequences = (
            execute_multi_story_text_transaction_v1(
                copy.deepcopy(base_project),
                multi_command,
            )
        )
    except MultiStoryTextTransactionError as exc:
        _reject(exc.code, str(exc))

    affected_story_ids = multi_operation["story_ids"]
    operation = {
        "protocol_version": "chaptera.document-text-replace-all.v1",
        "kind": "document_text_replace_all",
        "base_revision_id": base_revision_id,
        "snapshot_revision_id": snapshot["revision_id"],
        "query": snapshot["query"],
        "replacement_text": replacement,
        "total_match_count": total_match_count,
        "affected_story_ids": affected_story_ids,
        "multi_story_operation": multi_operation,
        "document_snapshot_staled": True,
    }

    base_operations = base_project.get("operations", [])
    if not isinstance(base_operations, list):
        _reject("invalid_story_state", "project operations must be list")
    project["operations"] = list(base_operations) + [copy.deepcopy(operation)]

    out_consequences = list(consequences) + [
        {
            "key": "document.find_snapshot",
            "state": "stale",
            "note": "regenerate_after_document_replace_all",
        }
    ]
    return operation, project, out_consequences


def validate_document_text_replace_all_request_v1(request: dict) -> None:
    if (
        request.get("protocol_version")
        != "chaptera.document-text-replace-all-intent.v1"
    ):
        raise ValueError("DocumentTextReplaceAllV1 protocol_version is required")
    command = request.get("command")
    if (
        not isinstance(command, dict)
        or command.get("kind") != "document_text_replace_all"
        or set(command)
        != {
            "kind",
            "base_revision_id",
            "document_find_snapshot",
            "external_replacement_text",
            "paragraph_ids_by_match",
        }
    ):
        raise ValueError("DocumentTextReplaceAllV1 intent fields are not exact")
    if command["base_revision_id"] != request.get("base_revision_id"):
        raise ValueError("document Replace All command base revision must equal request base")
    if not isinstance(command["external_replacement_text"], str):
        raise ValueError("external_replacement_text must be string")

    results = _snapshot_story_results(command["document_find_snapshot"])
    if command["document_find_snapshot"]["revision_id"] != command["base_revision_id"]:
        raise ValueError("document snapshot revision must equal request base revision")

    assignments = command["paragraph_ids_by_match"]
    if not isinstance(assignments, list):
        raise ValueError("paragraph_ids_by_match must be list")
    seen_keys = set()
    seen_ids = set()
    for index, item in enumerate(assignments):
        if (
            not isinstance(item, dict)
            or set(item) != {"story_id", "match_ordinal", "paragraph_ids"}
        ):
            raise ValueError(f"paragraph id assignment[{index}] shape is invalid")
        story_id = item["story_id"]
        ordinal = item["match_ordinal"]
        ids = item["paragraph_ids"]
        if (
            not isinstance(story_id, str)
            or not story_id
            or not isinstance(ordinal, int)
            or isinstance(ordinal, bool)
            or ordinal < 0
            or not isinstance(ids, list)
            or any(not isinstance(v, str) or not v for v in ids)
            or len(set(ids)) != len(ids)
        ):
            raise ValueError(f"paragraph id assignment[{index}] is invalid")
        key = (story_id, ordinal)
        if key in seen_keys:
            raise ValueError("paragraph id assignment keys must be unique")
        seen_keys.add(key)
        for value in ids:
            if value in seen_ids:
                raise ValueError("preallocated ParagraphIds must be globally unique in command")
            seen_ids.add(value)

    story_ids = [item["story_id"] for item in results]
    if story_ids != sorted(story_ids):
        raise ValueError("document snapshot StoryIds must be canonical")


def validate_document_text_replace_all_operation_v1(
    command: dict,
    operation: dict,
) -> None:
    expected = {
        "protocol_version",
        "kind",
        "base_revision_id",
        "snapshot_revision_id",
        "query",
        "replacement_text",
        "total_match_count",
        "affected_story_ids",
        "multi_story_operation",
        "document_snapshot_staled",
    }
    if not isinstance(operation, dict) or set(operation) != expected:
        raise ValueError("canonical DocumentTextReplaceAllV1 fields are not exact")
    if (
        operation["protocol_version"]
        != "chaptera.document-text-replace-all.v1"
        or operation["kind"] != "document_text_replace_all"
    ):
        raise ValueError("canonical DocumentTextReplaceAllV1 protocol/kind mismatch")
    snapshot = command["document_find_snapshot"]
    replacement = _canonical_replacement(command["external_replacement_text"])
    multi_command, total = _build_multi_story_command(
        base_revision_id=command["base_revision_id"],
        snapshot=snapshot,
        replacement_text=replacement,
        paragraph_ids_by_match=command["paragraph_ids_by_match"],
    )
    if operation["base_revision_id"] != command["base_revision_id"]:
        raise ValueError("canonical document Replace All base revision differs from intent")
    if operation["snapshot_revision_id"] != snapshot["revision_id"]:
        raise ValueError("canonical document Replace All snapshot revision mismatch")
    if operation["query"] != snapshot["query"]:
        raise ValueError("canonical document Replace All query mismatch")
    if operation["replacement_text"] != replacement:
        raise ValueError("canonical document Replace All replacement mismatch")
    if operation["total_match_count"] != total:
        raise ValueError("canonical document Replace All match count mismatch")
    if operation["affected_story_ids"] != sorted(
        entry["story_id"] for entry in multi_command["entries"]
    ):
        raise ValueError("canonical document Replace All affected StoryIds mismatch")
    validate_multi_story_text_transaction_operation_v1(
        multi_command,
        operation["multi_story_operation"],
    )
    if operation["document_snapshot_staled"] is not True:
        raise ValueError("successful document Replace All must stale input snapshot")
