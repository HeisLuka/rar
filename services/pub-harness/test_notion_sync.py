#!/usr/bin/env python3
import tempfile
import unittest
from pathlib import Path

from notion_sync import page_to_entity, prop_value, semantic_id
from store import PubHarnessStore


def rt(text):
    return [{"plain_text": text}]


class NotionSyncTests(unittest.TestCase):
    def test_property_decoder(self):
        self.assertEqual(prop_value({"type": "title", "title": rt("PUB-T-1")}), "PUB-T-1")
        self.assertEqual(
            prop_value({"type": "status", "status": {"name": "Ready"}}),
            "Ready",
        )
        self.assertEqual(
            prop_value({"type": "relation", "relation": [{"id": "abc"}]}),
            ["abc"],
        )

    def test_research_mapping(self):
        source = {
            "entity_type": "research_task",
            "id_property": "Task ID",
            "title_property": "Task",
            "fields": {"priority": "Priority"},
        }
        page = {
            "id": "page-1",
            "url": "https://notion.example/page-1",
            "created_time": "2026-09-24T00:00:00.000Z",
            "last_edited_time": "2026-09-24T01:00:00.000Z",
            "properties": {
                "Task ID": {"type": "rich_text", "rich_text": rt("PUB-T-1")},
                "Task": {"type": "title", "title": rt("Example task")},
                "Priority": {"type": "select", "select": {"name": "P0"}},
            },
        }
        self.assertEqual(semantic_id(page, source), "PUB-T-1")
        entity = page_to_entity(page, source)
        self.assertEqual(entity["title"], "Example task")
        self.assertEqual(entity["priority"], "P0")

    def test_notion_page_identity_is_queryable(self):
        with tempfile.TemporaryDirectory() as td:
            store = PubHarnessStore(Path(td) / "x.db")
            store.init_schema()
            store.upsert_entity({
                "entity_type": "research_task",
                "entity_id": "PUB-T-1",
                "notion_page_id": "page-1",
                "title": "Example",
            })
            row = store.conn.execute(
                "SELECT entity_id FROM entities WHERE entity_type=? AND notion_page_id=?",
                ("research_task", "page-1"),
            ).fetchone()
            self.assertEqual(row["entity_id"], "PUB-T-1")
            store.close()


if __name__ == "__main__":
    unittest.main()
