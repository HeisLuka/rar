#!/usr/bin/env python3
import copy
import json
import unittest

from create_shape_container_v2 import apply_create_shape_v2
from delete_node_v1 import DeleteNodeV1Error, execute_delete_node_v1
from revision_store import RevisionKernel, canonical_json, hash_id, project_hash


DOCUMENT_ID = "doc:delete-node-current"
SOURCE_HASH = "a" * 64
SOURCE_BLOB_SHA256 = "b" * 64
PAGE_ID = "page:1"
NODE_ID = "01890f47-0c00-7abc-8def-0123456789ab"
SECOND_NODE_ID = "01890f47-0c01-7abc-8def-0123456789ab"
GROUP_ID = "group:1"
PAINT = {
    "fill": {"visible": True, "color": {"r": 10, "g": 20, "b": 30}},
    "stroke": {
        "visible": True,
        "color": {"r": 40, "g": 50, "b": 60},
        "width_emu": 12700,
    },
}


def baseline_project():
    return {
        "schema_version": "pub-editor-v0.6",
        "source_hash": SOURCE_HASH,
        "immutable_source_blob_sha256": SOURCE_BLOB_SHA256,
        "operations": [],
        "pages": {
            PAGE_ID: {
                "authoring_enabled": True,
                "children": ["source:existing"],
            }
        },
        "shapes": {},
        "text_frames": {},
        "picture_frames": {},
        "groups": {
            GROUP_ID: {
                "node_id": GROUP_ID,
                "kind": "group",
                "page_id": PAGE_ID,
                "parent_id": PAGE_ID,
                "children": [],
                "provenance": {"kind": "author_created"},
            }
        },
        "authored_stacks": {PAGE_ID: [GROUP_ID]},
    }


def create_page_shape_request(base_revision_id, *, node_id=NODE_ID, op_id="create-for-delete"):
    return {
        "protocol_version": "chaptera.create-shape-intent.v2",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": {
            "kind": "create_shape_v2",
            "node_id": node_id,
            "page_id": PAGE_ID,
            "destination": {"kind": "page", "id": PAGE_ID},
            "placement": {
                "status": "contained",
                "desired_effective_page_rect": {
                    "x": 100,
                    "y": 200,
                    "width": 300,
                    "height": 400,
                },
                "destination_local_rect": {
                    "x": 100,
                    "y": 200,
                    "width": 300,
                    "height": 400,
                },
            },
            "paint": copy.deepcopy(PAINT),
            "expected_order_lane": [GROUP_ID],
            "insertion_policy": "append_authored_front",
        },
    }


def create_group_shape_request(base_revision_id, *, op_id="create-group-owned"):
    return {
        "protocol_version": "chaptera.create-shape-intent.v2",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": {
            "kind": "create_shape_v2",
            "node_id": NODE_ID,
            "page_id": PAGE_ID,
            "destination": {"kind": "group", "id": GROUP_ID},
            "placement": {
                "status": "contained",
                "desired_effective_page_rect": {
                    "x": 100,
                    "y": 200,
                    "width": 300,
                    "height": 400,
                },
                "destination_local_rect": {
                    "x": 10,
                    "y": 20,
                    "width": 300,
                    "height": 400,
                },
            },
            "paint": copy.deepcopy(PAINT),
            "expected_order_lane": [],
            "insertion_policy": "append_block_at_front",
        },
    }


def delete_request(project, base_revision_id, *, op_id="delete-node-current"):
    entity = copy.deepcopy(project["shapes"][NODE_ID])
    lane = project["authored_stacks"][PAGE_ID]
    return {
        "protocol_version": "chaptera.delete-node-intent.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": {
            "kind": "delete_node",
            "node_id": NODE_ID,
            "expected_state_id": hash_id(entity),
            "expected_parent_id": PAGE_ID,
            "expected_child_index": lane.index(NODE_ID),
        },
    }


