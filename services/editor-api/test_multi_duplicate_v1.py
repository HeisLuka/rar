import copy
import unittest

from create_shape_v1 import new_uuid7_node_id_v1
from duplicate_rectangle_v1 import DUPLICATE_OFFSET_EMU_V1, DUPLICATE_PLACEMENT_POLICY_V1
from multi_duplicate_v1 import (
    MultiDuplicateV1Error,
    execute_multi_duplicate_v1,
)
from revision_store import RevisionKernel

DOC = "multi-duplicate-doc"
SOURCE = "8" * 64
PAGE = "page:1"
A = new_uuid7_node_id_v1(now_ms=1_700_000_000_000, random_bits=1)
B = new_uuid7_node_id_v1(now_ms=1_700_000_000_001, random_bits=2)
A2 = new_uuid7_node_id_v1(now_ms=1_700_000_000_010, random_bits=10)
B2 = new_uuid7_node_id_v1(now_ms=1_700_000_000_011, random_bits=11)
A3 = new_uuid7_node_id_v1(now_ms=1_700_000_000_020, random_bits=20)
B3 = new_uuid7_node_id_v1(now_ms=1_700_000_000_021, random_bits=21)

def shape(node_id, x, y):
    return {
        "node_id": node_id,
        "kind": "shape",
        "shape_kind": "rectangle",
        "page_id": PAGE,
        "parent_id": PAGE,
        "bounds": {"x": x, "y": y, "width": 300, "height": 400},
        "transform": {"kind": "identity"},
        "paint": {
            "fill": {"visible": True, "color": {"r": 10, "g": 20, "b": 30}},
            "stroke": {
                "visible": True,
                "color": {"r": 40, "g": 50, "b": 60},
                "width_emu": 12700,
            },
            "provenance": {"kind": "author_created"},
        },
        "provenance": {"kind": "author_created"},
    }

def project():
    return {
        "schema_version": "pub-editor-v0.7",
        "source_hash": SOURCE,
        "operations": [],
        "pages": {PAGE: {"authoring_enabled": True}},
        "shapes": {A: shape(A, 100, 200), B: shape(B, 900, -300)},
    }

def command(ids=(A2, B2), primary=B):
    return {
        "kind": "duplicate_selection_set",
        "source_node_ids": sorted([A, B]),
        "primary_source_node_id": primary,
        "identity_map": [
            {"source_node_id": source, "destination_node_id": dest}
            for source, dest in zip(sorted([A, B]), ids, strict=True)
        ],
        "placement_policy": DUPLICATE_PLACEMENT_POLICY_V1,
    }

def request(base, op_id, cmd=None):
    return {
        "protocol_version": "chaptera.multi-duplicate-intent.v1",
        "document_id": DOC,
        "source_hash": SOURCE,
        "base_revision_id": base,
        "client_operation_id": op_id,
        "command": copy.deepcopy(cmd or command()),
    }

class MultiDuplicateTests(unittest.TestCase):
    def test_duplicate_is_one_fragment_set_paste_and_preserves_offsets(self):
        before = project()
        op, after, consequences = execute_multi_duplicate_v1(before, command())
        self.assertEqual("paste_fragment_set", op["kind"])
        self.assertEqual(1, len(after["operations"]))
        self.assertEqual(op, after["operations"][0])
        self.assertEqual(
            before["shapes"][B]["bounds"]["x"] - before["shapes"][A]["bounds"]["x"],
            after["shapes"][B2]["bounds"]["x"] - after["shapes"][A2]["bounds"]["x"],
        )
        self.assertEqual(
            before["shapes"][B]["bounds"]["y"] - before["shapes"][A]["bounds"]["y"],
            after["shapes"][B2]["bounds"]["y"] - after["shapes"][A2]["bounds"]["y"],
        )
        self.assertEqual(
            before["shapes"][A]["bounds"]["x"] + DUPLICATE_OFFSET_EMU_V1,
            after["shapes"][A2]["bounds"]["x"],
        )
        self.assertEqual(before["shapes"][A]["paint"], after["shapes"][A2]["paint"])
        self.assertEqual(before["shapes"][B]["paint"], after["shapes"][B2]["paint"])
        selection = consequences[-1]["note"]
        self.assertEqual(sorted([A2, B2]), selection["selected_node_ids"])
        self.assertEqual(B2, selection["primary_node_id"])

    def test_identity_map_and_source_selection_fail_closed_atomically(self):
        before = project()
        bad = command(ids=(A2, A2))
        with self.assertRaisesRegex(MultiDuplicateV1Error, "unique"):
            execute_multi_duplicate_v1(before, bad)
        self.assertEqual(set(before["shapes"]), {A, B})

        bad = command()
        bad["source_node_ids"].reverse()
        with self.assertRaisesRegex(MultiDuplicateV1Error, "normalized"):
            execute_multi_duplicate_v1(before, bad)

        bad = command(primary="not-selected")
        with self.assertRaisesRegex(MultiDuplicateV1Error, "primary"):
            execute_multi_duplicate_v1(before, bad)

    def test_mixed_or_unsupported_selection_commits_nothing(self):
        before = project()
        before["shapes"][B]["parent_id"] = "group:1"
        snapshot = copy.deepcopy(before)
        with self.assertRaisesRegex(MultiDuplicateV1Error, "direct page-owned"):
            execute_multi_duplicate_v1(before, command())
        self.assertEqual(snapshot, before)

    def test_revision_retry_stale_and_repeat_identity_map(self):
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id=DOC, source_hash=SOURCE, project=project()
        )
        req = request(baseline.revision_id, "multi-dup-1")
        first = kernel.commit_multi_duplicate(copy.deepcopy(req))
        retry = kernel.commit_multi_duplicate(copy.deepcopy(req))
        self.assertEqual(first, retry)
        self.assertEqual(1, len(kernel.current_revision(DOC).project["operations"]))

        stale = kernel.commit_multi_duplicate(
            request(baseline.revision_id, "multi-dup-stale", command(ids=(A3, B3)))
        )
        self.assertEqual("stale_revision", stale["code"])

        current = kernel.current_revision(DOC)
        second = kernel.commit_multi_duplicate(
            request(current.revision_id, "multi-dup-2", command(ids=(A3, B3)))
        )
        self.assertEqual("chaptera.commit-accepted.v1", second["protocol_version"])
        state = kernel.current_revision(DOC).project
        for node_id in (A2, B2, A3, B3):
            self.assertIn(node_id, state["shapes"])

if __name__ == "__main__":
    unittest.main()
