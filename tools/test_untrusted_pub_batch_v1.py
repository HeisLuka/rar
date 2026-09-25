import os
import pathlib
import tempfile
import unittest

from untrusted_pub_batch_v1 import BatchPolicyV1, discover_regular_pub_files


class UntrustedPubBatchDiscoveryTests(unittest.TestCase):
    def test_discovery_is_sorted_case_insensitive_and_regular_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "z.PUB").write_bytes(b"z")
            (root / "nested").mkdir()
            (root / "nested" / "a.pub").write_bytes(b"a")
            (root / "skip.pdf").write_bytes(b"x")

            if hasattr(os, "symlink"):
                os.symlink(root / "z.PUB", root / "escape.pub")
            if hasattr(os, "mkfifo"):
                os.mkfifo(root / "trap.pub")

            found = [
                path.relative_to(root).as_posix()
                for path in discover_regular_pub_files(root)
            ]
            self.assertEqual(["nested/a.pub", "z.PUB"], found)

    def test_policy_rejects_zero_limits(self):
        for field in (
            "max_file_bytes",
            "max_cfb_entries",
            "max_declared_stream_bytes",
            "wall_timeout_ms",
            "address_space_bytes",
            "cpu_seconds",
            "open_files",
        ):
            values = BatchPolicyV1().__dict__.copy()
            values[field] = 0
            with self.assertRaises(ValueError, msg=field):
                BatchPolicyV1(**values).validate()


if __name__ == "__main__":
    unittest.main()
