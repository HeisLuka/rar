import json
import pathlib
import sys
import tempfile
import unittest
import zipfile

from tools.build_editor_live_trial_package_receipt import build_receipt


PRODUCER = r"""
import json
import sys

mode = sys.argv[1]
request = json.loads(sys.stdin.read())
sha = "a" * 64
result = {
    "fixture_kind": request["fixture_kind"],
    "source_sha256_before": sha,
    "source_sha256_after": ("b" * 64 if mode == "mutate" else sha),
    "reader_only": mode == "reader",
    "editor_controls_enabled": mode != "reader",
    "native_save_pub_claimed": False,
    "launch_without_dev_toolchain": True,
    "real_pub_opened": True,
    "supported_story_edited": True,
    "supported_object_dragged": True,
    "undo_redo_verified": True,
    "editor_project_saved": True,
    "close_reopen_reproduced_state": True,
    "unsupported_actions_fail_closed": mode != "unsafe",
}
print(json.dumps(result))
"""


class EditorLiveTrialPackageBuilderTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        self.producer = self.root / "producer.py"
        self.producer.write_text(PRODUCER, encoding="utf-8")

    def tearDown(self):
        self.tmp.cleanup()

    def make_zip(self, *, readme_contract=True, source_pub=False):
        path = self.root / "Chaptera-Editor.zip"
        with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("Chaptera-Editor.exe", b"MZ\x00chaptera-editor")
            marker = (
                "chaptera.editor-live-trial-readme.v1"
                if readme_contract
                else "wrong.readme.contract"
            )
            archive.writestr("TRIAL-README.md", "# Trial\nContract: `" + marker + "`\n")
            if source_pub:
                archive.writestr("sample.pub", b"private-source")
        return path

    def command(self, mode="ok"):
        return [sys.executable, str(self.producer), mode]

    def test_real_pub_sanitized_receipt_passes_and_redacts_local_identity(self):
        zip_path = self.make_zip()
        receipt = build_receipt(
            self.command(),
            zip_path=zip_path,
            binary_entry="Chaptera-Editor.exe",
            readme_entry="TRIAL-README.md",
            chaptera_version="0.1.0-local",
            fixture_kind="real_pub_sanitized",
        )
        self.assertEqual(
            receipt["receipt_version"],
            "chaptera.editor-live-trial-package-receipt.v1",
        )
        self.assertEqual(receipt["fixture_kind"], "real_pub_sanitized")
        self.assertTrue(receipt["runtime_smoke"]["supported_story_edited"])
        self.assertTrue(receipt["editor_boundary"]["source_pub_immutable"])
        serialized = json.dumps(receipt)
        self.assertNotIn(str(self.root), serialized)
        self.assertNotIn("source_sha256", serialized)

    def test_source_mutation_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "changed immutable source PUB"):
            build_receipt(
                self.command("mutate"),
                zip_path=self.make_zip(),
                binary_entry="Chaptera-Editor.exe",
                readme_entry="TRIAL-README.md",
                chaptera_version="0.1.0-local",
                fixture_kind="real_pub_sanitized",
            )

    def test_reader_only_build_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "reader-only"):
            build_receipt(
                self.command("reader"),
                zip_path=self.make_zip(),
                binary_entry="Chaptera-Editor.exe",
                readme_entry="TRIAL-README.md",
                chaptera_version="0.1.0-local",
                fixture_kind="real_pub_sanitized",
            )

    def test_incomplete_runtime_smoke_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "runtime smoke is incomplete"):
            build_receipt(
                self.command("unsafe"),
                zip_path=self.make_zip(),
                binary_entry="Chaptera-Editor.exe",
                readme_entry="TRIAL-README.md",
                chaptera_version="0.1.0-local",
                fixture_kind="real_pub_sanitized",
            )

    def test_missing_readme_contract_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "README contract marker"):
            build_receipt(
                self.command(),
                zip_path=self.make_zip(readme_contract=False),
                binary_entry="Chaptera-Editor.exe",
                readme_entry="TRIAL-README.md",
                chaptera_version="0.1.0-local",
                fixture_kind="real_pub_sanitized",
            )

    def test_source_pub_must_not_be_bundled(self):
        with self.assertRaisesRegex(RuntimeError, "must not bundle a source PUB"):
            build_receipt(
                self.command(),
                zip_path=self.make_zip(source_pub=True),
                binary_entry="Chaptera-Editor.exe",
                readme_entry="TRIAL-README.md",
                chaptera_version="0.1.0-local",
                fixture_kind="real_pub_sanitized",
            )


if __name__ == "__main__":
    unittest.main()
