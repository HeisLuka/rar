#!/usr/bin/env python3
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from resolved_graph_scene_bridge_v1 import (
    ResolvedGraphSceneError,
    project_resolved_graph_scene,
)

MASTER_PAGE = "50000000-0000-4000-8000-000000000263"
PAGE_A = "50000000-0000-4000-8000-000000000266"
PAGE_B = "50000000-0000-4000-8000-000000000361"
PAGE_C = "50000000-0000-4000-8000-000000000406"
MASTER_NODE = "60000000-0000-4000-8000-000000000380"
LOCAL_NODE = "60000000-0000-4000-8000-000000000381"


def rect(x=100, y=200, width=300, height=400):
    return {"x": x, "y": y, "width": width, "height": height}


def transform():
    return {"a": "1", "b": "0", "c": "0", "d": "1", "tx": 0, "ty": 0}


def page(page_id, children):
    return {
        "id": page_id,
        "size": {"width": 9144000, "height": 11887200},
        "bleed": None,
        "margins": None,
        "children": list(children),
    }


def node(node_id, parent_id, bounds):
    return {
        "header": {
            "id": node_id,
            "parent_id": parent_id,
            "bounds": dict(bounds),
            "transform": transform(),
        },
        "payload": {},
    }


def graph():
    return {
        "document": {
            "source_hash": "a" * 64,
            "pages": [MASTER_PAGE, PAGE_A, PAGE_B, PAGE_C],
        },
        "pages": {
            MASTER_PAGE: page(MASTER_PAGE, [MASTER_NODE]),
            PAGE_A: page(PAGE_A, [LOCAL_NODE]),
            PAGE_B: page(PAGE_B, []),
            PAGE_C: page(PAGE_C, []),
        },
        "nodes": {
            MASTER_NODE: node(MASTER_NODE, MASTER_PAGE, rect(1000, 2000, 3000, 4000)),
            LOCAL_NODE: node(LOCAL_NODE, PAGE_A, rect(5000, 6000, 7000, 8000)),
        },
        "stories": {},
    }


def context():
    return {
        "schema_version": "chaptera.pub-projection-context.v1",
        "master_relations": [
            {
                "source_page_id": PAGE_A,
                "source_page_seq_num": 266,
                "master_page_id": MASTER_PAGE,
                "master_page_seq_num": 263,
            },
            {
                "source_page_id": PAGE_B,
                "source_page_seq_num": 361,
                "master_page_id": MASTER_PAGE,
                "master_page_seq_num": 263,
            },
            {
                "source_page_id": PAGE_C,
                "source_page_seq_num": 406,
                "master_page_id": MASTER_PAGE,
                "master_page_seq_num": 263,
            },
        ],
        "cmo_relations": [],
    }


class MasterPageSceneProjectionTests(unittest.TestCase):
    def test_one_master_origin_materializes_three_distinct_instances(self):
        scene = project_resolved_graph_scene(graph(), context=context())

        self.assertEqual(
            [PAGE_A, PAGE_B, PAGE_C],
            sorted(surface["origin"] for surface in scene["surfaces"]),
        )
        self.assertNotIn(
            MASTER_PAGE,
            [surface["origin"] for surface in scene["surfaces"]],
        )

        inherited = [
            item for item in scene["nodes"]
            if item.get("projection_kind") == "inherited_master"
        ]
        self.assertEqual(3, len(inherited))
        self.assertEqual({MASTER_NODE}, {item["origin"] for item in inherited})
        self.assertEqual(
            {PAGE_A, PAGE_B, PAGE_C},
            {item["parent_origin"] for item in inherited},
        )
        self.assertEqual(3, len({item["instance_id"] for item in inherited}))
        self.assertTrue(
            all(item["source_parent_origin"] == MASTER_PAGE for item in inherited)
        )

        # The source master node remains semantic provenance only; it is not
        # separately painted on the hidden master surface.
        direct_master = [
            item for item in scene["nodes"]
            if item["origin"] == MASTER_NODE
            and item.get("projection_kind") != "inherited_master"
        ]
        self.assertEqual([], direct_master)

    def test_direct_page_node_remains_direct_and_unaliased(self):
        scene = project_resolved_graph_scene(graph(), context=context())
        direct = [
            item for item in scene["nodes"]
            if item["origin"] == LOCAL_NODE
        ]
        self.assertEqual(1, len(direct))
        self.assertEqual(PAGE_A, direct[0]["parent_origin"])
        self.assertNotIn("instance_id", direct[0])
        self.assertNotIn("projection_kind", direct[0])

    def test_origin_mapping_carries_instance_identity_without_node_clones(self):
        scene = project_resolved_graph_scene(graph(), context=context())
        inherited = [
            item for item in scene["origin_mapping"]
            if item.get("projection_kind") == "inherited_master"
        ]
        self.assertEqual(3, len(inherited))
        self.assertEqual({MASTER_NODE}, {item["authoring_origin"] for item in inherited})
        self.assertEqual(3, len({item["resolved_instance_id"] for item in inherited}))

    def test_missing_master_target_fails_closed(self):
        broken = context()
        broken["master_relations"][0]["master_page_id"] = (
            "50000000-0000-4000-8000-000000000999"
        )
        with self.assertRaisesRegex(ResolvedGraphSceneError, "target page"):
            project_resolved_graph_scene(graph(), context=broken)

    def test_context_schema_and_cmo_semantics_remain_strict(self):
        broken = context()
        broken["schema_version"] = "chaptera.pub-projection-context.v2"
        with self.assertRaisesRegex(ResolvedGraphSceneError, "schema_version"):
            project_resolved_graph_scene(graph(), context=broken)

        broken = context()
        broken["cmo_relations"] = [{"not": "implemented"}]
        with self.assertRaisesRegex(ResolvedGraphSceneError, "cmo_relations"):
            project_resolved_graph_scene(graph(), context=broken)


if __name__ == "__main__":
    unittest.main()
