#!/usr/bin/env python3
"""Atomic composition of independent Story-local text candidates V1.

Each local producer runs against the same immutable base project. The
coordinator validates isolation, composes only the affected Story states, and
returns one canonical multi-Story operation for RevisionKernel to commit.
"""

from __future__ import annotations

import copy
from typing import Literal

from story_edit_transaction_v1 import (
    StoryEditTransactionError,
    execute_story_edit_transaction_v1,
    story_edit_core_state_from_dict,
    story_edit_core_state_id_v1,
    validate_story_edit_transaction_operation_v1,
    validate_story_edit_transaction_request_v1,
)
from story_find_replace_v1 import (
    StoryFindReplaceError,
    execute_story_find_replace_v1,
    validate_story_find_replace_operation_v1,
    validate_story_find_replace_request_v1,
)


class MultiStoryTextTransactionError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


ProducerKindV1 = Literal["story_edit_transaction", "story_find_replace"]


def _reject(code: str, message: str) -> None:
    raise MultiStoryTextTransactionError(code, message)


def _producer_protocol(kind: ProducerKindV1) -> str:
    if kind == "story_edit_transaction":
        return "chaptera.story-edit-transaction-intent.v1"
    if kind == "story_find_replace":
        return "chaptera.story-find-replace-intent.v1"
    _reject("unsupported_local_producer", "unsupported Story-local text producer")


def _validate_local_intent(kind: ProducerKindV1, command: dict) -> None:
    envelope = {
        "protocol_version": _producer_protocol(kind),
        "command": command,
    }
    if kind == "story_edit_transaction":
        validate_story_edit_transaction_request_v1(envelope)
        return
    if kind == "story_find_replace":
        validate_story_find_replace_request_v1(envelope)
        return
    _reject("unsupported_local_producer", "unsupported Story-local text producer")


def _execute_local(
    *,
    base_project: dict,
    kind: ProducerKindV1,
    command: dict,
) -> tuple[dict, dict, list[dict]]:
    try:
        if kind == "story_edit_transaction":
            return execute_story_edit_transaction_v1(
                copy.deepcopy(base_project),
                copy.deepcopy(command),
            )
        if kind == "story_find_replace":
            return execute_story_find_replace_v1(
                copy.deepcopy(base_project),
                copy.deepcopy(command),
            )
    except StoryEditTransactionError as exc:
        _reject(exc.code, str(exc))
    except StoryFindReplaceError as exc:
        _reject(exc.code, str(exc))
    _reject("unsupported_local_producer", "unsupported Story-local text producer")


def _validate_local_operation(
    *,
    kind: ProducerKindV1,
    command: dict,
    operation: dict,
) -> None:
    if kind == "story_edit_transaction":
        validate_story_edit_transaction_operation_v1(command, operation)
        return
    if kind == "story_find_replace":
        validate_story_find_replace_operation_v1(command, operation)
        return
    raise ValueError("unsupported Story-local text producer")


def _assert_local_isolation(
    *,
    base_project: dict,
    resulting_project: dict,
    story_id: str,
) -> None:
    """A local candidate may change only its Story mirrors plus operations."""
    restored = copy.deepcopy(resulting_project)
    base_story_models = base_project.get("story_models")
    base_stories = base_project.get("stories", {})
    result_story_models = restored.get("story_models")
    result_stories = restored.get("stories")
    if (
        not isinstance(base_story_models, dict)
        or not isinstance(base_stories, dict)
        or not isinstance(result_story_models, dict)
        or not isinstance(result_stories, dict)
    ):
        _reject("invalid_story_state", "Story registries must be objects")
    if story_id not in base_story_models or story_id not in result_story_models:
        _reject("invalid_story_state", "local candidate Story model is missing")
    if story_id not in base_stories or story_id not in result_stories:
        _reject("invalid_story_state", "local candidate Story text mirror is missing")

    restored["story_models"] = copy.deepcopy(result_story_models)
    restored["story_models"][story_id] = copy.deepcopy(base_story_models[story_id])
    restored["stories"] = copy.deepcopy(result_stories)
    restored["stories"][story_id] = copy.deepcopy(base_stories[story_id])
    restored["operations"] = copy.deepcopy(base_project.get("operations", []))
    if restored != base_project:
        _reject(
            "local_candidate_cross_story_mutation",
            "Story-local candidate mutated project state outside its declared Story",
        )


