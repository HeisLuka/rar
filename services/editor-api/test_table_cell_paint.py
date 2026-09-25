#!/usr/bin/env python3
import copy
import json
import unittest

from revision_store import RevisionKernel, canonical_json, project_hash


DOCUMENT_ID = "doc:table-cell-paint"
SOURCE_HASH = "cd" * 32
CELL_ID = "00112233-4455-6677-8899-aabbccddeeff"

FILL_A = {"visible": True, "color": {"r": 0x11, "g": 0x22, "b": 0x33}}
FILL_B = {"visible": False, "color": {"r": 0x44, "g": 0x55, "b": 0x66}}
BORDER_A = {
    "visible": True,
    "color": {"r": 1, "g": 2, "b": 3},
    "width_emu": 12_700,
}
BORDER_B = {
    "visible": False,
    "color": {"r": 7, "g": 8, "b": 9},
    "width_emu": 25_400,
}


class FakeTableCellPaintExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        self.calls += 1
        cell_id = command["table_cell_id"]
        cell = base_project.get("table_cells", {}).get(cell_id)
        if not isinstance(cell, dict) or cell.get("kind") != "table_cell":
            raise ValueError("unknown_table_cell")
        if cell.get("table_class") != "simple_unmerged_rectangular":
            raise ValueError("unsupported_table_cell_class")

        project = copy.deepcopy(base_project)
        target = project["table_cells"][cell_id]
        kind = command["kind"]

        if kind == "set_table_cell_fill":
            current = target.get("fill")
            if current != command["expected_before"]:
                raise ValueError("stale_table_cell_fill")
            operation = {
                "kind": kind,
                "table_cell_id": cell_id,
                "before": copy.deepcopy(current),
                "after": copy.deepcopy(command["after"]),
            }
            target["fill"] = copy.deepcopy(command["after"])
            consequence = "table_cell.fill"
        elif kind == "clear_table_cell_fill":
            current = target.get("fill")
            if current != command["expected_before"]:
                raise ValueError("stale_table_cell_fill")
            operation = {
                "kind": kind,
                "table_cell_id": cell_id,
                "before": copy.deepcopy(current),
            }
            target["fill"] = None
            consequence = "table_cell.fill"
        elif kind == "set_table_cell_border_side":
            side = command["side"]
            current = target["borders"].get(side)
            if current != command["expected_before"]:
                raise ValueError("stale_table_cell_border")
            operation = {
                "kind": kind,
                "table_cell_id": cell_id,
                "side": side,
                "before": copy.deepcopy(current),
                "after": copy.deepcopy(command["after"]),
            }
            target["borders"][side] = copy.deepcopy(command["after"])
            consequence = f"table_cell.border.{side}"
        elif kind == "clear_table_cell_border_side":
            side = command["side"]
            current = target["borders"].get(side)
            if current != command["expected_before"]:
                raise ValueError("stale_table_cell_border")
            operation = {
                "kind": kind,
                "table_cell_id": cell_id,
                "side": side,
                "before": copy.deepcopy(current),
            }
            target["borders"][side] = None
            consequence = f"table_cell.border.{side}"
        else:
            raise ValueError("unsupported_table_cell_paint_operation")

        project["operations"].append(copy.deepcopy(operation))
        consequences = [
            {"key": consequence, "state": "supported", "note": None},
            {"key": "layout.scene", "state": "invalidated", "note": None},
            {"key": "editable_export", "state": "invalidated", "note": None},
            {"key": "fixed_output", "state": "invalidated", "note": None},
        ]
        return operation, project, consequences


