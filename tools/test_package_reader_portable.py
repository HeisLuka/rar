import pathlib
import tempfile
import unittest
import zipfile

from tools.package_reader_portable import package_reader


class ReaderPortablePackageTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        self.exe = self.root / "reader.exe"
        self.exe.write_bytes(b"MZ" + b"chaptera-reader" * 32)
        self.readme = self.root / "README.md"
        self.readme.write_text(
            "# Reader\nContract: chaptera.reader-portable-readme.v1\n",
            encoding="utf-8",
        )

    def tearDown(self):
        self.tmp.cleanup()

    def test_package_is_product_qualified_and_deterministic(self):
        first = self.root / "first.zip"
        second = self.root / "second.zip"
        one = package_reader(self.exe, first, readme=self.readme)
        two = package_reader(self.exe, second, readme=self.readme)
        self.assertEqual(one["product_id"], "chaptera.reader")
        self.assertEqual(one["binary_entry"], "Chaptera-Reader.exe")
        self.assertEqual(one["zip_sha256"], two["zip_sha256"])
        with zipfile.ZipFile(first) as archive:
            self.assertEqual(
                sorted(archive.namelist()),
                ["Chaptera-Reader.exe", "README.md"],
            )

    def test_generic_entry_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "Chaptera-Reader.exe"):
            package_reader(
                self.exe,
                self.root / "bad.zip",
                readme=self.readme,
                binary_entry="Chaptera.exe",
            )

    def test_non_pe_is_rejected(self):
        self.exe.write_bytes(b"not-pe")
        with self.assertRaisesRegex(RuntimeError, "Windows PE"):
            package_reader(self.exe, self.root / "bad.zip", readme=self.readme)


if __name__ == "__main__":
    unittest.main()
