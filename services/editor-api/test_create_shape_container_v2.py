#!/usr/bin/env python3
import copy
import unittest

from create_shape_container_v2 import (
    CreateShapeV2Error,
    apply_create_shape_v2,
)
from revision_store import RevisionKernel

DOCUMENT_ID = "doc:create-shape-v2"
SOURCE_HASH = "cd" * 32
NODE_ID = "01890f47-0c00-7abc-8def-0123456789ab"
PAINT = {
    "fill": {"visible": True, "color": {"r": 1, "g": 2, "b": 3}},
    "stroke": {"visible": True, "color": {"r": 4, "g": 5, "b": 6}, "width_emu": 12700},
}


def project():
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": SOURCE_HASH,
        "immutable_source_blob_sha256": "ef" * 32,
        "operations": [],
        "pages": {
            "page:1": {
                "authoring_enabled": True,
                "children": ["source:1"],
            }
        },
        "shapes": {
            "shape:existing": {
                "node_id": "shape:existing",
                "page_id": "page:1",
                "parent_id": "group:1",
                "kind": "shape",
            }
        },
        "text_frames": {},
        "picture_frames": {},
        "groups": {
            "group:1": {
                "node_id": "group:1",
                "page_id": "page:1",
                "parent_id": "page:1",
                "children": ["shape:existing"],
                "provenance": {"kind": "author_created"},
            }
        },
        "authored_stacks": {"page:1": ["group:1"]},
    }


def request(base_revision_id, *, destination_kind="page", placement=None, expected=None, insertion=None, op_id="v2-op-1"):
    if destination_kind == "page":
        destination = {"kind": "page", "id": "page:1"}
        placement = placement or {
            "status": "contained",
            "desired_effective_page_rect": {"x": 10, "y": 20, "width": 100, "height": 50},
            "destination_local_rect": {"x": 10, "y": 20, "width": 100, "height": 50},
        }
        expected = ["group:1"] if expected is None else expected
        insertion = insertion or "append_authored_front"
    else:
        destination = {"kind": "group", "id": "group:1"}
        placement = placement or {
            "status": "contained",
            "desired_effective_page_rect": {"x": 110, "y": 120, "width": 100, "height": 50},
            "destination_local_rect": {"x": 10, "y": 20, "width": 100, "height": 50},
        }
        expected = ["shape:existing"] if expected is None else expected
        insertion = insertion or "append_block_at_front"

    return {
        "protocol_version": "chaptera.create-shape-intent.v2",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": {
            "kind": "create_shape_v2",
            "node_id": NODE_ID,
            "page_id": "page:1",
            "destination": destination,
            "placement": copy.deepcopy(placement),
            "paint": copy.deepcopy(PAINT),
            "expected_order_lane": list(expected),
            "insertion_policy": insertion,
        },
    }


