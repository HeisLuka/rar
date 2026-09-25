#!/usr/bin/env python3
import copy
import json
import unittest

from revision_store import RevisionKernel, canonical_json, project_hash
from rotate_nodes_quarter_v1 import (
    RotateNodesQuarterError,
    apply_rotate_nodes_quarter_v1,
)
from rotate_quarter_v1 import (
    RotateQuarterError,
    apply_rotate_node_quarter_v1,
)


DOCUMENT_ID = "doc:rotate-nodes-quarter"
SOURCE_HASH = "ab" * 32
SOURCE_BLOB_SHA256 = "cd" * 32
PAGE_ID = "page:1"
NODE_A = "01890f47-0c00-7abc-8def-0123456789ab"
NODE_B = "01890f47-0c00-7abc-8def-0123456789ac"
NODE_C = "01890f47-0c00-7abc-8def-0123456789ad"

IDENTITY = {
    "a": "1",
    "b": "0",
    "c": "0",
    "d": "1",
    "tx": "0",
    "ty": "0",
}


def shape(node_id, bounds, *, page_id=PAGE_ID):
    return {
        "kind": "shape",
        "node_id": node_id,
        "page_id": page_id,
        "parent_id": page_id,
        "shape_kind": "rectangle",
        "bounds": copy.deepcopy(bounds),
        "transform": {"kind": "identity"},
        "paint": {
            "fill": {
                "visible": True,
                "color": {"r": 17, "g": 34, "b": 51},
            },
            "stroke": {
                "visible": True,
                "color": {"r": 68, "g": 85, "b": 102},
                "width_emu": 12_700,
            },
            "provenance": {"kind": "author_created"},
        },
        "provenance": {"kind": "author_created"},
    }


BOUNDS_A = {"x": 10, "y": 20, "width": 101, "height": 51}
BOUNDS_B = {"x": 500, "y": 100, "width": 40, "height": 20}
BOUNDS_C = {"x": 900, "y": 300, "width": 80, "height": 60}


def project(*, include_third=False):
    shapes = {
        NODE_A: shape(NODE_A, BOUNDS_A),
        NODE_B: shape(NODE_B, BOUNDS_B),
    }
    children = [NODE_A, NODE_B]
    if include_third:
        shapes[NODE_C] = shape(NODE_C, BOUNDS_C)
        children.append(NODE_C)
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": SOURCE_HASH,
        "immutable_source_blob_sha256": SOURCE_BLOB_SHA256,
        "operations": [],
        "pages": {
            PAGE_ID: {
                "authoring_enabled": True,
                "children": children,
            }
        },
        "shapes": shapes,
        "text_frames": {},
        "picture_frames": {},
        "groups": {},
    }


def entry(node_id, expected=None):
    return {
        "node_id": node_id,
        "expected_before": copy.deepcopy(expected or IDENTITY),
    }


