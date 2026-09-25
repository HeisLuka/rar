#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
CORPUS = TOOLS / "corpus"
for path in (TOOLS, CORPUS):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

from build_movenode_diagnostic_receipt import BuilderError, build  # noqa: E402
from cfb_physical_diff import minimal  # noqa: E402


NODE_ID = "00112233-4455-6677-8899-aabbccddeeff"
INSTANCE_ID = "scene-instance-test"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def directory_normalized(source: bytes) -> bytes:
    out = bytearray(source)
    out[736] = 1
    return bytes(out)


def stream_mutated(control: bytes) -> bytes:
    out = bytearray(control)
    out[1024] = 99
    return bytes(out)


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def agent_trace(path: Path, source_hash: str, *, dx_emu: int = 12700) -> None:
    lines = [
        {
            "protocol_version": "chaptera.agent-control.v1",
            "message_type": "trace",
            "event_index": 1,
            "request_id": "r-move",
            "command": "edit.apply",
            "event_kind": "intent",
            "source_hash": source_hash,
            "payload": {"operation_kind": "move_node"},
        },
        {
            "protocol_version": "chaptera.agent-control.v1",
            "message_type": "trace",
            "event_index": 2,
            "request_id": "r-move",
            "command": "edit.apply",
            "event_kind": "durable_commit",
            "source_hash": source_hash,
            "payload": {
                "operation_id": "sha256:" + "1" * 64,
                "operation": {
                    "kind": "move_node",
                    "node_id": NODE_ID,
                    "before": {
                        "x": 100000,
                        "y": 200000,
                        "width": 300000,
                        "height": 400000,
                    },
                    "after": {
                        "x": 100000 + dx_emu,
                        "y": 200000,
                        "width": 300000,
                        "height": 400000,
                    },
                },
                "semantic_delta": {
                    "story_changed": False,
                    "geometry_changed": True,
                },
                "scene_delta": {
                    "geometry_changed": True,
                    "instance_id": INSTANCE_ID,
                    "origin_node_id": NODE_ID,
                    "before": {
                        "x": 100000,
                        "y": 200000,
                        "width": 300000,
                        "height": 400000,
                    },
                    "after": {
                        "x": 100000 + dx_emu,
                        "y": 200000,
                        "width": 300000,
                        "height": 400000,
                    },
                    "geometry_sync_policy": "apply_authored_origin_geometry",
                },
                "diagnostics_delta": {"added": [], "cleared": []},
                "before_state_id": "sha256:" + "2" * 64,
                "after_state_id": "sha256:" + "3" * 64,
            },
        },
    ]
    path.write_text(
        "\n".join(json.dumps(item, separators=(",", ":")) for item in lines) + "\n",
        encoding="utf-8",
    )


def native_manifest(
    path: Path,
    *,
    source_path: Path,
    control_first: Path,
    control_second: Path,
    mutation_first: Path,
    mutation_second: Path,
    source_hash: str,
) -> None:
    def geom(left: str) -> dict:
        return {"left": left, "top": "20", "width": "30", "height": "40"}

    common = {
        "publisher_version": "16.0",
        "publisher_build": "12527.22145",
        "baseline_source_sha256": source_hash,
        "working_copy_sha256_before_open": source_hash,
        "page_id": 12345,
        "oracle_tag": {"name": "PUB_ORACLE_ID", "value": "SHAPE_A"},
        "shape_identity": {
            "shape_id_before": 7,
            "shape_id_reopen": 7,
            "shape_name_before": "Rectangle 1",
            "shape_name_reopen": "Rectangle 1",
        },
    }
    manifest = {
        "schema": "chaptera.publisher-movenode-causal-local.v1",
        "source": {
            "sha256": source_hash,
            "byte_len": source_path.stat().st_size,
            "unchanged": True,
        },
        "publisher": {"version": "16.0", "build": "12527.22145"},
        "target": {
            "page_id": 12345,
            "tag_name": "PUB_ORACLE_ID",
            "tag_value": "SHAPE_A",
        },
        "delta_points": "1",
        "emu_per_point": 12700,
        "arms": [
            {
                **common,
                "name": "control",
                "mode": "control",
                "before": geom("10"),
                "after_set": geom("10"),
                "reopen": geom("10"),
                "first_save_sha256": sha(control_first.read_bytes()),
                "second_save_sha256": sha(control_second.read_bytes()),
                "first_save_path": str(control_first),
                "second_save_path": str(control_second),
            },
            {
                **common,
                "name": "move-x",
                "mode": "x",
                "before": geom("10"),
                "after_set": geom("11"),
                "reopen": geom("11"),
                "first_save_sha256": sha(mutation_first.read_bytes()),
                "second_save_sha256": sha(mutation_second.read_bytes()),
                "first_save_path": str(mutation_first),
                "second_save_path": str(mutation_second),
            },
            {
                **common,
                "name": "move-y",
                "mode": "y",
                "before": geom("10"),
                "after_set": {
                    "left": "10",
                    "top": "21",
                    "width": "30",
                    "height": "40",
                },
                "reopen": {
                    "left": "10",
                    "top": "21",
                    "width": "30",
                    "height": "40",
                },
                "first_save_sha256": sha(mutation_first.read_bytes()),
                "second_save_sha256": sha(mutation_second.read_bytes()),
                "first_save_path": str(mutation_first),
                "second_save_path": str(mutation_second),
            },
        ],
        "boundaries": {
            "disposable_copies_only": True,
            "source_pub_immutable": True,
            "native_writer_capability_granted": False,
            "shape_id_cross_save_stability_assumed": False,
            "oracle_tag_is_identity": True,
        },
    }
    write_json(path, manifest)


