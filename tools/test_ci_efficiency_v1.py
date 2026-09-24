#!/usr/bin/env python3
import unittest

from ci_efficiency_v1 import categorize_step, normalize_step_name, seconds_between

class CiEfficiencyTests(unittest.TestCase):
    def test_duration(self):
        self.assertEqual(65.0,seconds_between("2026-09-24T10:00:00Z","2026-09-24T10:01:05Z"))
        self.assertIsNone(seconds_between(None,"2026-09-24T10:01:05Z"))

    def test_categories(self):
        self.assertEqual("checkout",categorize_step("Run actions/checkout"))
        self.assertEqual("runtime_setup",categorize_step("Set up Python"))
        self.assertEqual("dependency_install",categorize_step("Install pinned MuPDF binding"))
        self.assertEqual("artifact_io",categorize_step("Upload artifact receipt"))
        self.assertEqual("test_or_benchmark",categorize_step("Run Render benchmark matrix"))
        self.assertEqual("other",categorize_step("Echo metadata"))

    def test_normalize(self):
        self.assertEqual("setup python <version>",normalize_step_name("Setup Python v3.12"))
        self.assertEqual("run <id>",normalize_step_name("Run 36004233040"))

if __name__=="__main__":
    unittest.main()
