#!/usr/bin/env python3
import copy
import json
import unittest

from revision_store import RevisionKernel, canonical_json, project_hash


DOCUMENT_ID = "doc:shape-paint"
SOURCE_HASH = "ab" * 32
NODE_ID = "shape:authored:1"


FILL_BEFORE = {
    "visible": True,
    "color": {"r": 0x11, "g": 0x22, "b": 0x33},
}
FILL_AFTER = {
    "visible": False,
    "color": {"r": 0x44, "g": 0x55, "b": 0x66},
}
STROKE_BEFORE = {
    "visible": True,
    "color": {"r": 1, "g": 2, "b": 3},
    "width_emu": 12_700,
}
STROKE_AFTER = {
    "visible": True,
    "color": {"r": 7, "g": 8, "b": 9},
    "width_emu": 25_400,
}


class FakeShapePaintExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        self.calls += 1
        node_id = command["node_id"]
        shape = base_project.get("shapes", {}).get(node_id)
        if not isinstance(shape, dict) or shape.get("kind") != "shape":
            raise ValueError("unsupported_shape_class")
        paint = shape.get("paint")
        if not isinstance(paint, dict):
            raise ValueError("missing_canonical_shape_paint")
        provenance = paint.get("provenance")
        if provenance != {"kind": "author_created"}:
            raise ValueError("source_backed_shape_paint_edit_not_enabled_v1")

        if command["kind"] == "set_fill":
            axis = "fill"
            consequence = "shape.fill"
        elif command["kind"] == "set_stroke":
            axis = "stroke"
            consequence = "shape.stroke"
        else:
            raise ValueError("unsupported_shape_paint_operation")

        current = paint.get(axis)
        if current != command["expected_before"]:
            raise ValueError(f"stale_shape_{axis}")

        operation = {
            "kind": command["kind"],
            "node_id": node_id,
            "before": copy.deepcopy(current),
            "after": copy.deepcopy(command["after"]),
        }
        project = copy.deepcopy(base_project)
        project["shapes"][node_id]["paint"][axis] = copy.deepcopy(command["after"])
        project["operations"].append(copy.deepcopy(operation))
        consequences = [
            {"key": consequence, "state": "supported", "note": None},
            {"key": "layout.scene", "state": "invalidated", "note": None},
            {"key": "editable_export", "state": "invalidated", "note": None},
        ]
        return operation, project, consequences


