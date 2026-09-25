#!/usr/bin/env python3
from __future__ import annotations

import copy
import hashlib
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from validate_movenode_diagnostic_receipt import ValidationError, validate  # noqa: E402


SOURCE = "a" * 64
CONTROL = "b" * 64
MUTATION = "c" * 64
NODE = "node-0001"


def blast_fixture() -> tuple[dict, bytes]:
    blast = {
        "schema_version": "chaptera.operation-blast-radius.v1",
        "operation": {"kind": "MoveNode", "node_id": NODE, "operation_id": "op-1"},
        "artifacts": {
            "source": {"sha256": SOURCE, "byte_len": 1000},
            "control": {"sha256": CONTROL, "byte_len": 1000},
            "mutation": {"sha256": MUTATION, "byte_len": 1000},
        },
        "cfb": {
            "source_control_topology_delta": [],
            "control_mutation_topology_delta": [],
            "source_control_stream_delta": [],
            "control_mutation_stream_delta": [],
            "control_mutation_byte_ranges": [],
        },
        "parsed_record_family_delta": [],
        "semantic_graph_delta": [],
        "parser_outcomes": {
            "source": {"status": "accepted", "diagnostic_codes": []},
            "control": {"status": "accepted", "diagnostic_codes": []},
            "mutation": {"status": "accepted", "diagnostic_codes": []},
        },
        "second_save_convergence": {
            "status": "converged",
            "artifact": {"sha256": MUTATION, "byte_len": 1000},
            "changed_stream_count": 0,
            "different_byte_count": 0,
        },
        "classification_counts": {
            "expected_derived": 0,
            "requested_semantic": 1,
            "save_normalization": 1,
            "unavailable": 0,
            "unexplained_collateral": 0,
        },
        "invariants": {
            "raw_byte_inequality_is_not_semantic_evidence": True,
            "matched_noop_control_used": True,
            "unexplained_collateral_preserved": True,
            "public_receipt_contains_raw_document_bytes": False,
            "native_pub_writer_capability_granted": False,
        },
    }
    raw = (json.dumps(blast, sort_keys=True, separators=(",", ":")) + "\n").encode()
    return blast, raw


def receipt_fixture(blast_raw: bytes) -> dict:
    return {
        "receipt_version": "chaptera.movenode-diagnostic-receipt.v1",
        "source_sha256": SOURCE,
        "chaptera": {
            "operation_kind": "MoveNode",
            "node_id": NODE,
            "scene_instance_id": "scene-instance-1",
            "admission": "direct_page_local",
            "base_revision_id": "revision-1",
            "result_revision_id": "revision-2",
            "before": {"x": 100000, "y": 200000, "width": 300000, "height": 400000},
            "after": {"x": 112700, "y": 200000, "width": 300000, "height": 400000},
        },
        "native_experiment": {
            "publisher_version": "16.0",
            "publisher_build": "12527.22145",
            "shape_identity": "page-1/shape-7",
            "axis": "x",
            "emu_per_point": 12700,
            "tolerance_emu": 0,
            "control": {
                "baseline_source_sha256": SOURCE,
                "first_save_sha256": CONTROL,
                "second_save_sha256": None,
                "before": {"left": "10", "top": "20", "width": "30", "height": "40"},
                "after": {"left": "10", "top": "20", "width": "30", "height": "40"},
                "parser_accepted": True,
                "publisher_reopen_accepted": True,
            },
            "mutation": {
                "baseline_source_sha256": SOURCE,
                "first_save_sha256": MUTATION,
                "second_save_sha256": MUTATION,
                "before": {"left": "10", "top": "20", "width": "30", "height": "40"},
                "after": {"left": "11", "top": "20", "width": "30", "height": "40"},
                "parser_accepted": True,
                "publisher_reopen_accepted": True,
            },
        },
        "blast_radius": {
            "receipt_sha256": hashlib.sha256(blast_raw).hexdigest(),
            "schema_version": "chaptera.operation-blast-radius.v1",
            "source_sha256": SOURCE,
            "control_sha256": CONTROL,
            "mutation_sha256": MUTATION,
            "second_save_sha256": MUTATION,
        },
        "invariants": {
            "exactly_one_durable_movenode": True,
            "native_pub_writer_capability_granted": False,
        },
    }


class MoveNodeDiagnosticValidatorTests(unittest.TestCase):
    def test_valid_x_move_binds_all_three_evidence_layers(self) -> None:
        blast, raw = blast_fixture()
        result = validate(receipt_fixture(raw), blast, raw)
        self.assertTrue(result["valid"])
        self.assertEqual(result["chaptera_delta_emu"], {"x": 12700, "y": 0})
        self.assertEqual(result["native_delta_points"], {"x": "1", "y": "0"})
        self.assertEqual(result["second_save_convergence"], "converged")

    def test_native_delta_must_match_chaptera_emu_delta(self) -> None:
        blast, raw = blast_fixture()
        receipt = receipt_fixture(raw)
        receipt["native_experiment"]["mutation"]["after"]["left"] = "11.5"
        with self.assertRaises(ValidationError):
            validate(receipt, blast, raw)

    def test_matched_control_must_be_geometry_noop(self) -> None:
        blast, raw = blast_fixture()
        receipt = receipt_fixture(raw)
        receipt["native_experiment"]["control"]["after"]["top"] = "20.1"
        with self.assertRaises(ValidationError):
            validate(receipt, blast, raw)

    def test_blast_receipt_hash_is_content_bound(self) -> None:
        blast, raw = blast_fixture()
        receipt = receipt_fixture(raw)
        mutated_raw = raw + b" "
        with self.assertRaises(ValidationError):
            validate(receipt, blast, mutated_raw)

    def test_projected_instance_cannot_pass_schema(self) -> None:
        blast, raw = blast_fixture()
        receipt = receipt_fixture(raw)
        receipt["chaptera"]["admission"] = "inherited_master"
        with self.assertRaises(Exception):
            validate(receipt, blast, raw)

    def test_native_reopen_rejection_is_fatal(self) -> None:
        blast, raw = blast_fixture()
        receipt = receipt_fixture(raw)
        receipt["native_experiment"]["mutation"]["publisher_reopen_accepted"] = False
        with self.assertRaises(ValidationError):
            validate(receipt, blast, raw)

    def test_unexplained_collateral_is_reported_not_hidden(self) -> None:
        blast, raw = blast_fixture()
        blast["classification_counts"]["unexplained_collateral"] = 3
        raw = (json.dumps(blast, sort_keys=True, separators=(",", ":")) + "\n").encode()
        receipt = receipt_fixture(raw)
        result = validate(receipt, blast, raw)
        self.assertEqual(result["unexplained_collateral_count"], 3)


if __name__ == "__main__":
    unittest.main()
