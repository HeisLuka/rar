#!/usr/bin/env python3
import tempfile
import unittest
from pathlib import Path

from store import PubHarnessStore
from query import PubHarnessQuery


class StoreTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.db = Path(self.tmp.name) / "pub-harness.db"
        self.store = PubHarnessStore(self.db)
        self.store.init_schema()

    def tearDown(self):
        self.store.close()
        self.tmp.cleanup()

    def test_upsert_get_and_fts(self):
        self.store.upsert_entity({
            "entity_type": "research_task",
            "entity_id": "PUB-T-575",
            "title": "KORVA provenance",
            "lifecycle": "Ready",
            "priority": "P2",
            "agent_gate": "runnable",
            "body_markdown": "classify parser lineage libmspub",
            "content_hydrated": True,
        })
        self.assertEqual(self.store.get("research_task", "PUB-T-575")["priority"], "P2")
        hits = self.store.text_search("libmspub")
        self.assertEqual(hits[0]["entity_id"], "PUB-T-575")

    def test_relation_replace(self):
        self.store.upsert_entity({
            "entity_type": "research_task",
            "entity_id": "PUB-T-1",
            "title": "one",
        })
        self.store.replace_relations(
            "research_task", "PUB-T-1", "depends_on",
            [("research_task", "PUB-T-2")]
        )
        rels = self.store.relations_from("research_task", "PUB-T-1", "depends_on")
        self.assertEqual(rels[0]["dst_id"], "PUB-T-2")

    def test_typed_query(self):
        self.store.upsert_entity({
            "entity_type": "research_task",
            "entity_id": "PUB-T-10",
            "title": "ten",
            "lifecycle": "Ready",
            "priority": "P0",
            "agent_gate": "runnable",
            "execution_owner": "Local Codex",
        })
        q = PubHarnessQuery(self.store)
        self.assertEqual(q.task_next(lane="local_research", owner="Local Codex")[0]["entity_id"], "PUB-T-10")


if __name__ == "__main__":
    unittest.main()