class DeleteNodeCurrentV1Tests(unittest.TestCase):
    def setUp(self):
        self.base = baseline_project()
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.base,
        )
        self.created = self.kernel.commit_create_shape_v2(
            create_page_shape_request(self.baseline.revision_id),
            apply_create_shape_v2,
        )
        self.created_project = copy.deepcopy(
            self.kernel.current_revision(DOCUMENT_ID).project
        )

    def test_delete_current_page_owned_shape_removes_shape_and_authored_lane_only(self):
        request = delete_request(
            self.created_project,
            self.created["revision_id"],
        )
        accepted = self.kernel.commit_delete_node(request)

        self.assertEqual("chaptera.commit-accepted.v1", accepted["protocol_version"])
        operation = accepted["canonical_operation"]
        self.assertEqual("delete_node", operation["kind"])
        self.assertEqual(NODE_ID, operation["node_id"])
        self.assertEqual(PAGE_ID, operation["parent_id"])
        self.assertEqual(1, operation["child_index"])
        self.assertEqual([GROUP_ID, NODE_ID], operation["authored_lane_before"])
        self.assertEqual([GROUP_ID], operation["authored_lane_after"])
        self.assertEqual(
            self.created_project["shapes"][NODE_ID],
            operation["before_entity"],
        )
        self.assertEqual(
            hash_id(self.created_project["shapes"][NODE_ID]),
            operation["before_state_id"],
        )

        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertNotIn(NODE_ID, current["shapes"])
        self.assertEqual([GROUP_ID], current["authored_stacks"][PAGE_ID])
        self.assertEqual(
            ["source:existing"],
            current["pages"][PAGE_ID]["children"],
            "DeleteNode must not reinterpret imported Page.children as authored order",
        )
        self.assertEqual(self.base["groups"], current["groups"])
        self.assertEqual(SOURCE_HASH, current["source_hash"])
        self.assertEqual(
            SOURCE_BLOB_SHA256,
            current["immutable_source_blob_sha256"],
        )
        self.assertEqual(
            "intentional_effective_deletion",
            accepted["consequences"][0]["note"],
        )

    def test_exact_retry_is_idempotent_and_does_not_delete_twice(self):
        request = delete_request(
            self.created_project,
            self.created["revision_id"],
            op_id="delete-retry",
        )
        first = self.kernel.commit_delete_node(copy.deepcopy(request))
        second = self.kernel.commit_delete_node(copy.deepcopy(request))
        self.assertEqual(first, second)
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertNotIn(NODE_ID, current["shapes"])
        self.assertEqual(2, len(current["operations"]))

    def test_stale_revision_rejected_before_second_delete(self):
        first = self.kernel.commit_delete_node(
            delete_request(
                self.created_project,
                self.created["revision_id"],
                op_id="delete-first",
            )
        )
        stale = self.kernel.commit_delete_node(
            delete_request(
                self.created_project,
                self.created["revision_id"],
                op_id="delete-stale",
            )
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(first["revision_id"], stale["current_revision_id"])

    def test_stale_entity_and_authored_lane_order_fail_closed(self):
        bad_state = delete_request(
            self.created_project,
            self.created["revision_id"],
            op_id="delete-bad-state",
        )
        bad_state["command"]["expected_state_id"] = "sha256:" + "f" * 64
        with self.assertRaisesRegex(DeleteNodeV1Error, "stale_delete_node_state"):
            self.kernel.commit_delete_node(bad_state)

        kernel = RevisionKernel()
        altered = copy.deepcopy(self.created_project)
        altered["authored_stacks"][PAGE_ID] = [NODE_ID, GROUP_ID]
        base = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=altered,
        )
        bad_order = delete_request(
            altered,
            base.revision_id,
            op_id="delete-bad-order",
        )
        bad_order["command"]["expected_child_index"] = 1
        with self.assertRaisesRegex(DeleteNodeV1Error, "stale_delete_node_order"):
            kernel.commit_delete_node(bad_order)

    def test_source_backed_and_group_owned_shapes_are_not_admitted(self):
        source_backed = copy.deepcopy(self.created_project)
        source_backed["shapes"][NODE_ID]["provenance"] = {"kind": "source_backed"}
        kernel = RevisionKernel()
        base = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=source_backed,
        )
        req = delete_request(source_backed, base.revision_id, op_id="delete-source")
        with self.assertRaisesRegex(DeleteNodeV1Error, "unsupported_delete_node_class"):
            kernel.commit_delete_node(req)

        group_kernel = RevisionKernel()
        group_base = group_kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=baseline_project(),
        )
        group_created = group_kernel.commit_create_shape_v2(
            create_group_shape_request(group_base.revision_id),
            apply_create_shape_v2,
        )
        group_project = group_kernel.current_revision(DOCUMENT_ID).project
        group_req = {
            "protocol_version": "chaptera.delete-node-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": group_created["revision_id"],
            "client_operation_id": "delete-group-owned",
            "command": {
                "kind": "delete_node",
                "node_id": NODE_ID,
                "expected_state_id": hash_id(group_project["shapes"][NODE_ID]),
                "expected_parent_id": GROUP_ID,
                "expected_child_index": 0,
            },
        }
        with self.assertRaises(DeleteNodeV1Error):
            group_kernel.commit_delete_node(group_req)

    def test_browser_cannot_supply_entity_or_authored_lane_authority(self):
        for field, value in (
            ("before_entity", copy.deepcopy(self.created_project["shapes"][NODE_ID])),
            ("authored_lane_before", [GROUP_ID, NODE_ID]),
            ("cascade", True),
            ("delete_story", True),
        ):
            request = delete_request(
                self.created_project,
                self.created["revision_id"],
                op_id=f"delete-extra-{field}",
            )
            request["command"][field] = value
            with self.subTest(field=field):
                with self.assertRaisesRegex(
                    DeleteNodeV1Error,
                    "non-intent/authoritative",
                ):
                    self.kernel.commit_delete_node(request)

    def test_forged_executor_cannot_change_entity_or_authored_lane(self):
        request = delete_request(
            self.created_project,
            self.created["revision_id"],
            op_id="delete-forged",
        )

        def forged(base_project, command):
            operation, project, consequences = execute_delete_node_v1(
                base_project,
                command,
            )
            operation["authored_lane_after"] = [GROUP_ID, NODE_ID]
            return operation, project, consequences

        with self.assertRaises(ValueError):
            self.kernel.commit_delete_node(request, forged)
        self.assertEqual(
            self.created["revision_id"],
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_undo_redo_restore_exact_shape_and_authored_order(self):
        accepted = self.kernel.commit_delete_node(
            delete_request(
                self.created_project,
                self.created["revision_id"],
                op_id="delete-history",
            )
        )
        deleted = copy.deepcopy(self.kernel.current_revision(DOCUMENT_ID).project)
        created = copy.deepcopy(self.created_project)

        def history(_base, kind):
            if kind == "undo":
                return copy.deepcopy(created), []
            if kind == "redo":
                return copy.deepcopy(deleted), []
            raise ValueError(kind)

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "delete-current-undo",
                "command": {"kind": "undo"},
            },
            history,
        )
        restored = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(
            created["shapes"][NODE_ID],
            restored["shapes"][NODE_ID],
        )
        self.assertEqual(
            [GROUP_ID, NODE_ID],
            restored["authored_stacks"][PAGE_ID],
        )

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "delete-current-redo",
                "command": {"kind": "redo"},
            },
            history,
        )
        self.assertEqual(
            deleted,
            self.kernel.current_revision(DOCUMENT_ID).project,
        )

    def test_save_reopen_preserves_intentional_absence_and_project_hash(self):
        self.kernel.commit_delete_node(
            delete_request(
                self.created_project,
                self.created["revision_id"],
                op_id="delete-save-reopen",
            )
        )
        deleted = copy.deepcopy(self.kernel.current_revision(DOCUMENT_ID).project)
        saved = json.loads(canonical_json(deleted).decode("utf-8"))

        reopened = RevisionKernel()
        reopened_base = reopened.register_baseline(
            document_id="doc:delete-node-reopened",
            source_hash=SOURCE_HASH,
            project=saved,
        )
        self.assertEqual(project_hash(deleted), reopened_base.project_hash)
        self.assertNotIn(NODE_ID, reopened_base.project["shapes"])
        self.assertEqual([GROUP_ID], reopened_base.project["authored_stacks"][PAGE_ID])
        self.assertEqual(
            SOURCE_BLOB_SHA256,
            reopened_base.project["immutable_source_blob_sha256"],
        )


if __name__ == "__main__":
    unittest.main()
