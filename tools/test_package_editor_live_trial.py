import json
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
        self.catalog = self.root / "agent-control-v1.catalog.json"
        self.write_catalog()

    def write_catalog(
        self,
        *,
        schema="chaptera.agent-control.catalog.v1",
        protocol_version="chaptera.agent-control.v1",
        executable="chaptera-editor.exe",
        native_pub_write=False,
        source_pub_immutable=True,
    ):
        self.catalog.write_text(
            json.dumps(
                {
                    "schema": schema,
                    "protocol_version": protocol_version,
                    "executable": executable,
                    "global_laws": {
                        "native_pub_write": native_pub_write,
                        "source_pub_immutable": source_pub_immutable,
                    },
                    "commands": {},
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )

    def tearDown(self):
        self.tmp.cleanup()

    def package(self, output, **kwargs):
        return package_editor(
            self.exe,
            output,
            readme=self.readme,
            agent_catalog=self.catalog,
            **kwargs,
        )

    def test_packages_binary_readme_and_agent_catalog(self):
        output = self.root / "Chaptera-Editor.zip"
        result = self.package(output)

        self.assertTrue(output.is_file())
        self.assertEqual(len(result["binary_sha256"]), 64)
        self.assertEqual(len(result["agent_catalog_sha256"]), 64)
        self.assertEqual(len(result["zip_sha256"]), 64)
        self.assertNotEqual(result["binary_sha256"], result["zip_sha256"])
        self.assertEqual(result["agent_catalog_entry"], "agent-control-v1.catalog.json")
        self.assertEqual(result["agent_catalog_size"], self.catalog.stat().st_size)

        with zipfile.ZipFile(output) as archive:
            self.assertEqual(
                sorted(archive.namelist()),
                [
                    "Chaptera-Editor.exe",
                    "TRIAL-README.md",
                    "agent-control-v1.catalog.json",
                ],
            )
            self.assertEqual(archive.read("Chaptera-Editor.exe"), self.exe.read_bytes())
            self.assertEqual(
                archive.read("agent-control-v1.catalog.json"),
                self.catalog.read_bytes(),
            )

    def test_package_is_deterministic(self):
        first = self.root / "first.zip"
        second = self.root / "second.zip"
        one = self.package(first)
        two = self.package(second)
        self.assertEqual(one["zip_sha256"], two["zip_sha256"])
        self.assertEqual(one["agent_catalog_sha256"], two["agent_catalog_sha256"])

    def test_rejects_non_pe_binary(self):
        bad = self.root / "bad.exe"
        bad.write_bytes(b"not-a-pe")
        with self.assertRaisesRegex(RuntimeError, "missing MZ header"):
            package_editor(
                bad,
                self.root / "bad.zip",
                readme=self.readme,
                agent_catalog=self.catalog,
            )

    def test_rejects_wrong_readme_contract(self):
        self.readme.write_text("# no contract\n", encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "README contract marker"):
            self.package(self.root / "bad.zip")

    def test_rejects_unsafe_entry_name(self):
        with self.assertRaisesRegex(RuntimeError, "single safe ZIP entry name"):
            self.package(
                self.root / "bad.zip",
                binary_entry="../Chaptera.exe",
            )

    def test_rejects_catalog_with_wrong_protocol_identity(self):
        self.write_catalog(protocol_version="chaptera.agent-control.v2")
        with self.assertRaisesRegex(RuntimeError, "protocol version mismatch"):
            self.package(self.root / "bad.zip")

    def test_rejects_catalog_that_claims_native_pub_write(self):
        self.write_catalog(native_pub_write=True)
        with self.assertRaisesRegex(RuntimeError, "must not claim native PUB write"):
            self.package(self.root / "bad.zip")

    def test_rejects_catalog_without_immutable_source_law(self):
        self.write_catalog(source_pub_immutable=False)
        with self.assertRaisesRegex(RuntimeError, "immutable source PUB"):
            self.package(self.root / "bad.zip")

    def test_rejects_duplicate_package_entry_names(self):
        with self.assertRaisesRegex(RuntimeError, "entry names must be distinct"):
            self.package(
                self.root / "bad.zip",
                agent_catalog_entry="TRIAL-README.md",
            )


if __name__ == "__main__":
    unittest.main()
