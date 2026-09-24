#!/usr/bin/env python3
import copy
import json
import unittest

from revision_store import RevisionKernel, canonical_json, project_hash
from rotate_quarter_v1 import (
    RotateQuarterError,
    apply_rotate_node_quarter_v1,
    canonical_entity_affine_v1,
)


DOCUMENT_ID = "doc:rotate-quarter"
SOURCE_HASH = "ab" * 32
SOURCE_BLOB_SHA256 = "cd" * 32
NODE_ID = "01890f47-0c00-7abc-8def-0123456789ab"
PAGE_ID = "page:1"

IDENTITY = {
    "a": "1",
    "b": "0",
    "c": "0",
    "d": "1",
    "tx": "0",
    "ty": "0",
}

BOUNDS = {
    "x": 10,
    "y": 20,
    "width": 101,
    "height": 51,
}


def shape():
    return {
        "kind": "shape",
        "node_id": NODE_ID,
        "page_id": PAGE_ID,
        "parent_id": PAGE_ID,
        "shape_kind": "rectangle",
        "bounds": copy.deepcopy(BOUNDS),
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


def project():
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": SOURCE_HASH,
        "immutable_source_blob_sha256": SOURCE_BLOB_SHA256,
        "operations": [],
        "pages": {
            PAGE_ID: {
                "authoring_enabled": True,
                "children": [NODE_ID],
            }
        },
        "shapes": {NODE_ID: shape()},
        "text_frames": {},
        "picture_frames": {},
        "groups": {},
    }


