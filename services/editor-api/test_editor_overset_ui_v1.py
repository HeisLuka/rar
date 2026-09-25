#!/usr/bin/env python3
import unittest

from editor_overset_ui_v1 import (
    EditorOversetUiError,
    project_editor_overset_ui_v1,
)


STORY_HASH = "sha256:" + "a" * 64
LAYOUT_HASH = "sha256:" + "b" * 64


def state(kind: str):
    if kind == "layout_unknown":
        return {
            "story_hash": STORY_HASH,
            "scalar_count": 37,
            "state": kind,
            "reason_code": "layout.environment_unavailable",
            "environment_authoritative": False,
            "layout_environment_hash": None,
        }
    return {
        "story_hash": STORY_HASH,
        "scalar_count": 37,
        "state": kind,
        "reason_code": None,
        "environment_authoritative": True,
        "layout_environment_hash": LAYOUT_HASH,
    }


class EditorOversetUiV1Tests(unittest.TestCase):
    def test_fits_is_quiet_and_authoritative(self):
        result = project_editor_overset_ui_v1(state("fits"))
        self.assertEqual("fits", result.state)
        self.assertFalse(result.frame_marker_visible)
        self.assertIsNone(result.frame_marker_kind)
        self.assertEqual("ok", result.status_severity)
        self.assertTrue(result.environment_authoritative)
        self.assertTrue(result.full_story_retained)

    def test_overset_is_visible_and_explains_full_story_is_retained(self):
        result = project_editor_overset_ui_v1(state("overset"))
        self.assertTrue(result.frame_marker_visible)
        self.assertEqual("overset", result.frame_marker_kind)
        self.assertEqual("warning", result.status_severity)
        self.assertIn("not placed", result.status_message)
        self.assertIn("full canonical Story is retained", result.status_message)
        self.assertTrue(result.full_story_retained)

    def test_layout_unknown_is_visible_non_authoritative_and_never_green(self):
        result = project_editor_overset_ui_v1(state("layout_unknown"))
        self.assertTrue(result.frame_marker_visible)
        self.assertEqual("layout_unknown", result.frame_marker_kind)
        self.assertEqual("unknown", result.status_severity)
        self.assertNotEqual("ok", result.status_severity)
        self.assertFalse(result.environment_authoritative)
        self.assertEqual("layout.environment_unavailable", result.reason_code)
        self.assertIn("No fit or overflow result is being guessed", result.status_message)

    def test_edit_refresh_is_pure_projection_of_new_canonical_state(self):
        before = project_editor_overset_ui_v1(state("fits"))
        after = project_editor_overset_ui_v1(state("overset"))
        self.assertFalse(before.frame_marker_visible)
        self.assertTrue(after.frame_marker_visible)
        self.assertEqual("overset", after.state)

    def test_save_reopen_same_state_projects_identically(self):
        canonical = state("overset")
        first = project_editor_overset_ui_v1(canonical)
        reopened = project_editor_overset_ui_v1(dict(canonical))
        self.assertEqual(first, reopened)

    def test_rejects_overset_without_authoritative_environment(self):
        value = state("overset")
        value["environment_authoritative"] = False
        with self.assertRaisesRegex(EditorOversetUiError, "authoritative"):
            project_editor_overset_ui_v1(value)

    def test_rejects_fits_without_layout_hash(self):
        value = state("fits")
        value["layout_environment_hash"] = None
        with self.assertRaisesRegex(EditorOversetUiError, "layout_environment_hash"):
            project_editor_overset_ui_v1(value)

    def test_rejects_layout_unknown_that_claims_authority(self):
        value = state("layout_unknown")
        value["environment_authoritative"] = True
        with self.assertRaisesRegex(EditorOversetUiError, "non-authoritative"):
            project_editor_overset_ui_v1(value)

    def test_rejects_unknown_state_instead_of_guessing(self):
        value = state("fits")
        value["state"] = "maybe"
        with self.assertRaisesRegex(EditorOversetUiError, "unsupported"):
            project_editor_overset_ui_v1(value)

    def test_rejects_extra_fields_that_might_smuggle_ui_measurements(self):
        value = state("overset")
        value["ui_measured_lines"] = 4
        with self.assertRaisesRegex(EditorOversetUiError, "fields mismatch"):
            project_editor_overset_ui_v1(value)


if __name__ == "__main__":
    unittest.main()
