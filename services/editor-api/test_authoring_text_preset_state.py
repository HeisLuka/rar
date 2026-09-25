#!/usr/bin/env python3
import copy
import pathlib
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

from revision_store import RevisionKernel, project_hash, state_id

DOCUMENT_ID = "doc:authoring-text-preset"
SOURCE_HASH = "ab" * 32
SOURCE_BLOB_SHA256 = "cd" * 32
FONT_FINGERPRINT = "12" * 32

def preset_record(font_size_emu=152_400):
    preset = {
        "preset_version": "chaptera.authoring-text-preset.v1",
        "font_fingerprint": FONT_FINGERPRINT,
        "face_index": 0,
        "font_size_emu": font_size_emu,
        "paragraph_defaults": {
            "alignment": "left",
            "space_before_emu": 0,
            "space_after_emu": 0,
        },
        "character_defaults": {"bold": False, "italic": False},
    }
    return {"preset_id": "sha256:" + "34" * 32, "preset": preset}

def project_with_preset(record):
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": SOURCE_HASH,
        "immutable_source_blob_sha256": SOURCE_BLOB_SHA256,
        "operations": [],
        "pages": {},
        "shapes": {},
        "text_frames": {},
        "picture_frames": {},
        "groups": {},
        "text_presets": {record["preset_id"]: copy.deepcopy(record)},
        "story_text_preset_bindings": {"author-story:empty": record["preset_id"]},
    }

class AuthoringTextPresetProjectStateTests(unittest.TestCase):
    def test_preset_is_revision_state_not_side_metadata(self):
        record = preset_record()
        project = project_with_preset(record)
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id=DOCUMENT_ID, source_hash=SOURCE_HASH, project=project
        )
        reopened = kernel.read_revision(
            document_id=DOCUMENT_ID, revision_id=baseline.revision_id
        )
        self.assertEqual(record, reopened.project["text_presets"][record["preset_id"]])
        self.assertEqual(
            record["preset_id"],
            reopened.project["story_text_preset_bindings"]["author-story:empty"],
        )

    def test_same_project_and_preset_have_stable_state_identity(self):
        project = project_with_preset(preset_record())
        first = state_id(DOCUMENT_ID, SOURCE_HASH, copy.deepcopy(project))
        second = state_id(DOCUMENT_ID, SOURCE_HASH, copy.deepcopy(project))
        self.assertEqual(first, second)
        self.assertEqual(project_hash(project), project_hash(copy.deepcopy(project)))

    def test_preset_change_changes_project_and_state_identity(self):
        original = project_with_preset(preset_record(152_400))
        changed = project_with_preset(preset_record(165_100))
        self.assertNotEqual(project_hash(original), project_hash(changed))
        self.assertNotEqual(
            state_id(DOCUMENT_ID, SOURCE_HASH, original),
            state_id(DOCUMENT_ID, SOURCE_HASH, changed),
        )

    def test_empty_story_binding_does_not_manufacture_story_text(self):
        project = project_with_preset(preset_record())
        self.assertIn("author-story:empty", project["story_text_preset_bindings"])
        self.assertNotIn("stories", project)
        self.assertNotIn("story_models", project)

if __name__ == "__main__":
    unittest.main()
