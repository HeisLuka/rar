#!/usr/bin/env python3
import hashlib
import json
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from run_local_fixed_pdf_shaped_flow import (
    LocalFixedPdfProducerError,
    run_local_fixed_pdf,
)

SOURCE_BYTES = b"x" * 32
SOURCE_HASH = hashlib.sha256(SOURCE_BYTES).hexdigest()
WITNESS = "a216335c-5e39-52a5-85f0-8a1abeb1819b"

FAKE_ENGINE = r"""#!/usr/bin/env python3
import hashlib
import json
import pathlib
import sys

fixture = pathlib.Path(sys.argv[1])
pdf = pathlib.Path(sys.argv[2])
font = pathlib.Path(sys.argv[3])
assert fixture.is_file()
assert font.is_file()
pdf.write_bytes(b"%PDF-1.7\n%fake\n")
source_hash = hashlib.sha256(fixture.read_bytes()).hexdigest()
receipt = {
  "receipt_version": "chaptera.fixed-pdf-shaped-flow-receipt.v1",
  "producer": {
    "implementation": "rar-native-test",
    "commit_or_build": "test-build",
    "core_integration": True
  },
  "source_hash": source_hash,
  "flow_id": "sha256:" + "1" * 64,
  "lines": [
    {
      "line_index": 0,
      "frame_node_id": "10000000-0000-4000-8000-000000000001",
      "story_id": "a216335c-5e39-52a5-85f0-8a1abeb1819b",
      "scalar_start": 2,
      "scalar_end": 4,
      "glyph_count": 2,
      "glyph_sequence_hash": "sha256:" + "2" * 64,
      "units_per_em": 1000,
      "measured_width": 1000
    }
  ],
  "runs": [
    {
      "run_index": 0,
      "frame_node_id": "10000000-0000-4000-8000-000000000001",
      "story_id": "a216335c-5e39-52a5-85f0-8a1abeb1819b",
      "scalar_base": 2,
      "scalar_end": 4,
      "glyph_count": 2,
      "glyph_sequence_hash": "sha256:" + "2" * 64,
      "baseline_x": 0,
      "baseline_y": 1200
    }
  ],
  "story_overset": False,
  "invariants": {
    "reshaping_calls": 0,
    "raw_text_emitted": False,
    "ascii_gate_applied": False,
    "overset_tail_painted": False,
    "line_order_preserved": True,
    "story_global_clusters_preserved": True
  }
}
report = {
  "typography": {
    "fixed_flow_receipt": receipt,
    "shaped_flow": {"output_adapter_reshaping_calls": 0},
    "skipped": []
  }
}
sys.stdout.write(json.dumps(report, separators=(",", ":")))
"""


class LocalFixedPdfRunnerTests(unittest.TestCase):
    def make_files(self, root, engine=FAKE_ENGINE):
        root = pathlib.Path(root)
        fixture = root / "SampleNewsletter.pub"
        fixture.write_bytes(SOURCE_BYTES)
        font = root / "fallback.ttf"
        font.write_bytes(b"font")
        engine_path = root / "engine.py"
        engine_path.write_text(engine, encoding="utf-8")
        pdf = root / "out.pdf"
        receipt = root / "receipt.json"
        command = [
            sys.executable,
            str(engine_path),
            "{fixture}",
            "{pdf}",
            "{font}",
        ]
        return fixture, font, pdf, receipt, command

    def test_real_witness_and_sanitized_receipt_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, font, pdf, receipt, command = self.make_files(tmp)
            summary = run_local_fixed_pdf(
                fixture=fixture,
                fallback_font=font,
                pdf_output=pdf,
                receipt_output=receipt,
                command_template=command,
                expected_hash=SOURCE_HASH,
                expected_len=len(SOURCE_BYTES),
                witness_story_ids={WITNESS},
            )
            saved = json.loads(receipt.read_text(encoding="utf-8"))
        self.assertEqual(summary["status"], "ok")
        self.assertEqual(summary["witness_story_id"], WITNESS)
        self.assertEqual(saved["runs"][0]["scalar_base"], 2)
        self.assertNotIn("logical_text", json.dumps(saved))

    def test_missing_later_line_fails(self):
        broken = FAKE_ENGINE.replace('"scalar_start": 2', '"scalar_start": 0').replace(
            '"scalar_base": 2', '"scalar_base": 0'
        )
        with tempfile.TemporaryDirectory() as tmp:
            fixture, font, pdf, receipt, command = self.make_files(tmp, broken)
            with self.assertRaisesRegex(LocalFixedPdfProducerError, "non-zero"):
                run_local_fixed_pdf(
                    fixture=fixture,
                    fallback_font=font,
                    pdf_output=pdf,
                    receipt_output=receipt,
                    command_template=command,
                    expected_hash=SOURCE_HASH,
                    expected_len=len(SOURCE_BYTES),
                    witness_story_ids={WITNESS},
                )

    def test_deprecated_bridge_gate_fails(self):
        broken = FAKE_ENGINE.replace(
            '"skipped": []',
            '"skipped": [{"code":"pdf.text.multiline_unsupported"}]',
        )
        with tempfile.TemporaryDirectory() as tmp:
            fixture, font, pdf, receipt, command = self.make_files(tmp, broken)
            with self.assertRaisesRegex(LocalFixedPdfProducerError, "deprecated one-line"):
                run_local_fixed_pdf(
                    fixture=fixture,
                    fallback_font=font,
                    pdf_output=pdf,
                    receipt_output=receipt,
                    command_template=command,
                    expected_hash=SOURCE_HASH,
                    expected_len=len(SOURCE_BYTES),
                    witness_story_ids={WITNESS},
                )

    def test_fixture_identity_is_checked_before_engine(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, font, pdf, receipt, command = self.make_files(tmp)
            with self.assertRaisesRegex(LocalFixedPdfProducerError, "fixture SHA-256 mismatch"):
                run_local_fixed_pdf(
                    fixture=fixture,
                    fallback_font=font,
                    pdf_output=pdf,
                    receipt_output=receipt,
                    command_template=command,
                    expected_hash="f" * 64,
                    expected_len=len(SOURCE_BYTES),
                    witness_story_ids={WITNESS},
                )


if __name__ == "__main__":
    unittest.main()
