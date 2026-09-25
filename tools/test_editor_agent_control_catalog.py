#!/usr/bin/env python3
from __future__ import annotations

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "packages" / "protocol" / "editor-agent-control" / "v1.catalog.json"

EXPECTED_COMMANDS = {
    "protocol.describe",
    "open",
    "document.describe",
    "pages.list",
    "stories.list",
    "story.inspect",
    "story.read_local",
    "scene.instances.list",
    "instance.inspect",
    "capabilities.get",
    "edit.apply",
    "undo",
    "redo",
    "project.save",
    "project.reopen",
    "loss.preview",
    "export",
    "snapshot.get",
    "trace.subscribe",
    "diagnostics.deep",
    "shutdown",
}


class AgentControlCatalogTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.catalog = json.loads(CATALOG.read_text(encoding="utf-8"))

    def test_catalog_identity_and_command_set_are_exact(self) -> None:
        self.assertEqual(
            self.catalog["schema"],
            "chaptera.agent-control.catalog.v1",
        )
        self.assertEqual(
            self.catalog["protocol_version"],
            "chaptera.agent-control.v1",
        )
        self.assertEqual(self.catalog["executable"], "chaptera-editor.exe")
        self.assertEqual(self.catalog["mode"], "--agent-v1")
        self.assertEqual(set(self.catalog["commands"]), EXPECTED_COMMANDS)

    def test_global_safety_laws_are_explicit(self) -> None:
        laws = self.catalog["global_laws"]
        self.assertIs(laws["source_pub_immutable"], True)
        self.assertIs(laws["native_pub_write"], False)
        self.assertEqual(laws["default_output"], "source_free")
        self.assertEqual(laws["projected_object_mutation"], "fail_closed")
        self.assertIs(laws["local_content_requires_explicit_consent"], True)
        self.assertIs(
            laws["local_diagnostic_file_reads_require_explicit_consent"],
            True,
        )

    def test_every_command_has_operational_discovery_fields(self) -> None:
        for name, command in self.catalog["commands"].items():
            self.assertIsInstance(command["requires_open_document"], bool, name)
            self.assertIsInstance(command["mutates_session"], bool, name)
            self.assertIsInstance(command["mutates_editor_state"], bool, name)
            self.assertIsInstance(command["reads_local_file"], bool, name)
            self.assertIsInstance(command["writes_local_file"], bool, name)
            self.assertEqual(
                command["request"]["required"][:2],
                ["request_id", "command"],
                name,
            )
            self.assertIn("optional", command["request"], name)
            self.assertIn("fields", command["request"], name)
            self.assertIsInstance(command["trace_event_kinds"], list, name)
            self.assertIsInstance(command["result_principal_fields"], list, name)

    def test_edit_apply_documents_both_v1_mutation_shapes(self) -> None:
        operation = self.catalog["commands"]["edit.apply"]["request"]["fields"]["operation"]
        variants = operation["one_of"]
        self.assertEqual(
            set(variants),
            {"replace_story_range", "move_node"},
        )
        self.assertEqual(
            variants["move_node"]["required"],
            ["kind", "instance_id", "x", "y"],
        )
        self.assertIn("expected_before", variants["replace_story_range"]["required"])
        self.assertEqual(
            self.catalog["commands"]["edit.apply"]["trace_event_kinds"],
            ["intent", "durable_commit", "projection_updated"],
        )

    def test_sensitive_reads_document_explicit_consent(self) -> None:
        story = self.catalog["commands"]["story.read_local"]
        self.assertEqual(story["consent"]["field"], "allow_content")
        self.assertIs(story["consent"]["required_value"], True)

        deep = self.catalog["commands"]["diagnostics.deep"]
        self.assertEqual(deep["consent"]["field"], "allow_local_file")
        self.assertIs(deep["consent"]["required_value"], True)
        self.assertIn("receipt_path", deep["request"]["optional"])
        self.assertIn("joined_receipt_path", deep["request"]["optional"])
        self.assertIn("native_join", deep["result_principal_fields"])

    def test_projected_instance_mutation_is_not_described_as_supported(self) -> None:
        instance = self.catalog["commands"]["instance.inspect"]["request"]["fields"][
            "instance_id"
        ]
        self.assertIn("direct_page_local", instance)
        move = self.catalog["commands"]["edit.apply"]["request"]["fields"]["operation"][
            "one_of"
        ]["move_node"]["instance_id"]
        self.assertIn("direct_page_local", move)


if __name__ == "__main__":
    unittest.main()