def _receipt_from_local_operation(
    *,
    story_id: str,
    producer_kind: ProducerKindV1,
    operation: dict,
    direction: Literal["forward", "inverse"],
) -> dict:
    edits = []
    if producer_kind == "story_edit_transaction":
        start = operation["start_scalar"]
        end = operation["end_scalar"]
        inserted_end = start + len(operation["replacement_text"])
        forward = [{
            "edit_ordinal": 0,
            "source_ordinal": None,
            "base_start_scalar": start,
            "base_end_scalar": end,
            "final_start_scalar": start,
            "final_end_scalar": inserted_end,
        }]
    elif producer_kind == "story_find_replace":
        forward = [
            {
                "edit_ordinal": item["edit_ordinal"],
                "source_ordinal": item["snapshot_match_ordinal"],
                "base_start_scalar": item["base_start_scalar"],
                "base_end_scalar": item["base_end_scalar"],
                "final_start_scalar": item["inserted_start_scalar"],
                "final_end_scalar": item["inserted_end_scalar"],
            }
            for item in operation["normalized_edits"]
        ]
    else:
        _reject("unsupported_local_producer", "unsupported Story-local text producer")

    if direction == "forward":
        edits = forward
    else:
        edits = [
            {
                "edit_ordinal": item["edit_ordinal"],
                "source_ordinal": item["source_ordinal"],
                "base_start_scalar": item["final_start_scalar"],
                "base_end_scalar": item["final_end_scalar"],
                "final_start_scalar": item["base_start_scalar"],
                "final_end_scalar": item["base_end_scalar"],
            }
            for item in forward
        ]
        edits.sort(
            key=lambda item: (
                item["base_start_scalar"],
                item["base_end_scalar"],
                item["edit_ordinal"],
            )
        )

    return {
        "protocol_version": "chaptera.text-edit-receipt.v1",
        "story_id": story_id,
        "direction": direction,
        "normalized_edits": edits,
    }


def derive_multi_story_text_receipts_v1(
    operation: dict,
    *,
    direction: Literal["forward", "inverse"],
) -> dict[str, dict]:
    if (
        not isinstance(operation, dict)
        or operation.get("protocol_version")
        != "chaptera.multi-story-text-transaction.v1"
    ):
        _reject(
            "invalid_multi_story_operation",
            "MultiStoryTextTransactionV1 operation is required",
        )
    if direction not in {"forward", "inverse"}:
        _reject("invalid_receipt_direction", "receipt direction is invalid")
    out = {}
    for item in operation["local_operations"]:
        story_id = item["story_id"]
        out[story_id] = _receipt_from_local_operation(
            story_id=story_id,
            producer_kind=item["producer_kind"],
            operation=item["operation"],
            direction=direction,
        )
    return out


def restore_multi_story_before_v1(operation: dict) -> dict[str, dict]:
    if (
        not isinstance(operation, dict)
        or operation.get("protocol_version")
        != "chaptera.multi-story-text-transaction.v1"
    ):
        _reject(
            "invalid_multi_story_operation",
            "MultiStoryTextTransactionV1 operation is required",
        )
    return {
        item["story_id"]: copy.deepcopy(item["operation"]["inverse_state"])
        for item in operation["local_operations"]
    }


def replay_multi_story_after_v1(operation: dict) -> dict[str, dict]:
    if (
        not isinstance(operation, dict)
        or operation.get("protocol_version")
        != "chaptera.multi-story-text-transaction.v1"
    ):
        _reject(
            "invalid_multi_story_operation",
            "MultiStoryTextTransactionV1 operation is required",
        )
    return {
        item["story_id"]: copy.deepcopy(item["operation"]["after_state"])
        for item in operation["local_operations"]
    }


