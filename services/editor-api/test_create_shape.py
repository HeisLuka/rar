#!/usr/bin/env python3
import copy
import json
import unittest
import uuid

from create_shape_v1 import (
    CreateShapeError,
    apply_create_shape_v1,
    new_uuid7_node_id_v1,
    validate_uuid7_node_id_v1,
)
from revision_store import RevisionKernel, canonical_json, project_hash


DOCUMENT_ID = "doc:create-shape"
SOURCE_HASH = "cd" * 32
SOURCE_BLOB_SHA256 = "ef" * 32
NODE_ID = "01890f47-0c00-7abc-8def-0123456789ab"
SECOND_NODE_ID = "01890f47-0c01-7abc-8def-0123456789ab"

BOUNDS = {"x": -100, "y": 200, "width": 300, "height": 400}
PAINT = {
    "fill": {
        "visible": True,
        "color": {"r": 0x11, "g": 0x22, "b": 0x33},
    },
    "stroke": {
        "visible": True,
        "color": {"r": 0x44, "g": 0x55, "b": 0x66},
        "width_emu": 12_700,
    },
}


class CreateShapeCommitTests(unittest.TestCase):
    def setUp(self):
        self.project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "immutable_source_blob_sha256": SOURCE_BLOB_SHA256,
            "operations": [],
            "pages": {
                "page:1": {
                    "authoring_enabled": True,
                    "children": ["source:existing"],
                }
            },
            "shapes": {},
            "text_frames": {},
            "picture_frames": {},
            "groups": {},
        }
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )

    def request(self, op_id, *, node_id=NODE_ID, page_id="page:1", bounds=None, paint=None, base=None):
        return {
            "protocol_version": "chaptera.create-shape-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "create_shape",
                "node_id": node_id,
                "page_id": page_id,
                "bounds": copy.deepcopy(bounds or BOUNDS),
                "paint": copy.deepcopy(paint or PAINT),
            },
        }

    def test_uuid7_generator_is_valid_and_injectable(self):
        generated = new_uuid7_node_id_v1(
            now_ms=0x01890F470C00,
            random_bits=(0xABC << 62) | 0x123456789AB,
        )
        validate_uuid7_node_id_v1(generated)
        parsed = uuid.UUID(generated)
        self.assertEqual(7, parsed.version)
        self.assertEqual(uuid.RFC_4122, parsed.variant)
        self.assertEqual(
            generated,
            new_uuid7_node_id_v1(
                now_ms=0x01890F470C00,
                random_bits=(0xABC << 62) | 0x123456789AB,
            ),
        )

    def test_create_shape_commits_one_direct_page_owned_identity_rectangle(self):
        result = self.kernel.commit_create_shape(
            self.request("create-shape-00000001"),
            apply_create_shape_v1,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).project
        entity = current["shapes"][NODE_ID]

        self.assertEqual("shape", entity["kind"])
        self.assertEqual("rectangle", entity["shape_kind"])
        self.assertEqual("page:1", entity["page_id"])
        self.assertEqual("page:1", entity["parent_id"])
        self.assertEqual(BOUNDS, entity["bounds"])
        self.assertEqual({"kind": "identity"}, entity["transform"])
        self.assertEqual({"kind": "author_created"}, entity["provenance"])
        self.assertEqual(
            {
                "fill": PAINT["fill"],
                "stroke": PAINT["stroke"],
                "provenance": {"kind": "author_created"},
            },
            entity["paint"],
        )
        self.assertNotIn("source_ref", json.dumps(entity))
        self.assertEqual(
            ["source:existing"],
            current["pages"]["page:1"]["children"],
            "CreateShape must not invent z-order/page-child ordering",
        )
        self.assertEqual(SOURCE_HASH, current["source_hash"])
        self.assertEqual(SOURCE_BLOB_SHA256, current["immutable_source_blob_sha256"])
        self.assertEqual("create_shape", result["canonical_operation"]["kind"])
        self.assertEqual(NODE_ID, result["canonical_operation"]["node_id"])

    def test_exact_retry_is_idempotent(self):
        request = self.request("create-shape-00000002")
        first = self.kernel.commit_create_shape(copy.deepcopy(request), apply_create_shape_v1)
        second = self.kernel.commit_create_shape(copy.deepcopy(request), apply_create_shape_v1)
        self.assertEqual(first, second)
        self.assertEqual(
            1,
            len(self.kernel.current_revision(DOCUMENT_ID).project["operations"]),
        )

    def test_stale_revision_is_rejected_before_second_create(self):
        first = self.kernel.commit_create_shape(
            self.request("create-shape-00000003"),
            apply_create_shape_v1,
        )
        stale = self.kernel.commit_create_shape(
            self.request(
                "create-shape-00000004",
                node_id=SECOND_NODE_ID,
                base=self.baseline.revision_id,
            ),
            apply_create_shape_v1,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(first["revision_id"], stale["current_revision_id"])
        self.assertNotIn(
            SECOND_NODE_ID,
            self.kernel.current_revision(DOCUMENT_ID).project["shapes"],
        )

    def test_invalid_page_and_disabled_page_fail_without_revision_move(self):
        with self.assertRaisesRegex(CreateShapeError, "invalid_create_shape_page"):
            self.kernel.commit_create_shape(
                self.request("create-shape-00000005", page_id="page:missing"),
                apply_create_shape_v1,
            )

        disabled = copy.deepcopy(self.project)
        disabled["pages"]["page:1"]["authoring_enabled"] = False
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:disabled-page",
            source_hash=SOURCE_HASH,
            project=disabled,
        )
        request = self.request("create-shape-disabled-0001")
        request["document_id"] = "doc:disabled-page"
        request["base_revision_id"] = baseline.revision_id
        with self.assertRaisesRegex(CreateShapeError, "not_authorable"):
            kernel.commit_create_shape(request, apply_create_shape_v1)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:disabled-page").revision_id,
        )

    def test_id_collision_fails_across_known_entity_registries(self):
        collision = copy.deepcopy(self.project)
        collision["text_frames"][NODE_ID] = {"kind": "text_frame"}
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:collision",
            source_hash=SOURCE_HASH,
            project=collision,
        )
        request = self.request("create-shape-collision-0001")
        request["document_id"] = "doc:collision"
        request["base_revision_id"] = baseline.revision_id
        with self.assertRaisesRegex(CreateShapeError, "node_id_collision"):
            kernel.commit_create_shape(request, apply_create_shape_v1)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:collision").revision_id,
        )

    def test_invalid_uuid_bounds_and_paint_fail_before_mutation(self):
        with self.assertRaisesRegex(CreateShapeError, "UUIDv7"):
            self.kernel.commit_create_shape(
                self.request(
                    "create-shape-bad-id-0001",
                    node_id="01890f47-0c00-4abc-8def-0123456789ab",
                ),
                apply_create_shape_v1,
            )

        with self.assertRaisesRegex(CreateShapeError, "positive"):
            self.kernel.commit_create_shape(
                self.request(
                    "create-shape-bad-bounds-0001",
                    bounds={"x": 0, "y": 0, "width": 0, "height": 1},
                ),
                apply_create_shape_v1,
            )

        bad_paint = copy.deepcopy(PAINT)
        bad_paint["stroke"]["width_emu"] = 0
        with self.assertRaisesRegex(CreateShapeError, "positive safe EMU"):
            self.kernel.commit_create_shape(
                self.request("create-shape-bad-paint-0001", paint=bad_paint),
                apply_create_shape_v1,
            )

        missing_stroke = {"fill": copy.deepcopy(PAINT["fill"])}
        with self.assertRaisesRegex(CreateShapeError, "explicitly contain fill/stroke"):
            self.kernel.commit_create_shape(
                self.request("create-shape-missing-paint-0001", paint=missing_stroke),
                apply_create_shape_v1,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_browser_cannot_supply_parent_transform_or_provenance(self):
        for field, value in (
            ("parent_id", "page:forged"),
            ("transform", {"kind": "rotate"}),
            ("provenance", {"kind": "source_backed"}),
            ("source_ref", {"carrier": "forged"}),
        ):
            request = self.request(f"create-shape-extra-{field}")
            request["command"][field] = value
            with self.subTest(field=field):
                with self.assertRaisesRegex(CreateShapeError, "non-intent"):
                    self.kernel.commit_create_shape(request, apply_create_shape_v1)

    def test_executor_cannot_forge_canonical_identity_geometry_or_provenance(self):
        mutations = (
            ("parent_id", "page:other"),
            ("shape_kind", "ellipse"),
            ("transform", {"kind": "rotate"}),
            ("provenance", {"kind": "source_backed"}),
        )
        for field, value in mutations:
            def forged(base_project, command, field=field, value=value):
                operation, project, consequences = apply_create_shape_v1(base_project, command)
                operation[field] = copy.deepcopy(value)
                return operation, project, consequences

            kernel = RevisionKernel()
            baseline = kernel.register_baseline(
                document_id=DOCUMENT_ID,
                source_hash=SOURCE_HASH,
                project=self.project,
            )
            request = self.request(f"create-shape-forged-{field}")
            request["base_revision_id"] = baseline.revision_id
            with self.subTest(field=field):
                with self.assertRaises(ValueError):
                    kernel.commit_create_shape(request, forged)
                self.assertEqual(
                    baseline.revision_id,
                    kernel.current_revision(DOCUMENT_ID).revision_id,
                )

    def test_undo_redo_replay_and_save_reopen_preserve_same_uuid_and_state(self):
        accepted = self.kernel.commit_create_shape(
            self.request("create-shape-00000006"),
            apply_create_shape_v1,
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
                "client_operation_id": "create-shape-undo-0001",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertNotIn(
            NODE_ID,
            self.kernel.current_revision(DOCUMENT_ID).project["shapes"],
        )

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "create-shape-redo-0001",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(
            accepted_project["shapes"][NODE_ID],
            self.kernel.current_revision(DOCUMENT_ID).project["shapes"][NODE_ID],
        )

        saved = json.loads(canonical_json(accepted_project).decode("utf-8"))
        reopened = RevisionKernel()
        reopened_baseline = reopened.register_baseline(
            document_id="doc:create-shape-reopened",
            source_hash=SOURCE_HASH,
            project=saved,
        )
        self.assertEqual(project_hash(accepted_project), reopened_baseline.project_hash)
        self.assertEqual(NODE_ID, reopened_baseline.project["shapes"][NODE_ID]["node_id"])

        canonical = accepted["canonical_operation"]
        replay_command = {
            "kind": "create_shape",
            "node_id": canonical["node_id"],
            "page_id": canonical["page_id"],
            "bounds": canonical["bounds"],
            "paint": {
                "fill": canonical["paint"]["fill"],
                "stroke": canonical["paint"]["stroke"],
            },
        }
        replay_operation, replayed, _ = apply_create_shape_v1(
            copy.deepcopy(self.project),
            replay_command,
        )
        self.assertEqual(canonical, replay_operation)
        self.assertEqual(
            accepted_project["shapes"][NODE_ID],
            replayed["shapes"][NODE_ID],
        )


if __name__ == "__main__":
    unittest.main()
