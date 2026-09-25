#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
CORPUS = TOOLS / "corpus"
for path in (TOOLS, CORPUS):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

from cfb_physical_diff import minimal  # noqa: E402
from operation_blast_radius_v1 import BlastRadiusError, SCHEMA, build_receipt  # noqa: E402


def mutate_directory_state(data: bytes, value: int) -> bytes:
    out = bytearray(data)
    out[736] = value
    return bytes(out)


def mutate_stream_payload(data: bytes, value: int) -> bytes:
    out = bytearray(data)
    out[1024] = value
    return bytes(out)


class OperationBlastRadiusTests(unittest.TestCase):
    def evidence(self) -> dict:
        return {
            "producer": {"name": "synthetic", "version": "v1"},
            "operation": {"kind": "MoveNode", "operation_id": "op-1"},
            "requested_streams": ["dir:1:Data"],
            "expected_derived_streams": [],
            "requested_records": [{"family": "Escher", "id": "shape-1"}],
            "expected_derived_records": [],
            "requested_entities": [{"kind": "node", "id": "shape-1"}],
            "expected_derived_entities": [],
            "arms": {
                "source": {
                    "parser": {"accepted": True, "diagnostic_codes": []},
                    "records": [{"family": "Escher", "id": "shape-1", "sha256": "1" * 64}],
                    "semantic_entities": [{"kind": "node", "id": "shape-1", "sha256": "a" * 64}],
                },
                "control": {
                    "parser": {"accepted": True, "diagnostic_codes": ["save.normalized"]},
                    "records": [{"family": "Escher", "id": "shape-1", "sha256": "1" * 64}],
                    "semantic_entities": [{"kind": "node", "id": "shape-1", "sha256": "a" * 64}],
                },
                "mutation": {
                    "parser": {"accepted": True, "diagnostic_codes": []},
                    "records": [{"family": "Escher", "id": "shape-1", "sha256": "2" * 64}],
                    "semantic_entities": [{"kind": "node", "id": "shape-1", "sha256": "b" * 64}],
                },
            },
        }

    def test_subtracts_noop_normalization_and_marks_requested_delta(self) -> None:
        source = minimal(0)
        control = mutate_directory_state(source, 1)
        mutation = mutate_stream_payload(control, 99)
        receipt = build_receipt(source, control, mutation, evidence=self.evidence())

        self.assertEqual(receipt["schema_version"], SCHEMA)
        self.assertEqual(
            receipt["cfb"]["source_control_stream_delta"],
            [],
            "directory-only save normalization must not be misreported as logical stream content",
        )
        stream_deltas = receipt["cfb"]["control_mutation_stream_delta"]
        self.assertEqual(len(stream_deltas), 1)
        self.assertEqual(stream_deltas[0]["stream_id"], "dir:1:Data")
        self.assertEqual(stream_deltas[0]["classification"], "requested_semantic")
        self.assertEqual(
            receipt["parsed_record_family_delta"][0]["classification"],
            "requested_semantic",
        )
        self.assertEqual(
            receipt["semantic_graph_delta"][0]["classification"],
            "requested_semantic",
        )
        self.assertTrue(receipt["invariants"]["matched_noop_control_used"])
        self.assertFalse(receipt["invariants"]["native_pub_writer_capability_granted"])

    def test_unknown_mutation_stays_unexplained(self) -> None:
        source = minimal(0)
        control = mutate_directory_state(source, 1)
        mutation = mutate_stream_payload(control, 42)
        evidence = self.evidence()
        evidence["requested_streams"] = []
        evidence["requested_records"] = []
        evidence["requested_entities"] = []
        receipt = build_receipt(source, control, mutation, evidence=evidence)

        self.assertEqual(
            receipt["cfb"]["control_mutation_stream_delta"][0]["classification"],
            "unexplained_collateral",
        )
        self.assertGreater(receipt["classification_counts"]["unexplained_collateral"], 0)

    def test_second_save_convergence_is_explicit(self) -> None:
        source = minimal(0)
        control = mutate_directory_state(source, 1)
        mutation = mutate_stream_payload(control, 42)
        receipt = build_receipt(
            source,
            control,
            mutation,
            evidence=self.evidence(),
            second_save=mutation,
        )
        self.assertEqual(receipt["second_save_convergence"]["status"], "converged")
        self.assertEqual(receipt["second_save_convergence"]["changed_stream_count"], 0)

    def test_missing_structured_fields_fail_closed(self) -> None:
        source = minimal(0)
        control = mutate_directory_state(source, 1)
        mutation = mutate_stream_payload(control, 42)
        evidence = self.evidence()
        evidence["arms"]["mutation"]["records"] = [{"family": "Escher", "id": "shape-1"}]
        with self.assertRaises(BlastRadiusError):
            build_receipt(source, control, mutation, evidence=evidence)

    def test_receipt_is_deterministic(self) -> None:
        source = minimal(0)
        control = mutate_directory_state(source, 1)
        mutation = mutate_stream_payload(control, 99)
        left = build_receipt(source, control, mutation, evidence=self.evidence())
        right = build_receipt(source, control, mutation, evidence=self.evidence())
        self.assertEqual(
            json.dumps(left, sort_keys=True, separators=(",", ":")),
            json.dumps(right, sort_keys=True, separators=(",", ":")),
        )


if __name__ == "__main__":
    unittest.main()