class MoveNodeDiagnosticBuilderTests(unittest.TestCase):
    def make_fixture(self, root: Path, *, dx_emu: int = 12700):
        source = minimal(0)
        control = directory_normalized(source)
        mutation = stream_mutated(control)
        second = mutation

        source_path = root / "source.pub"
        control_first = root / "control-first.pub"
        control_second = root / "control-second.pub"
        mutation_first = root / "mutation-first.pub"
        mutation_second = root / "mutation-second.pub"
        source_path.write_bytes(source)
        control_first.write_bytes(control)
        control_second.write_bytes(control)
        mutation_first.write_bytes(mutation)
        mutation_second.write_bytes(second)

        trace = root / "agent.ndjson"
        agent_trace(trace, sha(source), dx_emu=dx_emu)
        manifest = root / "publisher-movenode-causal.local.json"
        native_manifest(
            manifest,
            source_path=source_path,
            control_first=control_first,
            control_second=control_second,
            mutation_first=mutation_first,
            mutation_second=mutation_second,
            source_hash=sha(source),
        )
        return source_path, trace, manifest

    def parser_ok(self) -> list[str]:
        return [sys.executable, "-c", "import sys; sys.exit(0)"]

    def test_builds_both_receipts_and_keeps_paths_private(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, trace, manifest = self.make_fixture(root)
            blast_out = root / "blast.json"
            receipt_out = root / "joined.json"

            blast, receipt = build(
                source_path=source,
                agent_trace_path=trace,
                native_manifest_path=manifest,
                axis="x",
                parser_command=self.parser_ok(),
                blast_out=blast_out,
                receipt_out=receipt_out,
                tolerance_emu=0,
            )

            self.assertEqual(blast["schema_version"], "chaptera.operation-blast-radius.v1")
            self.assertEqual(
                receipt["receipt_version"],
                "chaptera.movenode-diagnostic-receipt.v1",
            )
            self.assertEqual(receipt["chaptera"]["node_id"], NODE_ID)
            self.assertEqual(
                receipt["chaptera"]["scene_instance_id"],
                INSTANCE_ID,
            )
            self.assertEqual(
                receipt["native_experiment"]["shape_identity"],
                "pageid:12345|tag:PUB_ORACLE_ID=SHAPE_A",
            )
            self.assertGreater(
                blast["classification_counts"]["unexplained_collateral"],
                0,
            )
            encoded = receipt_out.read_text(encoding="utf-8")
            self.assertNotIn(str(root), encoded)
            self.assertFalse(receipt["invariants"]["native_pub_writer_capability_granted"])

    def test_cross_layer_delta_mismatch_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, trace, manifest = self.make_fixture(root, dx_emu=25400)
            with self.assertRaises(BuilderError):
                build(
                    source_path=source,
                    agent_trace_path=trace,
                    native_manifest_path=manifest,
                    axis="x",
                    parser_command=self.parser_ok(),
                    blast_out=root / "blast.json",
                    receipt_out=root / "joined.json",
                    tolerance_emu=0,
                )

    def test_parser_rejection_is_fatal(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, trace, manifest = self.make_fixture(root)
            with self.assertRaises(BuilderError):
                build(
                    source_path=source,
                    agent_trace_path=trace,
                    native_manifest_path=manifest,
                    axis="x",
                    parser_command=[sys.executable, "-c", "import sys; sys.exit(1)"],
                    blast_out=root / "blast.json",
                    receipt_out=root / "joined.json",
                    tolerance_emu=0,
                )

    def test_multiple_durable_moves_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, trace, manifest = self.make_fixture(root)
            trace.write_text(trace.read_text() + trace.read_text().splitlines()[1] + "\n")
            with self.assertRaises(BuilderError):
                build(
                    source_path=source,
                    agent_trace_path=trace,
                    native_manifest_path=manifest,
                    axis="x",
                    parser_command=self.parser_ok(),
                    blast_out=root / "blast.json",
                    receipt_out=root / "joined.json",
                    tolerance_emu=0,
                )


if __name__ == "__main__":
    unittest.main()
