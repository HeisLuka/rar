#!/usr/bin/env python3
import base64
import copy
import hashlib
import json
import pathlib
import sys
import tempfile
import textwrap
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

import validate_editor_fixed_page_trial_receipt as fixed_page_validator
from run_local_editor_fixed_page_trial import (
    FixedPageTrialError,
    run_local_fixed_page_trial,
)

SOURCE_BYTES = b"fixed-page-trial-fixture"
SOURCE_HASH = hashlib.sha256(SOURCE_BYTES).hexdigest()
RAR_COMMIT = "d" * 40
NODE = "20000000-0000-4000-8000-000000000001"
STORY = "10000000-0000-4000-8000-000000000001"

FAKE_ENGINE = r"""#!/usr/bin/env python3
import base64
import hashlib
import json
import os
import pathlib
import sys
import zipfile

fixture = pathlib.Path(sys.argv[1])
project_path = pathlib.Path(sys.argv[2])
export_path = pathlib.Path(sys.argv[3])
replacement_path = pathlib.Path(sys.argv[4])

source_hash = os.environ["CHAPTERA_SOURCE_HASH"]
rar_commit = os.environ["CHAPTERA_RAR_COMMIT"]
auth_wrap_sha = os.environ["CHAPTERA_AUTH_WRAP_RECEIPT_SHA256"]
story_witness = os.environ["CHAPTERA_TRIAL_STORY_WITNESS"]
replacement = replacement_path.read_bytes()
replacement_sha = hashlib.sha256(replacement).hexdigest()

story = "10000000-0000-4000-8000-000000000001"
node = "20000000-0000-4000-8000-000000000001"
move_before = {"x": 12700, "y": 25400, "width": 38100, "height": 50800}
move_after = {"x": 127000, "y": 254000, "width": 38100, "height": 50800}
resize_after = {"x": 127000, "y": 254000, "width": 50800, "height": 63500}

project = {
    "schema_version": "pub-editor-v0.5",
    "source_hash": source_hash,
    "assets": [
        {"sha256": replacement_sha, "mime": "image/png", "byte_len": len(replacement)}
    ],
    "operations": [
        {
            "kind": "replace_story_range",
            "story_id": story,
            "start_scalar": 0,
            "end_scalar": 1,
            "expected_before": "X",
            "replacement_text": story_witness,
            "before_story_state_id": "sha256:" + "1" * 64,
            "after_story_state_id": "sha256:" + "2" * 64
        },
        {
            "kind": "move_node",
            "node_id": node,
            "before": move_before,
            "after": move_after
        },
        {
            "kind": "resize_node",
            "node_id": node,
            "before": move_after,
            "after": resize_after
        },
        {
            "kind": "replace_image",
            "node_id": node,
            "before_asset": None,
            "after_asset": replacement_sha
        }
    ]
}
project_raw = json.dumps(project, sort_keys=True, separators=(",", ":")).encode("utf-8")
project_path.write_bytes(project_raw)
project_sha = hashlib.sha256(project_raw).hexdigest()

encoded = base64.b64encode(replacement).decode("ascii")
with zipfile.ZipFile(export_path, "w") as archive:
    archive.writestr("designmap.xml", "<Document/>")
    archive.writestr(
        "Stories/Story_u1.xml",
        f"<Story><Content>{story_witness}</Content></Story>"
    )
    archive.writestr(
        "Spreads/Spread_u1.xml",
        '<Spread><TextFrame Self="uf20000000000040008000000000000001">'
        '<PathPointType Anchor="10 20"/>'
        '<PathPointType Anchor="10 25"/>'
        '<PathPointType Anchor="14 25"/>'
        '<PathPointType Anchor="14 20"/>'
        f'<Contents><![CDATA[{encoded}]]></Contents>'
        '</TextFrame></Spread>'
    )

observation = {
    "protocol_version": "chaptera.editor-fixed-page-trial-observation.v1",
    "source_hash": source_hash,
    "rar_commit": rar_commit,
    "auth_wrap_receipt_sha256": auth_wrap_sha,
    "saved_project_sha256": project_sha,
    "reopened_project_sha256": project_sha,
    "user_path": {
        "launch_without_dev_toolchain": True,
        "pub_opened": True,
        "supported_story_edited": True,
        "supported_object_moved": True,
        "supported_object_resized_from_canvas": True,
        "supported_image_replaced": True,
        "bounded_wrap_preserved": True,
        "undo_redo_verified": True,
        "editor_project_saved": True,
        "close_reopen_reproduced_state": True,
        "capability_loss_state_visible": True
    },
    "export_result": {
        "target": "idml",
        "edited_story_present": True,
        "moved_geometry_present": True,
        "resized_geometry_present": True,
        "replacement_image_exact": True,
        "bounded_wrap_result_preserved": True,
        "blocking_loss_count": 0,
        "approximations_explicit": True
    },
    "safety": {
        "source_pub_immutable": True,
        "native_save_pub_claimed": False,
        "unsupported_mutation_fails_closed": True,
        "no_silent_source_image_fallback": True,
        "no_hidden_network_upload": True
    }
}
sys.stdout.write(json.dumps(observation, separators=(",", ":")))
"""


