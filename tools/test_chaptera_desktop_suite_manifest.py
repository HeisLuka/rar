import copy
import json
import pathlib
import unittest

from tools.validate_chaptera_desktop_suite_manifest import validate_manifest

ROOT = pathlib.Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "packages" / "product" / "desktop-suite" / "v1" / "product-manifest.json"


class ChapteraDesktopSuiteManifestTests(unittest.TestCase):
    def load(self):
        return json.loads(MANIFEST.read_text(encoding="utf-8"))

    def test_canonical_manifest_passes(self):
        summary = validate_manifest(self.load())
        self.assertEqual(summary["product_ids"], ["chaptera.editor", "chaptera.migration", "chaptera.reader", "chaptera.rescue"])
        self.assertEqual(summary["legacy_chaptera_exe_owner"], "chaptera.editor")

    def test_generic_binary_cannot_be_canonical(self):
        value = self.load()
        value["products"][0]["canonical_windows_executable"] = "chaptera.exe"
        with self.assertRaisesRegex(AssertionError, "product-qualified|generic chaptera.exe"):
            validate_manifest(value)

    def test_executables_must_be_unique(self):
        value = self.load()
        value["products"][1]["canonical_windows_executable"] = value["products"][0]["canonical_windows_executable"]
        with self.assertRaisesRegex(AssertionError, "duplicate canonical executable"):
            validate_manifest(value)

    def test_all_four_product_ids_are_required(self):
        value = self.load()
        value["products"].pop()
        with self.assertRaisesRegex(AssertionError, "exactly Reader, Rescue, Editor and Migration"):
            validate_manifest(value)

    def test_legacy_generic_alias_must_belong_to_editor(self):
        value = copy.deepcopy(self.load())
        value["legacy_binary_aliases"][0]["product_id"] = "chaptera.reader"
        with self.assertRaisesRegex(AssertionError, "belong only to Editor"):
            validate_manifest(value)


if __name__ == "__main__":
    unittest.main()
