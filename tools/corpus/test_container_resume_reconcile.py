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
        while len(container_shas) < 869:
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
        self.assertEqual(869, summary["container_publisher_unique_sha_count"])
        self.assertEqual(425, summary["container_first_wave_sha_count"])
        self.assertEqual(435, summary["container_overlap_baseline_sha_count"])
        self.assertEqual(5, summary["container_overlap_crossarm_sha_count"])
        self.assertEqual(429, summary["container_net_new_sha_count"])
        self.assertEqual(1508, summary["rar_union_after_container_sha_count"])
        self.assertEqual(429, len(rows))

    def test_container_locator_provenance_is_preserved(self):
        baseline = {
            f"{i:064x}": item("container_first_wave" if i < 425 else "other")
            for i in range(950)
        }
        crossarm = {f"{2000 + i:064x}" for i in range(129)}
        first_wave = list(baseline)[:425]
        container_shas = set(first_wave)
        target = f"{4000:064x}"
        container_shas.add(target)
        next_i = 4001
        while len(container_shas) < 869:
            container_shas.add(f"{next_i:064x}")
            next_i += 1

        rows = {}
        for sha in container_shas:
            rows[sha] = [{
                "container_url": "https://archive.example/root.iso",
                "container_final_url": "https://cdn.example/root.iso",
                "container_filename": "root.iso",
                "root_sha256": "a" * 64,
                "root_size_bytes": 123456,
                "container_depth": 0,
                "archive_member": "leaf.pub",
                "size_bytes": 4096,
                "source_page": "https://archive.example/item",
            }]
        rows[target] = [{
            "container_url": "https://archive.example/root.iso",
            "container_final_url": "https://cdn.example/root.iso",
            "container_filename": "root.iso",
            "root_sha256": "a" * 64,
            "root_size_bytes": 123456,
            "container_depth": 1,
            "parent_member": "nested.cab",
            "parent_member_sha256": "b" * 64,
            "archive_member": "PAGEWIZ/AWARD.PUB",
            "size_bytes": 8192,
            "source_page": "https://archive.example/item",
        }]

        summary, delta = reconcile(baseline, rows, crossarm)
        self.assertEqual(444, summary["container_net_new_sha_count"])
        hit = next(row for row in delta if row["sha256"] == target)
        self.assertEqual(1, hit["observation_count"])
        prov = hit["provenance"][0]
        self.assertEqual("https://archive.example/root.iso", prov["container_url"])
        self.assertEqual(prov["container_url"], prov["source_url"])
        self.assertEqual("a" * 64, prov["root_sha256"])
        self.assertEqual(prov["root_sha256"], prov["parent_sha256"])
        self.assertEqual(1, prov["container_depth"])
        self.assertEqual("nested.cab", prov["parent_member"])
        self.assertEqual("b" * 64, prov["parent_member_sha256"])
        self.assertEqual("PAGEWIZ/AWARD.PUB", prov["archive_member"])
        self.assertEqual(8192, prov["size_bytes"])

    def test_first_wave_loss_fails_closed(self):
        baseline = {
            f"{i:064x}": item("container_first_wave" if i < 425 else "other")
            for i in range(950)
        }
        crossarm = {f"{2000 + i:064x}" for i in range(129)}
        container = {f"{i:064x}": [{}] for i in range(1, 870)}
        with self.assertRaisesRegex(ValueError, "lost"):
            reconcile(baseline, container, crossarm)


if __name__ == "__main__":
    unittest.main()
