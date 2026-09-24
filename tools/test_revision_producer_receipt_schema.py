#!/usr/bin/env python3
import copy
import unittest

from validate_revision_producer_receipt import (
    hash_id,
    project_hash,
    revision_id,
    state_id,
    validate_schema,
    validate_semantics,
)

DOCUMENT_ID = "10000000-0000-4000-8000-000000000001"
NODE_ID = "30000000-0000-4000-8000-000000000001"
OP_ID = "90000000-0000-4000-8000-000000000001"
SOURCE_HASH = "a" * 64


def valid_receipt():
    baseline_project = {
        "schema_version": "pub-editor-v0.2",
        "source_hash": SOURCE_HASH,
        "operations": [],
    }
    baseline_state = state_id(DOCUMENT_ID, SOURCE_HASH, baseline_project)
    baseline_revision = revision_id(
        DOCUMENT_ID,
        SOURCE_HASH,
        None,
        baseline_state,
        "baseline",
        None,
    )
    request = {
        "protocol_version": "chaptera.commit-request.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": baseline_revision,
        "client_operation_id": OP_ID,
        "command": {
            "kind": "move_node_to",
            "node_id": NODE_ID,
            "x_emu": 127000,
            "y_emu": 254000,
        },
    }
    operation = {
        "kind": "move_node",
        "node_id": NODE_ID,
        "before": {"x": 0, "y": 0, "width": 1828800, "height": 914400},
        "after": {"x": 127000, "y": 254000, "width": 1828800, "height": 914400},
    }
    resulting_project = {
        "schema_version": "pub-editor-v0.4",
        "source_hash": SOURCE_HASH,
        "operations": [copy.deepcopy(operation)],
    }
    accepted_state = state_id(DOCUMENT_ID, SOURCE_HASH, resulting_project)
    accepted_revision = revision_id(
        DOCUMENT_ID,
        SOURCE_HASH,
        baseline_revision,
        accepted_state,
        "commit",
        hash_id(operation),
    )
    accepted = {
        "protocol_version": "chaptera.commit-accepted.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": baseline_revision,
        "revision_id": accepted_revision,
        "state_id": accepted_state,
        "client_operation_id": OP_ID,
        "canonical_operation": copy.deepcopy(operation),
        "project_schema_version": "pub-editor-v0.4",
        "consequences": [{
            "key": "node.geometry.position",
            "state": "supported",
            "note": None,
        }],
        "scene_refresh": "full_snapshot",
    }
    return {
        "receipt_version": "chaptera.revision-producer-receipt.v1",
        "producer": {
            "implementation": "chaptera-canonical-editor",
            "commit_or_build": "deadbeef",
            "core_integration": True,
        },
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "baseline": {
            "project": baseline_project,
            "project_hash": project_hash(baseline_project),
            "state_id": baseline_state,
            "revision_id": baseline_revision,
        },
        "request": request,
        "accepted": accepted,
        "resulting_project": resulting_project,
        "replayed_project": copy.deepcopy(resulting_project),
        "probes": {
            "stale_base": {
                "code": "stale_revision",
                "before_revision_id": accepted_revision,
                "after_revision_id": accepted_revision,
                "executor_calls_delta": 0,
            },
            "exact_retry": {
                "same_result": True,
                "executor_calls_total": 1,
            },
            "idempotency_conflict": {
                "code": "idempotency_conflict",
                "before_revision_id": accepted_revision,
                "after_revision_id": accepted_revision,
                "executor_calls_delta": 0,
            },
            "source_identity": {
                "before": SOURCE_HASH,
                "after": SOURCE_HASH,
                "replay": SOURCE_HASH,
            },
        },
    }


class RevisionProducerReceiptSchemaTests(unittest.TestCase):
    def test_current_move_node_receipt_is_admitted(self):
        receipt = valid_receipt()
        validate_schema(receipt)
        validate_semantics(receipt)

    def test_unknown_request_private_field_fails_closed(self):
        receipt = valid_receipt()
        receipt["request"]["private_checkout_path"] = "/home/private/canonical-core"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_unknown_project_field_fails_closed(self):
        receipt = valid_receipt()
        receipt["resulting_project"]["parser_carrier"] = {"raw": "secret"}
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_raw_asset_bytes_fail_closed(self):
        receipt = valid_receipt()
        receipt["resulting_project"]["assets"] = [{
            "sha256": "b" * 64,
            "mime": "image/png",
            "byte_len": 8,
            "bytes": "iVBORw0K",
        }]
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_unknown_canonical_operation_field_fails_closed(self):
        receipt = valid_receipt()
        receipt["accepted"]["canonical_operation"]["raw_source_offset"] = 123
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_wrong_stale_probe_code_fails_closed(self):
        receipt = valid_receipt()
        receipt["probes"]["stale_base"]["code"] = "some_failure"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_wrong_conflict_probe_code_fails_closed(self):
        receipt = valid_receipt()
        receipt["probes"]["idempotency_conflict"]["code"] = "some_conflict"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)


if __name__ == "__main__":
    unittest.main()