class RotateQuarterV1Tests(unittest.TestCase):
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
        expected_before=None,
        quarter_turns=1,
        base=None,
    ):
        return {
            "protocol_version": "chaptera.rotate-node-quarter-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "rotate_node_quarter_turn",
                "node_id": NODE_ID,
                "expected_before": copy.deepcopy(expected_before or IDENTITY),
                "pivot_policy": "authored_bounds_center",
                "quarter_turns": quarter_turns,
            },
        }

    def test_clockwise_quarter_turn_commits_exact_half_emu_center_matrix(self):
        result = self.kernel.commit_rotate_node_quarter(
            self.request("rotate-quarter-0001"),
            apply_rotate_node_quarter_v1,
        )
        operation = result["canonical_operation"]
        self.assertEqual({"x": "60.5", "y": "45.5"}, operation["pivot"])
        self.assertEqual(1, operation["quarter_turns"])
        self.assertEqual(
            {
                "a": "0",
                "b": "1",
                "c": "-1",
                "d": "0",
                "tx": "106",
                "ty": "-15",
            },
            operation["after"],
        )

        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(
            {"kind": "affine", **operation["after"]},
            current["shapes"][NODE_ID]["transform"],
        )
        self.assertEqual(SOURCE_HASH, current["source_hash"])
        self.assertEqual(
            SOURCE_BLOB_SHA256,
            current["immutable_source_blob_sha256"],
        )
        self.assertEqual(
            "unsupported",
            next(
                item["state"]
                for item in result["consequences"]
                if item["key"] == "native_pub_write"
            ),
        )

    def test_negative_turn_canonicalizes_to_three(self):
        result = self.kernel.commit_rotate_node_quarter(
            self.request("rotate-quarter-neg-0001", quarter_turns=-1),
            apply_rotate_node_quarter_v1,
        )
        operation = result["canonical_operation"]
        self.assertEqual(3, operation["quarter_turns"])
        self.assertEqual(
            {
                "a": "0",
                "b": "-1",
                "c": "1",
                "d": "0",
                "tx": "15",
                "ty": "106",
            },
            operation["after"],
        )

    def test_four_successive_quarters_cycle_to_explicit_identity(self):
        base = self.baseline.revision_id
        expected = copy.deepcopy(IDENTITY)
        for index in range(4):
            result = self.kernel.commit_rotate_node_quarter(
                self.request(
                    f"rotate-cycle-{index}",
                    expected_before=expected,
                    base=base,
                ),
                apply_rotate_node_quarter_v1,
            )
            base = result["revision_id"]
            expected = copy.deepcopy(result["canonical_operation"]["after"])

        self.assertEqual(IDENTITY, expected)
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(
            {"kind": "affine", **IDENTITY},
            current["shapes"][NODE_ID]["transform"],
        )
        self.assertEqual(4, len(current["operations"]))

    def test_full_turn_rejects_without_revision_or_history_move(self):
        before = self.kernel.current_revision(DOCUMENT_ID)
        with self.assertRaisesRegex(RotateQuarterError, "full-turn/no-op"):
            self.kernel.commit_rotate_node_quarter(
                self.request("rotate-full-turn-0001", quarter_turns=4),
                apply_rotate_node_quarter_v1,
            )
        after = self.kernel.current_revision(DOCUMENT_ID)
        self.assertEqual(before.revision_id, after.revision_id)
        self.assertEqual(before.state_id, after.state_id)
        self.assertEqual([], after.project["operations"])

    def test_stale_revision_and_stale_transform_precondition_fail_closed(self):
        accepted = self.kernel.commit_rotate_node_quarter(
            self.request("rotate-stale-first"),
            apply_rotate_node_quarter_v1,
        )
        stale = self.kernel.commit_rotate_node_quarter(
            self.request("rotate-stale-second", base=self.baseline.revision_id),
            apply_rotate_node_quarter_v1,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(accepted["revision_id"], stale["current_revision_id"])

        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:rotate-precondition",
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        req = self.request(
            "rotate-bad-before",
            expected_before={**IDENTITY, "tx": "1"},
            base=baseline.revision_id,
        )
        req["document_id"] = "doc:rotate-precondition"
        with self.assertRaisesRegex(
            RotateQuarterError,
            "transform changed since expected_before",
        ):
            kernel.commit_rotate_node_quarter(req, apply_rotate_node_quarter_v1)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:rotate-precondition").revision_id,
        )

    def test_source_backed_group_owned_and_flipped_targets_fail_closed(self):
        mutations = [
            lambda entity: entity.update(
                {"provenance": {"kind": "source_backed"}}
            ),
            lambda entity: entity.update({"parent_id": "group:1"}),
            lambda entity: entity.update(
                {
                    "transform": {
                        "kind": "affine",
                        "a": "-1",
                        "b": "0",
                        "c": "0",
                        "d": "1",
                        "tx": "0",
                        "ty": "0",
                    }
                }
            ),
        ]
        for index, mutate in enumerate(mutations):
            candidate = project()
            mutate(candidate["shapes"][NODE_ID])
            kernel = RevisionKernel()
            baseline = kernel.register_baseline(
                document_id=f"doc:unsupported:{index}",
                source_hash=SOURCE_HASH,
                project=candidate,
            )
            req = self.request(f"rotate-unsupported-{index}", base=baseline.revision_id)
            req["document_id"] = f"doc:unsupported:{index}"
            with self.subTest(index=index):
                with self.assertRaises(RotateQuarterError):
                    kernel.commit_rotate_node_quarter(
                        req,
                        apply_rotate_node_quarter_v1,
                    )
                self.assertEqual(
                    baseline.revision_id,
                    kernel.current_revision(f"doc:unsupported:{index}").revision_id,
                )

    def test_browser_cannot_supply_authoritative_pivot_or_after_state(self):
        for field, value in (
            ("pivot", {"x": "0", "y": "0"}),
            ("after", copy.deepcopy(IDENTITY)),
            ("before", copy.deepcopy(IDENTITY)),
            ("source_ref", {"carrier": "forged"}),
        ):
            req = self.request(f"rotate-extra-{field}")
            req["command"][field] = value
            with self.subTest(field=field):
                with self.assertRaisesRegex(
                    RotateQuarterError,
                    "non-intent/authoritative",
                ):
                    self.kernel.commit_rotate_node_quarter(
                        req,
                        apply_rotate_node_quarter_v1,
                    )

    def test_executor_cannot_forge_pivot_or_resulting_transform(self):
        def forged_pivot(base_project, command):
            operation, resulting, consequences = apply_rotate_node_quarter_v1(
                base_project,
                command,
            )
            operation["pivot"] = {"x": "0", "y": "0"}
            return operation, resulting, consequences

        with self.assertRaisesRegex(ValueError, "pivot differs"):
            self.kernel.commit_rotate_node_quarter(
                self.request("rotate-forged-pivot"),
                forged_pivot,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

        def forged_result(base_project, command):
            operation, resulting, consequences = apply_rotate_node_quarter_v1(
                base_project,
                command,
            )
            resulting["shapes"][NODE_ID]["transform"] = {"kind": "identity"}
            return operation, resulting, consequences

        with self.assertRaisesRegex(ValueError, "not bound"):
            self.kernel.commit_rotate_node_quarter(
                self.request("rotate-forged-result"),
                forged_result,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_undo_redo_replay_and_save_reopen_are_exact(self):
        accepted = self.kernel.commit_rotate_node_quarter(
            self.request("rotate-history-commit"),
            apply_rotate_node_quarter_v1,
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
                "client_operation_id": "rotate-history-undo",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertEqual(
            {"kind": "identity"},
            self.kernel.current_revision(DOCUMENT_ID)
            .project["shapes"][NODE_ID]["transform"],
        )

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "rotate-history-redo",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(
            rotated_project["shapes"][NODE_ID]["transform"],
            self.kernel.current_revision(DOCUMENT_ID)
            .project["shapes"][NODE_ID]["transform"],
        )

        saved = json.loads(canonical_json(rotated_project).decode("utf-8"))
        reopened = RevisionKernel()
        reopened_baseline = reopened.register_baseline(
            document_id="doc:rotate-reopened",
            source_hash=SOURCE_HASH,
            project=saved,
        )
        self.assertEqual(project_hash(rotated_project), reopened_baseline.project_hash)

        replay_operation, replayed, _ = apply_rotate_node_quarter_v1(
            copy.deepcopy(self.project),
            self.request("rotate-replay")["command"],
        )
        self.assertEqual(accepted["canonical_operation"], replay_operation)
        self.assertEqual(
            rotated_project["shapes"][NODE_ID]["transform"],
            replayed["shapes"][NODE_ID]["transform"],
        )
        self.assertEqual(
            canonical_entity_affine_v1(replayed["shapes"][NODE_ID]["transform"]),
            replay_operation["after"],
        )


if __name__ == "__main__":
    unittest.main()
