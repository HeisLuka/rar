#!/usr/bin/env python3
from __future__ import annotations

import json
import unittest
from pathlib import Path

from t720_register_29_proof import canonical_digest

HERE = Path(__file__).resolve().parent
INPUT = HERE / "receipts" / "version-labelled-training-29-input-2026-09-25.json"


class T720RegistrationTests(unittest.TestCase):
    def test_input_is_exact_and_source_safe(self):
        spec = json.loads(INPUT.read_text(encoding="utf-8"))
        self.assertEqual("chaptera.corpus-version-labelled-training-register-input.v1", spec["schema"])
        self.assertEqual(4, len(spec["packages"]))

        observations = []
        package_sets = []
        for package in spec["packages"]:
            rows = package["files"]
            self.assertEqual(package["expected_observation_count"], len(rows))
            shas = {row["sha256"] for row in rows}
            self.assertEqual(package["expected_unique_sha_count"], len(shas))
            self.assertTrue(str(package["source_page"]).startswith("https://www.microsoftpressstore.com/"))
            self.assertTrue(str(package["source_download"]).startswith("https://www.microsoftpressstore.com/"))
            for row in rows:
                self.assertRegex(row["sha256"], r"^[0-9a-f]{64}$")
                self.assertGreater(row["size"], 0)
                self.assertEqual(44, row["contents_revision_u16_at_2"])
            observations.extend(rows)
            package_sets.append(shas)

        self.assertEqual(33, len(observations))
        combined = {row["sha256"] for row in observations}
        self.assertEqual(29, len(combined))
        for i, left in enumerate(package_sets):
            for right in package_sets[i + 1:]:
                self.assertFalse(left & right)

        # This digest freezes the exact 29-SHA input independent of package/member naming.
        self.assertEqual(
            "eb2cd6ad8f21860f93d33763a8f630135c12c128e0160e0d1909c0c3ae228ae8",
            canonical_digest(combined),
        )


if __name__ == "__main__":
    unittest.main()
