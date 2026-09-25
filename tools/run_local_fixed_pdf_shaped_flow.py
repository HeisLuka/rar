#!/usr/bin/env python3
"""Rar-owned local/native closure runner for FIXED-PDF-SHAPED-FLOW-01.

This launcher keeps the active workspace boundary in HeisLuka/rar while allowing
an authorized local/native fixed-PDF engine to remain private. It does not parse
PUB, shape text, or serialize PDF itself.

The engine command must:
- consume the verified pinned PUB fixture;
- write a PDF to {pdf};
- consume an explicit fallback font from {font};
- print exactly one conversion-report JSON object to stdout.

The report must carry typography.fixed_flow_receipt matching the admitted
chaptera.fixed-pdf-shaped-flow-receipt.v1 contract. This runner independently
validates the receipt, source identity, real SampleNewsletter witness, output
PDF, and immutable input bytes, then writes only the sanitized receipt.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
from typing import Any, Iterable

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from validate_fixed_pdf_shaped_flow_receipt import validate_schema, validate_semantics

SAMPLE_SOURCE_HASH = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
SAMPLE_SOURCE_BYTE_LEN = 291840
FORMER_MULTILINE_STORY_IDS = frozenset(
    {
        "a216335c-5e39-52a5-85f0-8a1abeb1819b",
        "902668d7-7275-5e4e-9598-280d9f479198",
        "52322820-64da-5ed6-ac6e-37a695e0b30f",
        "0439345d-6742-58f7-ad3e-4890fcf2c39f",
        "e4e0f03b-7cdc-5025-a1da-f97383b817d0",
    }
)
DEPRECATED_BRIDGE_CODES = frozenset(
    {
        "pdf.text.multi_frame_unsupported",
        "pdf.text.multiline_unsupported",
        "pdf.text.non_ascii_unsupported",
        "pdf.text.single_line_overflow",
    }
)


class LocalFixedPdfProducerError(RuntimeError):
    pass


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def bind_file(path: pathlib.Path, *, label: str) -> pathlib.Path:
    resolved = path.expanduser().resolve(strict=True)
    if not resolved.is_file():
        raise LocalFixedPdfProducerError(f"{label} path is not a regular file")
    return resolved


def bind_fixture(
    fixture: pathlib.Path,
    *,
    expected_hash: str,
    expected_len: int,
) -> pathlib.Path:
    path = bind_file(fixture, label="fixture")
    actual_len = path.stat().st_size
    if actual_len != expected_len:
        raise LocalFixedPdfProducerError(
            f"fixture byte length mismatch: expected={expected_len} actual={actual_len}"
        )
    actual_hash = sha256_file(path)
    if actual_hash != expected_hash:
        raise LocalFixedPdfProducerError(
            f"fixture SHA-256 mismatch: expected={expected_hash} actual={actual_hash}"
        )
    return path


def render_command(
    template: list[str],
    *,
    fixture: pathlib.Path,
    pdf_output: pathlib.Path,
    fallback_font: pathlib.Path,
) -> list[str]:
    if not template:
        raise LocalFixedPdfProducerError("fixed-PDF engine command is empty")

    replacements = {
        "{fixture}": str(fixture),
        "{pdf}": str(pdf_output),
        "{font}": str(fallback_font),
    }
    for placeholder in replacements:
        count = sum(part.count(placeholder) for part in template)
        if count != 1:
            raise LocalFixedPdfProducerError(
                f"fixed-PDF engine command must contain {placeholder} exactly once"
            )

    rendered = list(template)
    for placeholder, value in replacements.items():
        rendered = [part.replace(placeholder, value) for part in rendered]
    return rendered


def extract_fixed_flow_receipt(report: Any) -> dict[str, Any]:
    if not isinstance(report, dict):
        raise LocalFixedPdfProducerError("fixed-PDF engine stdout must be a JSON object")
    try:
        typography = report["typography"]
        receipt = typography["fixed_flow_receipt"]
    except (KeyError, TypeError) as error:
        raise LocalFixedPdfProducerError(
            "conversion report is missing typography.fixed_flow_receipt"
        ) from error
    if not isinstance(typography, dict) or not isinstance(receipt, dict):
        raise LocalFixedPdfProducerError(
            "typography.fixed_flow_receipt must be a JSON object"
        )
    return receipt


def require_real_witness(
    report: dict[str, Any],
    receipt: dict[str, Any],
    *,
    expected_hash: str,
    witness_story_ids: Iterable[str],
) -> tuple[str, int]:
    try:
        validate_schema(receipt)
        validate_semantics(receipt)
    except AssertionError as error:
        raise LocalFixedPdfProducerError(str(error)) from error

    if receipt.get("source_hash") != expected_hash:
        raise LocalFixedPdfProducerError(
            "fixed-flow receipt source_hash differs from verified fixture"
        )

    lines = receipt["lines"]
    runs = receipt["runs"]
    if not lines:
        raise LocalFixedPdfProducerError("real receipt contains zero visible fixed-output lines")
    if not any(run["scalar_base"] > 0 for run in runs):
        raise LocalFixedPdfProducerError(
            "real receipt does not exercise a non-zero Story-global scalar_base"
        )

    witness_set = set(witness_story_ids)
    witnessed = sorted(
        {
            line["story_id"]
            for line in lines
            if line["story_id"] in witness_set
        }
    )
    if not witnessed:
        raise LocalFixedPdfProducerError(
            "real receipt does not materialize any historically multiline-dropped SampleNewsletter Story"
        )

    typography = report.get("typography")
    if not isinstance(typography, dict):
        raise LocalFixedPdfProducerError("conversion report typography must be an object")
    shaped_flow = typography.get("shaped_flow")
    if not isinstance(shaped_flow, dict):
        raise LocalFixedPdfProducerError("conversion report is missing typography.shaped_flow")
    if shaped_flow.get("output_adapter_reshaping_calls") != 0:
        raise LocalFixedPdfProducerError(
            "native fixed-output adapter performed or failed to prove zero reshaping calls"
        )

    skipped = typography.get("skipped", [])
    if not isinstance(skipped, list):
        raise LocalFixedPdfProducerError("conversion report typography.skipped must be an array")
    deprecated_seen = sorted(
        {
            item.get("code")
            for item in skipped
            if isinstance(item, dict) and item.get("code") in DEPRECATED_BRIDGE_CODES
        }
    )
    if deprecated_seen:
        raise LocalFixedPdfProducerError(
            "deprecated one-line bridge gates are still active: " + ", ".join(deprecated_seen)
        )

    return witnessed[0], len(witnessed)


def run_local_fixed_pdf(
    *,
    fixture: pathlib.Path,
    fallback_font: pathlib.Path,
    pdf_output: pathlib.Path,
    receipt_output: pathlib.Path,
    command_template: list[str],
    expected_hash: str = SAMPLE_SOURCE_HASH,
    expected_len: int = SAMPLE_SOURCE_BYTE_LEN,
    witness_story_ids: Iterable[str] = FORMER_MULTILINE_STORY_IDS,
) -> dict[str, Any]:
    fixture = bind_fixture(
        fixture,
        expected_hash=expected_hash,
        expected_len=expected_len,
    )
    fallback_font = bind_file(fallback_font, label="fallback font")
    if fallback_font.stat().st_size == 0:
        raise LocalFixedPdfProducerError("fallback font is empty")

    pdf_output = pdf_output.expanduser().resolve()
    receipt_output = receipt_output.expanduser().resolve()
    pdf_output.parent.mkdir(parents=True, exist_ok=True)
    receipt_output.parent.mkdir(parents=True, exist_ok=True)
    if pdf_output.exists():
        pdf_output.unlink()
    if receipt_output.exists():
        receipt_output.unlink()

    command = render_command(
        command_template,
        fixture=fixture,
        pdf_output=pdf_output,
        fallback_font=fallback_font,
    )
    completed = subprocess.run(
        command,
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace").strip()
        raise LocalFixedPdfProducerError(
            "local fixed-PDF engine failed" + (f": {detail}" if detail else "")
        )

    try:
        report_text = completed.stdout.decode("utf-8")
    except UnicodeDecodeError as error:
        raise LocalFixedPdfProducerError(
            "fixed-PDF engine stdout is not UTF-8 JSON"
        ) from error
    try:
        report = json.loads(report_text)
    except json.JSONDecodeError as error:
        raise LocalFixedPdfProducerError(
            "fixed-PDF engine stdout is not exactly one JSON value"
        ) from error
    if not isinstance(report, dict):
        raise LocalFixedPdfProducerError("fixed-PDF engine stdout must be a JSON object")

    if fixture.stat().st_size != expected_len or sha256_file(fixture) != expected_hash:
        raise LocalFixedPdfProducerError("source fixture changed during conversion")

    if not pdf_output.is_file():
        raise LocalFixedPdfProducerError("fixed-PDF engine did not create the requested PDF")
    pdf_bytes = pdf_output.read_bytes()
    if not pdf_bytes.startswith(b"%PDF-"):
        raise LocalFixedPdfProducerError("fixed-PDF output is not a PDF file")
    if len(pdf_bytes) < 16:
        raise LocalFixedPdfProducerError("fixed-PDF output is implausibly small")

    receipt = extract_fixed_flow_receipt(report)
    witness_story_id, witness_story_count = require_real_witness(
        report,
        receipt,
        expected_hash=expected_hash,
        witness_story_ids=witness_story_ids,
    )

    receipt_output.write_text(
        json.dumps(receipt, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )

    return {
        "status": "ok",
        "source_hash": expected_hash,
        "source_byte_len": expected_len,
        "receipt_version": receipt["receipt_version"],
        "flow_id": receipt["flow_id"],
        "visible_line_count": len(receipt["lines"]),
        "fixed_run_count": len(receipt["runs"]),
        "story_overset": receipt["story_overset"],
        "witness_story_id": witness_story_id,
        "witness_story_count": witness_story_count,
        "pdf_sha256": hashlib.sha256(pdf_bytes).hexdigest(),
        "pdf_byte_len": len(pdf_bytes),
        "fallback_font_sha256": sha256_file(fallback_font),
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Run the authorized local/native SampleNewsletter fixed-PDF engine "
            "from the Rar checkout and retain only the sanitized shaped-flow receipt"
        )
    )
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--fallback-font", required=True, type=pathlib.Path)
    parser.add_argument("--pdf-output", required=True, type=pathlib.Path)
    parser.add_argument("--receipt-output", required=True, type=pathlib.Path)
    parser.add_argument("fixed_pdf_command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = list(args.fixed_pdf_command)
    if command and command[0] == "--":
        command = command[1:]

    try:
        summary = run_local_fixed_pdf(
            fixture=args.fixture,
            fallback_font=args.fallback_font,
            pdf_output=args.pdf_output,
            receipt_output=args.receipt_output,
            command_template=command,
        )
    except (LocalFixedPdfProducerError, OSError) as error:
        print(str(error), file=sys.stderr)
        return 2

    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
