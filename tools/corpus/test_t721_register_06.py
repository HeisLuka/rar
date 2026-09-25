#!/usr/bin/env python3
from __future__ import annotations
import hashlib, json, unittest
from pathlib import Path

HERE=Path(__file__).resolve().parent
INPUT=HERE/"receipts"/"publisher2002-sbs-06-input-2026-09-25.json"

def digest(xs):
    return hashlib.sha256(("\n".join(sorted(xs))+"\n").encode("ascii")).hexdigest()

class T721RegistrationTests(unittest.TestCase):
    def test_exact_source_safe_input(self):
        spec=json.loads(INPUT.read_text(encoding="utf-8"))
        self.assertEqual("chaptera.corpus-publisher2002-sbs-register-input.v1",spec["schema"])
        rows=spec["files"]
        self.assertEqual(6,len(rows))
        shas={r["sha256"] for r in rows}
        self.assertEqual(6,len(shas))
        self.assertEqual("74eccc943b9187a6a351b27ecdb3b666880953f8cbfeb9aa45cec8794afc0f15",digest(shas))
        self.assertEqual(1515,spec["expected_predecessor_count"])
        self.assertEqual(1521,spec["expected_successor_count"])
        self.assertEqual(646,spec["provenance"]["upstream_pr"])
        self.assertEqual(36159842901,spec["provenance"]["upstream_run_id"])

if __name__=="__main__":
    unittest.main()