class ShapePaintCommitTests(unittest.TestCase):
    def setUp(self):
        self.project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [],
            "shapes": {
                NODE_ID: {
                    "kind": "shape",
                    "author_created": True,
                    "bounds": {"x": 10, "y": 20, "width": 300, "height": 200},
                    "paint": {
                        "fill": copy.deepcopy(FILL_BEFORE),
                        "stroke": copy.deepcopy(STROKE_BEFORE),
                        "provenance": {"kind": "author_created"},
                    },
                }
            },
        }
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        self.executor = FakeShapePaintExecutor()

    def fill_request(self, op_id, *, before=None, after=None, base=None):
        return {
            "protocol_version": "chaptera.shape-fill-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "set_fill",
                "node_id": NODE_ID,
                "expected_before": copy.deepcopy(before or FILL_BEFORE),
                "after": copy.deepcopy(after or FILL_AFTER),
            },
        }

    def stroke_request(self, op_id, *, before=None, after=None, base=None):
        return {
            "protocol_version": "chaptera.shape-stroke-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "set_stroke",
                "node_id": NODE_ID,
                "expected_before": copy.deepcopy(before or STROKE_BEFORE),
                "after": copy.deepcopy(after or STROKE_AFTER),
            },
        }

    def test_set_fill_commits_only_fill_and_invalidates_consumers(self):
        result = self.kernel.commit_shape_fill(
            self.fill_request("shape-fill-00000001"),
            self.executor,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).project
        shape = current["shapes"][NODE_ID]
        self.assertEqual(FILL_AFTER, shape["paint"]["fill"])
        self.assertEqual(STROKE_BEFORE, shape["paint"]["stroke"])
        self.assertEqual({"kind": "author_created"}, shape["paint"]["provenance"])
        self.assertEqual(
            ["shape.fill", "layout.scene", "editable_export"],
            [item["key"] for item in result["consequences"]],
        )
        self.assertEqual(FILL_BEFORE, result["canonical_operation"]["before"])
        self.assertEqual(FILL_AFTER, result["canonical_operation"]["after"])

    def test_set_stroke_commits_positive_exact_emu_width(self):
        result = self.kernel.commit_shape_stroke(
            self.stroke_request("shape-stroke-00000001"),
            self.executor,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(STROKE_AFTER, current["shapes"][NODE_ID]["paint"]["stroke"])
        self.assertEqual("set_stroke", result["canonical_operation"]["kind"])

    def test_exact_retry_is_idempotent(self):
        req = self.fill_request("shape-fill-00000002")
        first = self.kernel.commit_shape_fill(copy.deepcopy(req), self.executor)
        second = self.kernel.commit_shape_fill(copy.deepcopy(req), self.executor)
        self.assertEqual(first, second)
        self.assertEqual(1, self.executor.calls)

    def test_stale_revision_rejected_before_executor(self):
        accepted = self.kernel.commit_shape_fill(
            self.fill_request("shape-fill-00000003"),
            self.executor,
        )
        calls = self.executor.calls
        stale = self.kernel.commit_shape_stroke(
            self.stroke_request(
                "shape-stroke-00000002",
                base=self.baseline.revision_id,
            ),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(accepted["revision_id"], stale["current_revision_id"])
        self.assertEqual(calls, self.executor.calls)

    def test_stale_semantic_before_fails_without_revision_move(self):
        with self.assertRaisesRegex(ValueError, "stale_shape_fill"):
            self.kernel.commit_shape_fill(
                self.fill_request(
                    "shape-fill-00000004",
                    before={
                        "visible": True,
                        "color": {"r": 0, "g": 0, "b": 0},
                    },
                ),
                self.executor,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_noop_bad_rgb_and_bad_stroke_width_fail_before_executor(self):
        with self.assertRaisesRegex(ValueError, "no-op"):
            self.kernel.commit_shape_fill(
                self.fill_request("shape-fill-00000005", after=FILL_BEFORE),
                self.executor,
            )
        bad_rgb = self.fill_request("shape-fill-00000006")
        bad_rgb["command"]["after"]["color"]["r"] = 256
        with self.assertRaisesRegex(ValueError, "sRGB byte"):
            self.kernel.commit_shape_fill(bad_rgb, self.executor)

        bad_width = self.stroke_request("shape-stroke-00000003")
        bad_width["command"]["after"]["width_emu"] = 0
        with self.assertRaisesRegex(ValueError, "positive"):
            self.kernel.commit_shape_stroke(bad_width, self.executor)
        self.assertEqual(0, self.executor.calls)

    def test_browser_cannot_supply_provenance_or_source_ref(self):
        for method, request in (
            (self.kernel.commit_shape_fill, self.fill_request("shape-fill-extra-0001")),
            (self.kernel.commit_shape_stroke, self.stroke_request("shape-stroke-extra-0001")),
        ):
            request["command"]["source_ref"] = {"carrier": "forged"}
            with self.subTest(kind=request["command"]["kind"]):
                with self.assertRaisesRegex(ValueError, "non-intent"):
                    method(request, self.executor)
        self.assertEqual(0, self.executor.calls)

    def test_executor_cannot_forge_before_after_or_target(self):
        def forged(base_project, command):
            operation, project, consequences = self.executor(base_project, command)
            operation["after"]["color"]["b"] ^= 1
            return operation, project, consequences

        with self.assertRaisesRegex(ValueError, "after-state"):
            self.kernel.commit_shape_fill(
                self.fill_request("shape-fill-00000007"),
                forged,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_source_backed_edit_remains_fail_closed_and_source_ref_unchanged(self):
        source_project = copy.deepcopy(self.project)
        source_project["shapes"][NODE_ID]["author_created"] = False
        source_project["shapes"][NODE_ID]["paint"]["provenance"] = {
            "kind": "source_backed",
            "source_ref": {
                "format": "pub",
                "source_hash_hex": SOURCE_HASH,
                "carrier": "/Escher/EscherStm",
            },
        }
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:source-shape",
            source_hash=SOURCE_HASH,
            project=source_project,
        )
        request = self.fill_request("shape-fill-source-0001")
        request["document_id"] = "doc:source-shape"
        request["base_revision_id"] = baseline.revision_id

        executor = FakeShapePaintExecutor()
        with self.assertRaisesRegex(ValueError, "source_backed_shape_paint_edit_not_enabled_v1"):
            kernel.commit_shape_fill(request, executor)

        current = kernel.current_revision("doc:source-shape")
        self.assertEqual(baseline.revision_id, current.revision_id)
        self.assertEqual(
            source_project["shapes"][NODE_ID]["paint"]["provenance"],
            current.project["shapes"][NODE_ID]["paint"]["provenance"],
        )
        self.assertEqual(SOURCE_HASH, current.project["source_hash"])

    def test_undo_redo_replay_and_save_reopen_preserve_exact_paint(self):
        accepted = self.kernel.commit_shape_fill(
            self.fill_request("shape-fill-00000008"),
            self.executor,
        )
        accepted_project = copy.deepcopy(self.kernel.current_revision(DOCUMENT_ID).project)

        def history_executor(_base_project, kind):
            if kind == "undo":
                return copy.deepcopy(self.project), [
                    {"key": "history.undo", "state": "supported", "note": None}
                ]
            if kind == "redo":
                return copy.deepcopy(accepted_project), [
                    {"key": "history.redo", "state": "supported", "note": None}
                ]
            raise ValueError("unsupported history transition")

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "shape-paint-undo-0001",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertEqual(
            FILL_BEFORE,
            self.kernel.current_revision(DOCUMENT_ID).project["shapes"][NODE_ID]["paint"]["fill"],
        )

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "shape-paint-redo-0001",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(
            FILL_AFTER,
            self.kernel.current_revision(DOCUMENT_ID).project["shapes"][NODE_ID]["paint"]["fill"],
        )

        saved = json.loads(canonical_json(accepted_project).decode("utf-8"))
        reopened = RevisionKernel()
        reopened_baseline = reopened.register_baseline(
            document_id="doc:reopened",
            source_hash=SOURCE_HASH,
            project=saved,
        )
        self.assertEqual(project_hash(accepted_project), reopened_baseline.project_hash)
        self.assertEqual(
            FILL_AFTER,
            reopened_baseline.project["shapes"][NODE_ID]["paint"]["fill"],
        )

        replay_project = copy.deepcopy(self.project)
        replay_op, replayed, _ = FakeShapePaintExecutor()(
            replay_project,
            {
                "kind": accepted["canonical_operation"]["kind"],
                "node_id": accepted["canonical_operation"]["node_id"],
                "expected_before": accepted["canonical_operation"]["before"],
                "after": accepted["canonical_operation"]["after"],
            },
        )
        self.assertEqual(accepted["canonical_operation"], replay_op)
        self.assertEqual(
            accepted_project["shapes"][NODE_ID]["paint"],
            replayed["shapes"][NODE_ID]["paint"],
        )


if __name__ == "__main__":
    unittest.main()