def make_native_auth_wrap(value):
    value = copy.deepcopy(value)
    value["receipt_kind"] = "native_observation"
    value["scope"]["closure_candidate"] = True
    value["environment"]["reset_provider_receipt_verified"] = True
    value["environment"]["cold_restore_pair_verified"] = True
    value["environment"]["environment_fingerprint_sha256"] = "c" * 64
    for family_name in ("family_a", "family_b"):
        for key in value[family_name]["capture"]:
            value[family_name]["capture"][key] = True
    value["family_a"]["post_save_0x47_outcome"] = "patched_ref_preserved"
    value["family_a"]["post_save_fopt_outcome"] = "unchanged"
    value["family_a"]["layout_outcome"] = "followed_patched_ref"
    value["family_b"]["post_save_0x47_outcome"] = "regenerated_for_new_wrap_state"
    value["family_b"]["post_save_fopt_outcome"] = "requested_change_persisted"
    value["family_b"]["layout_outcome"] = "followed_new_wrap_state"
    value["conclusion"]["authority_class"] = "source"
    value["conclusion"]["both_conflict_families_executed"] = True
    value["conclusion"]["save_reopen_evidence_complete"] = True
    value["conclusion"]["needs_additional_native_discriminator"] = False
    return value


class FixedPageTrialRunnerTests(unittest.TestCase):
    def setUp(self):
        self.original_public_sha = fixed_page_validator.PUBLIC_SURROGATE_SHA
        fixed_page_validator.PUBLIC_SURROGATE_SHA = SOURCE_HASH

    def tearDown(self):
        fixed_page_validator.PUBLIC_SURROGATE_SHA = self.original_public_sha

    def copy_receipt(self, root, relative, name, mutate=None):
        value = json.loads((ROOT / relative).read_text(encoding="utf-8"))
        if mutate:
            value = mutate(value)
        path = pathlib.Path(root) / name
        path.write_text(json.dumps(value, sort_keys=True), encoding="utf-8")
        return path

    def make_files(self, root, engine=FAKE_ENGINE):
        root = pathlib.Path(root)
        fixture = root / "SampleNewsletter.pub"
        fixture.write_bytes(SOURCE_BYTES)
        engine_path = root / "engine.py"
        engine_path.write_text(textwrap.dedent(engine), encoding="utf-8")
        project = root / "trial.chaptera.json"
        export = root / "trial.idml"
        receipt = root / "fixed-page.real.json"

        package = self.copy_receipt(
            root,
            "packages/product/editor-live-trial/v1/fixtures/package-receipt.synthetic.json",
            "package.json",
        )
        resize = self.copy_receipt(
            root,
            "packages/protocol/editor-resize/v1/fixtures/producer-receipt.synthetic.json",
            "resize.json",
        )
        replace_ui = self.copy_receipt(
            root,
            "packages/protocol/editor-image-replace/v1/fixtures/ui-producer-receipt.synthetic.json",
            "replace-ui.json",
        )
        replace_export = self.copy_receipt(
            root,
            "packages/protocol/editor-image-replace/v1/fixtures/export-producer-receipt.synthetic.json",
            "replace-export.json",
        )
        auth = self.copy_receipt(
            root,
            "packages/research/auth-wrap/v1/fixtures/native-receipt.synthetic.json",
            "auth-wrap.json",
            make_native_auth_wrap,
        )
        command = [
            sys.executable,
            str(engine_path),
            "{fixture}",
            "{project}",
            "{export}",
            "{replacement}",
        ]
        return {
            "fixture": fixture,
            "project": project,
            "export": export,
            "receipt": receipt,
            "package": package,
            "resize": resize,
            "replace_ui": replace_ui,
            "replace_export": replace_export,
            "auth": auth,
            "command": command,
        }

    def run_trial(self, files):
        return run_local_fixed_page_trial(
            fixture=files["fixture"],
            package_receipt=files["package"],
            resize_receipt=files["resize"],
            replace_ui_receipt=files["replace_ui"],
            replace_export_receipt=files["replace_export"],
            auth_wrap_receipt=files["auth"],
            project_output=files["project"],
            export_output=files["export"],
            receipt_output=files["receipt"],
            command_template=files["command"],
            expected_hash=SOURCE_HASH,
            expected_len=len(SOURCE_BYTES),
            rar_commit=RAR_COMMIT,
            host_system="Windows",
            allow_synthetic_upstream_for_test=True,
        )

    def test_continuous_trial_binds_evidence_project_and_export(self):
        with tempfile.TemporaryDirectory() as tmp:
            files = self.make_files(tmp)
            receipt = self.run_trial(files)
            saved = json.loads(files["receipt"].read_text(encoding="utf-8"))

        self.assertEqual(receipt, saved)
        self.assertEqual(saved["receipt_kind"], "real_trial")
        self.assertTrue(saved["user_path"]["supported_object_resized_from_canvas"])
        self.assertTrue(saved["export_result"]["replacement_image_exact"])
        self.assertEqual(
            saved["evidence_chain"]["replace_image_ui"]["replacement_binding_id"],
            saved["evidence_chain"]["replace_image_export"]["replacement_binding_id"],
        )
        self.assertTrue(saved["evidence_chain"]["auth_wrap"]["native_observation"])
        self.assertEqual(saved["evidence_chain"]["auth_wrap"]["authority_class"], "source")

    def test_missing_resize_operation_fails_closed(self):
        broken = FAKE_ENGINE.replace(
            '        {\n            "kind": "resize_node",\n            "node_id": node,\n            "before": move_after,\n            "after": resize_after\n        },\n',
            "",
        )
        with tempfile.TemporaryDirectory() as tmp:
            files = self.make_files(tmp, broken)
            with self.assertRaisesRegex(FixedPageTrialError, "exactly one Story/Move/Resize"):
                self.run_trial(files)

    def test_source_mutation_fails_closed(self):
        broken = FAKE_ENGINE.replace(
            'replacement = replacement_path.read_bytes()',
            'replacement = replacement_path.read_bytes()\nfixture.write_bytes(fixture.read_bytes() + b"x")',
        )
        with tempfile.TemporaryDirectory() as tmp:
            files = self.make_files(tmp, broken)
            with self.assertRaisesRegex(FixedPageTrialError, "source PUB changed"):
                self.run_trial(files)

    def test_synthetic_upstream_is_rejected_outside_test_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            files = self.make_files(tmp)
            with self.assertRaisesRegex(FixedPageTrialError, "package receipt is not real_pub_sanitized"):
                run_local_fixed_page_trial(
                    fixture=files["fixture"],
                    package_receipt=files["package"],
                    resize_receipt=files["resize"],
                    replace_ui_receipt=files["replace_ui"],
                    replace_export_receipt=files["replace_export"],
                    auth_wrap_receipt=files["auth"],
                    project_output=files["project"],
                    export_output=files["export"],
                    receipt_output=files["receipt"],
                    command_template=files["command"],
                    expected_hash=SOURCE_HASH,
                    expected_len=len(SOURCE_BYTES),
                    rar_commit=RAR_COMMIT,
                    host_system="Windows",
                )


if __name__ == "__main__":
    unittest.main()