class TableCellPaintCommitTests(unittest.TestCase):
    def setUp(self):
        self.project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [],
            "table_cells": {
                CELL_ID: {
                    "kind": "table_cell",
                    "table_class": "simple_unmerged_rectangular",
                    "fill": copy.deepcopy(FILL_A),
                    "borders": {
                        "top": copy.deepcopy(BORDER_A),
                        "right": None,
                        "bottom": None,
                        "left": None,
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
        self.executor = FakeTableCellPaintExecutor()

    def fill_request(self, op_id, *, before=FILL_A, after=FILL_B, base=None):
        return {
            "protocol_version": "chaptera.table-cell-fill-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "set_table_cell_fill",
                "table_cell_id": CELL_ID,
                "expected_before": copy.deepcopy(before),
                "after": copy.deepcopy(after),
            },
        }

    def clear_fill_request(self, op_id, *, before=FILL_A, base=None):
        return {
            "protocol_version": "chaptera.table-cell-fill-clear-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "clear_table_cell_fill",
                "table_cell_id": CELL_ID,
                "expected_before": copy.deepcopy(before),
            },
        }

    def border_request(
        self,
        op_id,
        *,
        side="right",
        before=None,
        after=BORDER_B,
        base=None,
    ):
        return {
            "protocol_version": "chaptera.table-cell-border-side-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "set_table_cell_border_side",
                "table_cell_id": CELL_ID,
                "side": side,
                "expected_before": copy.deepcopy(before),
                "after": copy.deepcopy(after),
            },
        }

    def clear_border_request(self, op_id, *, side="top", before=BORDER_A, base=None):
        return {
            "protocol_version": "chaptera.table-cell-border-side-clear-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "clear_table_cell_border_side",
                "table_cell_id": CELL_ID,
                "side": side,
                "expected_before": copy.deepcopy(before),
            },
        }

    def test_set_fill_targets_cell_identity_not_shape_identity(self):
        result = self.kernel.commit_table_cell_fill(
            self.fill_request("cell-fill-0001"),
            self.executor,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(FILL_B, current["table_cells"][CELL_ID]["fill"])
        self.assertEqual(BORDER_A, current["table_cells"][CELL_ID]["borders"]["top"])
        self.assertEqual(CELL_ID, result["canonical_operation"]["table_cell_id"])
        self.assertEqual(
            ["table_cell.fill", "layout.scene", "editable_export", "fixed_output"],
            [item["key"] for item in result["consequences"]],
        )

    def test_set_and_clear_border_side_do_not_alias_other_sides(self):
        set_result = self.kernel.commit_table_cell_border_side(
            self.border_request("cell-border-0001", side="right"),
            self.executor,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(BORDER_B, current["table_cells"][CELL_ID]["borders"]["right"])
        self.assertEqual(BORDER_A, current["table_cells"][CELL_ID]["borders"]["top"])
        self.assertIsNone(current["table_cells"][CELL_ID]["borders"]["bottom"])
        self.assertIsNone(current["table_cells"][CELL_ID]["borders"]["left"])

        clear = self.clear_border_request(
            "cell-border-clear-0001",
            side="right",
            before=BORDER_B,
            base=set_result["revision_id"],
        )
        self.kernel.commit_clear_table_cell_border_side(clear, self.executor)
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertIsNone(current["table_cells"][CELL_ID]["borders"]["right"])
        self.assertEqual(BORDER_A, current["table_cells"][CELL_ID]["borders"]["top"])

    def test_clear_fill_is_explicit_and_replayable(self):
        accepted = self.kernel.commit_clear_table_cell_fill(
            self.clear_fill_request("cell-fill-clear-0001"),
            self.executor,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertIsNone(current["table_cells"][CELL_ID]["fill"])
        self.assertEqual("clear_table_cell_fill", accepted["canonical_operation"]["kind"])

        replay_project = copy.deepcopy(self.project)
        op, replayed, _ = FakeTableCellPaintExecutor()(
            replay_project,
            {
                "kind": "clear_table_cell_fill",
                "table_cell_id": CELL_ID,
                "expected_before": accepted["canonical_operation"]["before"],
            },
        )
        self.assertEqual(accepted["canonical_operation"], op)
        self.assertIsNone(replayed["table_cells"][CELL_ID]["fill"])

    def test_exact_retry_and_stale_revision_are_fenced(self):
        request = self.fill_request("cell-fill-idem-0001")
        first = self.kernel.commit_table_cell_fill(copy.deepcopy(request), self.executor)
        second = self.kernel.commit_table_cell_fill(copy.deepcopy(request), self.executor)
        self.assertEqual(first, second)
        self.assertEqual(1, self.executor.calls)

        stale = self.kernel.commit_table_cell_border_side(
            self.border_request(
                "cell-border-stale-revision-0001",
                base=self.baseline.revision_id,
            ),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(first["revision_id"], stale["current_revision_id"])
        self.assertEqual(1, self.executor.calls)

    def test_stale_semantic_before_noop_bad_side_and_bad_width_fail_closed(self):
        with self.assertRaisesRegex(ValueError, "stale_table_cell_fill"):
            self.kernel.commit_table_cell_fill(
                self.fill_request(
                    "cell-fill-stale-0001",
                    before={"visible": True, "color": {"r": 0, "g": 0, "b": 0}},
                ),
                self.executor,
            )

        with self.assertRaisesRegex(ValueError, "no-op"):
            self.kernel.commit_table_cell_fill(
                self.fill_request("cell-fill-noop-0001", after=FILL_A),
                self.executor,
            )

        bad_side = self.border_request("cell-border-side-0001", side="diagonal")
        with self.assertRaisesRegex(ValueError, "top/right/bottom/left"):
            self.kernel.commit_table_cell_border_side(bad_side, self.executor)

        bad_width = self.border_request("cell-border-width-0001")
        bad_width["command"]["after"]["width_emu"] = 0
        with self.assertRaisesRegex(ValueError, "positive"):
            self.kernel.commit_table_cell_border_side(bad_width, self.executor)

    def test_browser_cannot_forge_raw_carriers_or_shape_target(self):
        for key in ("fopt", "mcld", "node_id", "source_ref"):
            request = self.fill_request(f"cell-extra-{key}")
            request["command"][key] = "forged"
            with self.subTest(key=key):
                with self.assertRaisesRegex(ValueError, "non-intent"):
                    self.kernel.commit_table_cell_fill(request, self.executor)
        self.assertEqual(0, self.executor.calls)

    def test_executor_cannot_forge_target_side_or_after(self):
        def forged(base_project, command):
            operation, project, consequences = self.executor(base_project, command)
            operation["after"]["color"]["b"] ^= 1
            return operation, project, consequences

        with self.assertRaisesRegex(ValueError, "after differs"):
            self.kernel.commit_table_cell_border_side(
                self.border_request("cell-border-forged-0001"),
                forged,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_unsupported_table_class_fails_closed_without_revision_move(self):
        project = copy.deepcopy(self.project)
        project["table_cells"][CELL_ID]["table_class"] = "merged_or_styled_unknown"
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:unsupported-table",
            source_hash=SOURCE_HASH,
            project=project,
        )
        request = self.fill_request("cell-fill-unsupported-0001")
        request["document_id"] = "doc:unsupported-table"
        request["base_revision_id"] = baseline.revision_id
        executor = FakeTableCellPaintExecutor()
        with self.assertRaisesRegex(ValueError, "unsupported_table_cell_class"):
            kernel.commit_table_cell_fill(request, executor)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:unsupported-table").revision_id,
        )

    def test_undo_redo_save_reopen_and_replay_preserve_exact_cell_paint(self):
        accepted = self.kernel.commit_table_cell_border_side(
            self.border_request("cell-border-history-0001", side="right"),
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
                "client_operation_id": "cell-paint-undo-0001",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertIsNone(
            self.kernel.current_revision(DOCUMENT_ID).project["table_cells"][CELL_ID]["borders"]["right"]
        )

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "cell-paint-redo-0001",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(
            BORDER_B,
            self.kernel.current_revision(DOCUMENT_ID).project["table_cells"][CELL_ID]["borders"]["right"],
        )

        saved = json.loads(canonical_json(accepted_project).decode("utf-8"))
        reopened = RevisionKernel()
        reopened_baseline = reopened.register_baseline(
            document_id="doc:cell-reopened",
            source_hash=SOURCE_HASH,
            project=saved,
        )
        self.assertEqual(project_hash(accepted_project), reopened_baseline.project_hash)
        self.assertEqual(
            BORDER_B,
            reopened_baseline.project["table_cells"][CELL_ID]["borders"]["right"],
        )

        replay_op, replayed, _ = FakeTableCellPaintExecutor()(
            copy.deepcopy(self.project),
            {
                "kind": accepted["canonical_operation"]["kind"],
                "table_cell_id": accepted["canonical_operation"]["table_cell_id"],
                "side": accepted["canonical_operation"]["side"],
                "expected_before": accepted["canonical_operation"]["before"],
                "after": accepted["canonical_operation"]["after"],
            },
        )
        self.assertEqual(accepted["canonical_operation"], replay_op)
        self.assertEqual(
            accepted_project["table_cells"][CELL_ID],
            replayed["table_cells"][CELL_ID],
        )


if __name__ == "__main__":
    unittest.main()