class RotateNodesQuarterV1Tests(unittest.TestCase):
    def setUp(self):
        self.project = project()
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )

    def request(
        self,
        op_id,
        *,
        entries=None,
        quarter_turns=1,
        base=None,
        document_id=DOCUMENT_ID,
        page_id=PAGE_ID,
    ):
        return {
            "protocol_version": "chaptera.rotate-nodes-quarter-intent.v1",
            "document_id": document_id,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "rotate_nodes_quarter_turn",
                "page_id": page_id,
                "entries": copy.deepcopy(
                    entries or [entry(NODE_B), entry(NODE_A)]
                ),
                "pivot_policy": "per_node_authored_bounds_center",
                "quarter_turns": quarter_turns,
            },
        }

    def test_batch_is_sorted_atomic_and_reuses_single_object_law(self):
        accepted = self.kernel.commit_rotate_nodes_quarter(
            self.request("rotate-nodes-0001"),
            apply_rotate_nodes_quarter_v1,
        )
        operation = accepted["canonical_operation"]
        self.assertEqual(
            [NODE_A, NODE_B],
            [member["node_id"] for member in operation["entries"]],
        )
        self.assertEqual("per_node_authored_bounds_center", operation["pivot_policy"])
        self.assertEqual(1, operation["quarter_turns"])
        self.assertEqual(
            [{"x": "60.5", "y": "45.5"}, {"x": "520", "y": "110"}],
            [member["pivot"] for member in operation["entries"]],
        )

        for member in operation["entries"]:
            single, _, _ = apply_rotate_node_quarter_v1(
                copy.deepcopy(self.project),
                {
                    "kind": "rotate_node_quarter_turn",
                    "node_id": member["node_id"],
                    "expected_before": copy.deepcopy(IDENTITY),
                    "pivot_policy": "authored_bounds_center",
                    "quarter_turns": 1,
                },
            )
            self.assertEqual(single["after"], member["after"])
            self.assertEqual(single["pivot"], member["pivot"])

        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(1, len(current["operations"]))
        self.assertEqual(operation, current["operations"][0])
        self.assertEqual(BOUNDS_A, current["shapes"][NODE_A]["bounds"])
        self.assertEqual(BOUNDS_B, current["shapes"][NODE_B]["bounds"])
        self.assertEqual(SOURCE_HASH, current["source_hash"])
        self.assertEqual(
            SOURCE_BLOB_SHA256,
            current["immutable_source_blob_sha256"],
        )
        self.assertNotEqual(
            current["shapes"][NODE_A]["transform"],
            current["shapes"][NODE_B]["transform"],
            "per-object centers should produce distinct translations",
        )

    def test_negative_turn_canonicalizes_to_three_for_every_member(self):
        accepted = self.kernel.commit_rotate_nodes_quarter(
            self.request("rotate-nodes-neg", quarter_turns=-1),
            apply_rotate_nodes_quarter_v1,
        )
        operation = accepted["canonical_operation"]
        self.assertEqual(3, operation["quarter_turns"])
        self.assertEqual(
            {"x": "60.5", "y": "45.5"},
            operation["entries"][0]["pivot"],
        )
        self.assertEqual(
            {"x": "520", "y": "110"},
            operation["entries"][1]["pivot"],
        )

    def test_enumeration_is_not_semantic_order(self):
        sorted_request = self.request(
            "rotate-enum-a",
            entries=[entry(NODE_A), entry(NODE_B)],
        )
        reversed_request = self.request(
            "rotate-enum-b",
            entries=[entry(NODE_B), entry(NODE_A)],
        )

        left = RevisionKernel()
        left_base = left.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project(),
        )
        sorted_request["base_revision_id"] = left_base.revision_id
        left_result = left.commit_rotate_nodes_quarter(
            sorted_request,
            apply_rotate_nodes_quarter_v1,
        )

        right = RevisionKernel()
        right_base = right.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project(),
        )
        reversed_request["base_revision_id"] = right_base.revision_id
        right_result = right.commit_rotate_nodes_quarter(
            reversed_request,
            apply_rotate_nodes_quarter_v1,
        )

        self.assertEqual(
            left_result["canonical_operation"],
            right_result["canonical_operation"],
        )
        self.assertEqual(
            project_hash(left.current_revision(DOCUMENT_ID).project),
            project_hash(right.current_revision(DOCUMENT_ID).project),
        )

    def test_full_turn_rejects_without_revision_or_history_entry(self):
        before = self.kernel.current_revision(DOCUMENT_ID)
        with self.assertRaises((RotateNodesQuarterError, RotateQuarterError)):
            self.kernel.commit_rotate_nodes_quarter(
                self.request("rotate-full-turn", quarter_turns=4),
                apply_rotate_nodes_quarter_v1,
            )
        after = self.kernel.current_revision(DOCUMENT_ID)
        self.assertEqual(before.revision_id, after.revision_id)
        self.assertEqual([], after.project["operations"])

    def test_one_bad_member_rejects_the_whole_batch(self):
        cases = []

        source_backed = project()
        source_backed["shapes"][NODE_B]["provenance"] = {"kind": "source_backed"}
        cases.append(source_backed)

        flipped = project()
        flipped["shapes"][NODE_B]["transform"] = {
            "kind": "affine",
            "a": "-1",
            "b": "0",
            "c": "0",
            "d": "1",
            "tx": "0",
            "ty": "0",
        }
        cases.append(flipped)

        for index, candidate in enumerate(cases):
            kernel = RevisionKernel()
            document_id = f"doc:bad-member:{index}"
            baseline = kernel.register_baseline(
                document_id=document_id,
                source_hash=SOURCE_HASH,
                project=candidate,
            )
            req = self.request(
                f"rotate-bad-member-{index}",
                base=baseline.revision_id,
                document_id=document_id,
            )
            with self.subTest(index=index):
                with self.assertRaises(RotateQuarterError):
                    kernel.commit_rotate_nodes_quarter(
                        req,
                        apply_rotate_nodes_quarter_v1,
                    )
                current = kernel.current_revision(document_id)
                self.assertEqual(baseline.revision_id, current.revision_id)
                self.assertEqual([], current.project["operations"])

        stale_entries = [
            entry(NODE_A),
            entry(NODE_B, {**IDENTITY, "tx": "1"}),
        ]
        with self.assertRaises(RotateQuarterError):
            self.kernel.commit_rotate_nodes_quarter(
                self.request("rotate-stale-member", entries=stale_entries),
                apply_rotate_nodes_quarter_v1,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_cross_page_member_rejects_atomically(self):
        candidate = project()
        candidate["shapes"][NODE_B]["page_id"] = "page:2"
        candidate["shapes"][NODE_B]["parent_id"] = "page:2"
        candidate["pages"]["page:2"] = {
            "authoring_enabled": True,
            "children": [NODE_B],
        }
        candidate["pages"][PAGE_ID]["children"] = [NODE_A]

        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:cross-page",
            source_hash=SOURCE_HASH,
            project=candidate,
        )
        req = self.request(
            "rotate-cross-page",
            base=baseline.revision_id,
            document_id="doc:cross-page",
        )
        with self.assertRaises(RotateNodesQuarterError):
            kernel.commit_rotate_nodes_quarter(
                req,
                apply_rotate_nodes_quarter_v1,
            )
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:cross-page").revision_id,
        )

    def test_executor_cannot_forge_pivot_bounds_or_non_target_shape(self):
        candidate = project(include_third=True)
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:forgery",
            source_hash=SOURCE_HASH,
            project=candidate,
        )
        req = self.request(
            "rotate-forgery",
            base=baseline.revision_id,
            document_id="doc:forgery",
        )

        def forged_pivot(base_project, command):
            operation, resulting, consequences = apply_rotate_nodes_quarter_v1(
                base_project, command
            )
            operation["entries"][0]["pivot"] = {"x": "0", "y": "0"}
            return operation, resulting, consequences

        with self.assertRaisesRegex(ValueError, "pivot differs"):
            kernel.commit_rotate_nodes_quarter(req, forged_pivot)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:forgery").revision_id,
        )

        def forged_bounds(base_project, command):
            operation, resulting, consequences = apply_rotate_nodes_quarter_v1(
                base_project, command
            )
            resulting["shapes"][NODE_A]["bounds"]["x"] += 1
            return operation, resulting, consequences

        with self.assertRaisesRegex(ValueError, "outside canonical transform"):
            kernel.commit_rotate_nodes_quarter(req, forged_bounds)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:forgery").revision_id,
        )

        def forged_other(base_project, command):
            operation, resulting, consequences = apply_rotate_nodes_quarter_v1(
                base_project, command
            )
            resulting["shapes"][NODE_C]["transform"] = {
                "kind": "affine",
                **operation["entries"][0]["after"],
            }
            return operation, resulting, consequences

        with self.assertRaisesRegex(ValueError, "non-target shape"):
            kernel.commit_rotate_nodes_quarter(req, forged_other)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:forgery").revision_id,
        )

    def test_undo_redo_replay_and_save_reopen_are_exact(self):
        accepted = self.kernel.commit_rotate_nodes_quarter(
            self.request("rotate-history-commit"),
            apply_rotate_nodes_quarter_v1,
        )
        rotated_project = copy.deepcopy(
            self.kernel.current_revision(DOCUMENT_ID).project
        )

        def history_executor(_base, kind):
            if kind == "undo":
                return copy.deepcopy(self.project), [
                    {"key": "history.undo", "state": "supported", "note": None}
                ]
            if kind == "redo":
                return copy.deepcopy(rotated_project), [
                    {"key": "history.redo", "state": "supported", "note": None}
                ]
            raise ValueError("unsupported history transition")

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "rotate-nodes-history-undo",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertEqual(
            {"kind": "identity"},
            self.kernel.current_revision(DOCUMENT_ID)
            .project["shapes"][NODE_A]["transform"],
        )
        self.assertEqual(
            {"kind": "identity"},
            self.kernel.current_revision(DOCUMENT_ID)
            .project["shapes"][NODE_B]["transform"],
        )

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "rotate-nodes-history-redo",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(
            project_hash(rotated_project),
            project_hash(self.kernel.current_revision(DOCUMENT_ID).project),
        )

        saved = json.loads(canonical_json(rotated_project).decode("utf-8"))
        reopened = RevisionKernel()
        reopened_baseline = reopened.register_baseline(
            document_id="doc:rotate-nodes-reopened",
            source_hash=SOURCE_HASH,
            project=saved,
        )
        self.assertEqual(
            project_hash(rotated_project),
            reopened_baseline.project_hash,
        )

        replay_operation, replayed, _ = apply_rotate_nodes_quarter_v1(
            copy.deepcopy(self.project),
            self.request(
                "rotate-nodes-replay",
                entries=[entry(NODE_A), entry(NODE_B)],
            )["command"],
        )
        self.assertEqual(accepted["canonical_operation"], replay_operation)
        self.assertEqual(
            project_hash(rotated_project),
            project_hash(replayed),
        )


if __name__ == "__main__":
    unittest.main()