def execute_multi_story_text_transaction_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list[dict]]:
    if not isinstance(base_project, dict):
        _reject("invalid_story_state", "base project must be object")
    story_models = base_project.get("story_models")
    stories = base_project.get("stories")
    operations = base_project.get("operations", [])
    if (
        not isinstance(story_models, dict)
        or not isinstance(stories, dict)
        or not isinstance(operations, list)
    ):
        _reject("invalid_story_state", "project Story registries/operations are invalid")

    entries = command["entries"]
    local_results = []
    for entry in entries:
        story_id = entry["story_id"]
        producer_kind = entry["producer_kind"]
        local_command = entry["producer_command"]

        if local_command.get("story_id") != story_id:
            _reject(
                "local_story_mismatch",
                "Story-local producer command targets a different StoryId",
            )
        if producer_kind == "story_find_replace":
            if local_command.get("base_story_revision_id") != command["base_revision_id"]:
                _reject(
                    "stale_revision",
                    "StoryFindReplace local base revision differs from multi-Story base",
                )

        try:
            local_operation, local_project, local_consequences = _execute_local(
                base_project=base_project,
                kind=producer_kind,
                command=local_command,
            )
        except (KeyError, TypeError, ValueError) as exc:
            if isinstance(exc, MultiStoryTextTransactionError):
                raise
            _reject("local_candidate_rejected", str(exc))

        _assert_local_isolation(
            base_project=base_project,
            resulting_project=local_project,
            story_id=story_id,
        )
        _validate_local_operation(
            kind=producer_kind,
            command=local_command,
            operation=local_operation,
        )

        raw_after = local_project["story_models"][story_id]
        raw_before = base_project["story_models"][story_id]
        try:
            before_state = story_edit_core_state_from_dict(raw_before)
            after_state = story_edit_core_state_from_dict(raw_after)
        except StoryEditTransactionError as exc:
            _reject("local_candidate_rejected", str(exc))
        if before_state.story_id != story_id or after_state.story_id != story_id:
            _reject("local_story_mismatch", "local candidate state StoryId mismatch")

        local_results.append(
            {
                "story_id": story_id,
                "producer_kind": producer_kind,
                "producer_command": copy.deepcopy(local_command),
                "operation": copy.deepcopy(local_operation),
                "after_state": copy.deepcopy(raw_after),
                "after_text": local_project["stories"][story_id],
                "before_state_id": story_edit_core_state_id_v1(before_state),
                "after_state_id": story_edit_core_state_id_v1(after_state),
                "consequences": copy.deepcopy(local_consequences),
            }
        )

    local_results.sort(key=lambda item: item["story_id"])

    project = copy.deepcopy(base_project)
    project["story_models"] = copy.deepcopy(story_models)
    project["stories"] = copy.deepcopy(stories)
    for item in local_results:
        project["story_models"][item["story_id"]] = copy.deepcopy(item["after_state"])
        project["stories"][item["story_id"]] = item["after_text"]

    canonical_operation = {
        "protocol_version": "chaptera.multi-story-text-transaction.v1",
        "kind": "multi_story_text_transaction",
        "base_revision_id": command["base_revision_id"],
        "story_ids": [item["story_id"] for item in local_results],
        "local_operations": [
            {
                "story_id": item["story_id"],
                "producer_kind": item["producer_kind"],
                "before_state_id": item["before_state_id"],
                "after_state_id": item["after_state_id"],
                "operation": item["operation"],
            }
            for item in local_results
        ],
        "layout_invalidation_story_ids": [
            item["story_id"] for item in local_results
        ],
    }
    project["operations"] = list(operations) + [copy.deepcopy(canonical_operation)]

    receipts = derive_multi_story_text_receipts_v1(
        canonical_operation,
        direction="forward",
    )
    consequences = [
        {
            "key": "text.multi_story_reconciliation",
            "state": "supported",
            "note": None,
            "receipt_map": receipts,
        },
        {
            "key": "layout.reflow",
            "state": "unknown",
            "note": "union_of_affected_story_dependencies",
            "story_ids": canonical_operation["layout_invalidation_story_ids"],
        },
    ]
    return canonical_operation, project, consequences