class CreateShapeContainerV2Tests(unittest.TestCase):
    def test_page_create_appends_authored_stack_and_keeps_direct_page_parent(self):
        base = project()
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(document_id=DOCUMENT_ID, source_hash=SOURCE_HASH, project=base)
        result = kernel.commit_create_shape_v2(request(baseline.revision_id), apply_create_shape_v2)

        current = kernel.current_revision(DOCUMENT_ID).project
        entity = current["shapes"][NODE_ID]
        self.assertEqual("page:1", entity["parent_id"])
        self.assertEqual({"x":10,"y":20,"width":100,"height":50}, entity["bounds"])
        self.assertEqual(["group:1", NODE_ID], current["authored_stacks"]["page:1"])
        self.assertEqual(["group:1"], result["canonical_operation"]["order_before"])
        self.assertEqual(["group:1", NODE_ID], result["canonical_operation"]["order_after"])
        self.assertEqual(["source:1"], current["pages"]["page:1"]["children"])

    def test_group_create_appends_group_children_only(self):
        base = project()
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(document_id=DOCUMENT_ID, source_hash=SOURCE_HASH, project=base)
        result = kernel.commit_create_shape_v2(
            request(baseline.revision_id, destination_kind="group"),
            apply_create_shape_v2,
        )
        current = kernel.current_revision(DOCUMENT_ID).project
        entity = current["shapes"][NODE_ID]
        self.assertEqual("group:1", entity["parent_id"])
        self.assertEqual({"x":10,"y":20,"width":100,"height":50}, entity["bounds"])
        self.assertEqual(["shape:existing", NODE_ID], current["groups"]["group:1"]["children"])
        self.assertEqual(["group:1"], current["authored_stacks"]["page:1"])
        self.assertEqual("append_block_at_front", result["canonical_operation"]["insertion_policy"])

    def test_refit_required_or_inexact_placement_is_rejected_before_mutation(self):
        base = project()
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(document_id=DOCUMENT_ID, source_hash=SOURCE_HASH, project=base)
        for status in ("destination_refit_required", "not_exactly_representable"):
            bad = request(baseline.revision_id, destination_kind="group", op_id=f"bad-{status}")
            bad["command"]["placement"]["status"] = status
            with self.subTest(status=status):
                with self.assertRaisesRegex(CreateShapeV2Error, "contained placement only"):
                    kernel.commit_create_shape_v2(bad, apply_create_shape_v2)
                self.assertNotIn(NODE_ID, kernel.current_revision(DOCUMENT_ID).project["shapes"])

    def test_stale_order_lane_and_collision_fail_closed(self):
        base = project()
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(document_id=DOCUMENT_ID, source_hash=SOURCE_HASH, project=base)
        stale = request(baseline.revision_id, destination_kind="group", expected=[])
        with self.assertRaisesRegex(CreateShapeV2Error, "stale"):
            kernel.commit_create_shape_v2(stale, apply_create_shape_v2)

        collision_base = project()
        collision_base["picture_frames"][NODE_ID] = {"kind":"picture_frame"}
        k2 = RevisionKernel()
        b2 = k2.register_baseline(document_id="doc:collision-v2", source_hash=SOURCE_HASH, project=collision_base)
        req = request(b2.revision_id)
        req["document_id"] = "doc:collision-v2"
        with self.assertRaisesRegex(CreateShapeV2Error, "collision"):
            k2.commit_create_shape_v2(req, apply_create_shape_v2)

    def test_group_destination_must_be_authored_and_page_consistent(self):
        base = project()
        base["groups"]["group:1"]["provenance"] = {"kind":"source_backed"}
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(document_id=DOCUMENT_ID, source_hash=SOURCE_HASH, project=base)
        with self.assertRaisesRegex(CreateShapeV2Error, "source-backed"):
            kernel.commit_create_shape_v2(
                request(baseline.revision_id, destination_kind="group"),
                apply_create_shape_v2,
            )

    def test_undo_redo_restores_same_identity_and_order(self):
        base = project()
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(document_id=DOCUMENT_ID, source_hash=SOURCE_HASH, project=base)
        accepted = kernel.commit_create_shape_v2(
            request(baseline.revision_id, destination_kind="group"),
            apply_create_shape_v2,
        )
        edited = copy.deepcopy(kernel.current_revision(DOCUMENT_ID).project)

        def history(_base, kind):
            if kind == "undo":
                return copy.deepcopy(base), []
            if kind == "redo":
                return copy.deepcopy(edited), []
            raise ValueError(kind)

        undo = kernel.commit_history_transition({
            "protocol_version":"chaptera.history-transition-intent.v1",
            "document_id":DOCUMENT_ID,
            "source_hash":SOURCE_HASH,
            "base_revision_id":accepted["revision_id"],
            "client_operation_id":"v2-undo",
            "command":{"kind":"undo"},
        }, history)
        self.assertNotIn(NODE_ID, kernel.current_revision(DOCUMENT_ID).project["shapes"])
        self.assertEqual(["shape:existing"], kernel.current_revision(DOCUMENT_ID).project["groups"]["group:1"]["children"])

        kernel.commit_history_transition({
            "protocol_version":"chaptera.history-transition-intent.v1",
            "document_id":DOCUMENT_ID,
            "source_hash":SOURCE_HASH,
            "base_revision_id":undo["revision_id"],
            "client_operation_id":"v2-redo",
            "command":{"kind":"redo"},
        }, history)
        current = kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(NODE_ID, current["shapes"][NODE_ID]["node_id"])
        self.assertEqual(["shape:existing", NODE_ID], current["groups"]["group:1"]["children"])


if __name__ == "__main__":
    unittest.main()
