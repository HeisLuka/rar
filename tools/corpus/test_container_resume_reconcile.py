#!/usr/bin/env python3
import unittest

from container_resume_reconcile import reconcile


def item(*sources):
    return {"sources": list(sources)}


class ReconcileTests(unittest.TestCase):
    def test_exact_partition_math(self):
        baseline = {
            f"{i:064x}": item("container_first_wave" if i < 425 else "other")
            for i in range(950)
        }
        crossarm = {f"{2000 + i:064x}" for i in range(129)}

        first_wave = list(baseline)[:425]
        other_baseline = list(baseline)[425:435]
        crossarm_overlap = sorted(crossarm)[:5]
        container_shas = set(first_wave + other_baseline + crossarm_overlap)
        next_i = 4000
        while len(container_shas) < 960:
            container_shas.add(f"{next_i:064x}")
            next_i += 1
        container = {
            sha: [
                {
                    "source_url": "https://example.invalid/a",
                    "archive_member": "x.pub",
                }
            ]
            for sha in container_shas
        }

        summary, rows = reconcile(baseline, container, crossarm)
        self.assertEqual(960, summary["container_full_unique_sha_count"])
        self.assertEqual(425, summary["container_first_wave_sha_count"])
        self.assertEqual(435, summary["container_overlap_baseline_sha_count"])
        self.assertEqual(5, summary["container_overlap_crossarm_sha_count"])
        self.assertEqual(520, summary["container_net_new_sha_count"])
        self.assertEqual(1599, summary["rar_union_after_container_sha_count"])
        self.assertEqual(520, len(rows))

    def test_first_wave_loss_fails_closed(self):
        baseline = {
            f"{i:064x}": item("container_first_wave" if i < 425 else "other")
            for i in range(950)
        }
        crossarm = {f"{2000 + i:064x}" for i in range(129)}
        container = {f"{i:064x}": [{}] for i in range(1, 961)}
        with self.assertRaisesRegex(ValueError, "lost"):
            reconcile(baseline, container, crossarm)


if __name__ == "__main__":
    unittest.main()