def validate_multi_story_text_transaction_request_v1(request: dict) -> None:
    if (
        request.get("protocol_version")
        != "chaptera.multi-story-text-transaction-intent.v1"
    ):
        raise ValueError("MultiStoryTextTransactionV1 protocol_version is required")
    command = request.get("command")
    if (
        not isinstance(command, dict)
        or command.get("kind") != "multi_story_text_transaction"
        or set(command) != {"kind", "base_revision_id", "entries"}
    ):
        raise ValueError("MultiStoryTextTransactionV1 intent fields are not exact")
    if command["base_revision_id"] != request.get("base_revision_id"):
        raise ValueError("multi-Story command base_revision_id must equal request base")
    entries = command["entries"]
    if not isinstance(entries, list) or not entries or len(entries) > 1024:
        raise ValueError("multi-Story entries must be a non-empty bounded list")

    story_ids = []
    for index, entry in enumerate(entries):
        if (
            not isinstance(entry, dict)
            or set(entry) != {"story_id", "producer_kind", "producer_command"}
        ):
            raise ValueError(f"multi-Story entry[{index}] shape is invalid")
        story_id = entry["story_id"]
        producer_kind = entry["producer_kind"]
        producer_command = entry["producer_command"]
        if not isinstance(story_id, str) or not story_id:
            raise ValueError(f"multi-Story entry[{index}].story_id is required")
        if producer_kind not in {"story_edit_transaction", "story_find_replace"}:
            raise ValueError(f"multi-Story entry[{index}] producer kind is unsupported")
        if not isinstance(producer_command, dict):
            raise ValueError(f"multi-Story entry[{index}] producer_command must be object")
        if producer_command.get("story_id") != story_id:
            raise ValueError(f"multi-Story entry[{index}] StoryId differs from producer command")
        _validate_local_intent(producer_kind, producer_command)
        story_ids.append(story_id)

    if len(set(story_ids)) != len(story_ids):
        raise ValueError("multi-Story StoryIds must be unique")
    if story_ids != sorted(story_ids):
        raise ValueError("multi-Story entries must be normalized by StoryId")


def validate_multi_story_text_transaction_operation_v1(
    command: dict,
    operation: dict,
) -> None:
    expected = {
        "protocol_version",
        "kind",
        "base_revision_id",
        "story_ids",
        "local_operations",
        "layout_invalidation_story_ids",
    }
    if not isinstance(operation, dict) or set(operation) != expected:
        raise ValueError("canonical MultiStoryTextTransactionV1 fields are not exact")
    if (
        operation["protocol_version"]
        != "chaptera.multi-story-text-transaction.v1"
        or operation["kind"] != "multi_story_text_transaction"
    ):
        raise ValueError("canonical MultiStoryTextTransactionV1 protocol/kind mismatch")
    if operation["base_revision_id"] != command["base_revision_id"]:
        raise ValueError("canonical multi-Story base revision differs from intent")

    by_story = {entry["story_id"]: entry for entry in command["entries"]}
    expected_story_ids = sorted(by_story)
    if operation["story_ids"] != expected_story_ids:
        raise ValueError("canonical multi-Story StoryId ordering differs from intent")
    if operation["layout_invalidation_story_ids"] != expected_story_ids:
        raise ValueError("canonical multi-Story layout union differs from affected Stories")
    if len(operation["local_operations"]) != len(expected_story_ids):
        raise ValueError("canonical multi-Story local operation count mismatch")

    for story_id, item in zip(expected_story_ids, operation["local_operations"]):
        if (
            not isinstance(item, dict)
            or set(item)
            != {
                "story_id",
                "producer_kind",
                "before_state_id",
                "after_state_id",
                "operation",
            }
            or item["story_id"] != story_id
        ):
            raise ValueError("canonical multi-Story local entry shape/order mismatch")
        intent = by_story[story_id]
        if item["producer_kind"] != intent["producer_kind"]:
            raise ValueError("canonical multi-Story producer kind differs from intent")
        _validate_local_operation(
            kind=item["producer_kind"],
            command=intent["producer_command"],
            operation=item["operation"],
        )
        before = story_edit_core_state_from_dict(item["operation"]["inverse_state"])
        after = story_edit_core_state_from_dict(item["operation"]["after_state"])
        if item["before_state_id"] != story_edit_core_state_id_v1(before):
            raise ValueError("canonical multi-Story before state id mismatch")
        if item["after_state_id"] != story_edit_core_state_id_v1(after):
            raise ValueError("canonical multi-Story after state id mismatch")
