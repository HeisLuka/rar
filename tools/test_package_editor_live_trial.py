import pathlib
import tempfile
import unittest
import zipfile

from tools.package_editor_live_trial import package_editor


class EditorLiveTrialPackagerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        self.exe = self.root / "editor.exe"
        self.exe.write_bytes(b"MZ" + b"chaptera-editor" * 32)
        self.readme = self.root / "README.md"
        self.readme.write_text(
            "# Trial\nContract: chaptera.editor-live-trial-readme.v1\n",
            encoding="utf-8",
        )

    def tearDown(self):
        self.tmp.cleanup()

    def test_packages_only_binary_and_readme(self):
        output = self.root / "Chaptera-Editor.zip"
        result = package_editor(self.exe, output, readme=self.readme)

        self.assertTrue(output.is_file())
        self.assertEqual(len(result["binary_sha256"]), 64)
        self.assertEqual(len(result["zip_sha256"]), 64)
        self.assertNotEqual(result["binary_sha256"], result["zip_sha256"])

        with zipfile.ZipFile(output) as archive:
            self.assertEqual(
                sorted(archive.namelist()),
                ["Chaptera-Editor.exe", "TRIAL-README.md"],
            )
            self.assertEqual(archive.read("Chaptera-Editor.exe"), self.exe.read_bytes())

    def test_package_is_deterministic(self):
        first = self.root / "first.zip"
        second = self.root / "second.zip"
        one = package_editor(self.exe, first, readme=self.readme)
        two = package_editor(self.exe, second, readme=self.readme)
        self.assertEqual(one["zip_sha256"], two["zip_sha256"])

    def test_rejects_non_pe_binary(self):
        bad = self.root / "bad.exe"
        bad.write_bytes(b"not-a-pe")
        with self.assertRaisesRegex(RuntimeError, "missing MZ header"):
            package_editor(bad, self.root / "bad.zip", readme=self.readme)

    def test_rejects_wrong_readme_contract(self):
        self.readme.write_text("# no contract\n", encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "README contract marker"):
            package_editor(self.exe, self.root / "bad.zip", readme=self.readme)

    def test_rejects_unsafe_entry_name(self):
        with self.assertRaisesRegex(RuntimeError, "single safe ZIP entry name"):
            package_editor(
                self.exe,
                self.root / "bad.zip",
                readme=self.readme,
                binary_entry="../Chaptera.exe",
            )


if __name__ == "__main__":
    unittest.main()
