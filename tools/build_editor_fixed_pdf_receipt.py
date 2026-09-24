#!/usr/bin/env python3
"""Build a sanitized current-Editor fixed-PDF receipt through an external renderer.

Rar owns current-state assembly and proof. The actual PDF serializer stays in the
existing local/native runtime. This builder passes only the source-neutral
current fixed-output packet to the renderer; raw PUB bytes are never supplied.

Renderer protocol:
- reads one JSON request from stdin;
- writes PDF bytes to CHAPTERA_PDF_OUTPUT;
- returns one JSON object on stdout describing the exact packet it rendered.

The builder independently validates packet identity, the PDF header/size/hash
and a bounded report summary before emitting a public-safe receipt.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tempfile
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from editor_fixed_output_current_state_v1 import (  # noqa: E402
    EditorFixedOutputError,
    _load_input,
    build_current_fixed_output,
)

RENDER_REQUEST_VERSION = "chaptera.editor-fixed-pdf-render-request.v1"
RENDER_RESULT_VERSION = "chaptera.editor-fixed-pdf-render-result.v1"
RECEIPT_VERSION = "chaptera.editor-fixed-pdf-receipt.v1"

MAX_PDF_BYTES = 512 * 1024 * 1024
MAX_DIAGNOSTIC_CODES = 256


class EditorFixedPdfReceiptError(RuntimeError):
    pass


def _canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def _sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _hash_id(value: Any) -> str:
    return "sha256:" + _sha256_bytes(_canonical_json(value))


def _require_exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EditorFixedPdfReceiptError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise EditorFixedPdfReceiptError(
            f"{label} fields mismatch: missing={sorted(expected-actual)} "
            f"extra={sorted(actual-expected)}"
        )
    return value


def _require_nonempty_string(value: Any, label: str, *, max_len: int = 160) -> str:
    if not isinstance(value, str) or not value or len(value) > max_len:
        raise EditorFixedPdfReceiptError(f"{label} must be a bounded non-empty string")
    return value


def _require_count(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise EditorFixedPdfReceiptError(f"{label} must be a non-negative integer")
    return value


def _validate_render_summary(value: Any) -> dict[str, Any]:
    summary = _require_exact_keys(
        value,
        {
            "page_count",
            "node_painted",
            "node_partial",
            "node_unsupported",
            "diagnostic_codes",
        },
        "renderer summary",
    )
    diagnostic_codes = summary["diagnostic_codes"]
    if not isinstance(diagnostic_codes, list) or len(diagnostic_codes) > MAX_DIAGNOSTIC_CODES:
        raise EditorFixedPdfReceiptError("renderer diagnostic_codes must be a bounded array")
    normalized_codes: list[str] = []
    for index, code in enumerate(diagnostic_codes):
        code = _require_nonempty_string(
            code,
            f"renderer diagnostic_codes[{index}]",
            max_len=128,
        )
        normalized_codes.append(code)
    if normalized_codes != sorted(set(normalized_codes)):
        raise EditorFixedPdfReceiptError(
            "renderer diagnostic_codes must be canonical unique sorted values"
        )
    return {
        "page_count": _require_count(summary["page_count"], "renderer page_count"),
        "node_painted": _require_count(summary["node_painted"], "renderer node_painted"),
        "node_partial": _require_count(summary["node_partial"], "renderer node_partial"),
        "node_unsupported": _require_count(
            summary["node_unsupported"],
            "renderer node_unsupported",
        ),
        "diagnostic_codes": normalized_codes,
    }


def _invoke_renderer(
    renderer_command: list[str],
    *,
    packet: dict[str, Any],
    packet_receipt: dict[str, Any],
) -> tuple[bytes, dict[str, Any]]:
    if not renderer_command:
        raise EditorFixedPdfReceiptError("renderer command is required")

    request = {
        "protocol_version": RENDER_REQUEST_VERSION,
        "source_hash": packet_receipt["source_hash"],
        "project_hash": packet_receipt["project_hash"],
        "packet_id": packet_receipt["packet_id"],
        "scene_snapshot_id": packet_receipt["scene_snapshot_id"],
        "flow_id": packet_receipt["flow_id"],
        "packet": packet,
    }

    with tempfile.TemporaryDirectory(prefix="chaptera-fixed-pdf-") as tmp:
        output_path = pathlib.Path(tmp) / "artifact.pdf"
        env = dict(os.environ)
        env["CHAPTERA_PDF_OUTPUT"] = str(output_path)

        completed = subprocess.run(
            renderer_command,
            input=json.dumps(request, ensure_ascii=False, separators=(",", ":")),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            check=False,
        )
        if completed.returncode != 0:
            detail = completed.stderr.strip()
            raise EditorFixedPdfReceiptError(
                "fixed-PDF renderer failed"
                + (f": {detail}" if detail else "")
            )

        try:
            result = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise EditorFixedPdfReceiptError(
                "fixed-PDF renderer returned invalid JSON"
            ) from error

        result = _require_exact_keys(
            result,
            {
                "protocol_version",
                "source_hash",
                "project_hash",
                "packet_id",
                "scene_snapshot_id",
                "flow_id",
                "renderer_revision",
                "target_profile",
                "summary",
            },
            "renderer result",
        )
        if result["protocol_version"] != RENDER_RESULT_VERSION:
            raise EditorFixedPdfReceiptError("renderer result protocol_version mismatch")

        for key in (
            "source_hash",
            "project_hash",
            "packet_id",
            "scene_snapshot_id",
            "flow_id",
        ):
            if result[key] != request[key]:
                raise EditorFixedPdfReceiptError(
                    f"renderer result {key} differs from current-state request"
                )

        renderer_revision = _require_nonempty_string(
            result["renderer_revision"],
            "renderer_revision",
            max_len=128,
        )
        target_profile = _require_nonempty_string(
            result["target_profile"],
            "target_profile",
            max_len=128,
        )
        summary = _validate_render_summary(result["summary"])

        if not output_path.is_file():
            raise EditorFixedPdfReceiptError(
                "renderer succeeded without writing CHAPTERA_PDF_OUTPUT"
            )
        artifact = output_path.read_bytes()
        if len(artifact) < 8 or len(artifact) > MAX_PDF_BYTES:
            raise EditorFixedPdfReceiptError("renderer PDF byte length outside bounded range")
        if not artifact.startswith(b"%PDF-"):
            raise EditorFixedPdfReceiptError("renderer artifact is not a PDF header")

        return artifact, {
            "renderer_revision": renderer_revision,
            "target_profile": target_profile,
            "summary": summary,
        }


def build_receipt(
    renderer_command: list[str],
    *,
    current_state_input: dict[str, Any],
    implementation: str,
    commit_or_build: str,
) -> dict[str, Any]:
    packet, packet_receipt = build_current_fixed_output(
        baseline_graph=current_state_input["resolved_graph"],
        editor_project=current_state_input["editor_project"],
        projection_context=current_state_input["projection_context"],
        shaped_flow=current_state_input["shaped_flow"],
        implementation=implementation,
        commit_or_build=commit_or_build,
    )

    artifact, renderer = _invoke_renderer(
        renderer_command,
        packet=packet,
        packet_receipt=packet_receipt,
    )

    artifact_sha256 = _sha256_bytes(artifact)
    return {
        "receipt_version": RECEIPT_VERSION,
        "producer": {
            "implementation": implementation,
            "commit_or_build": commit_or_build,
            "core_integration": True,
        },
        "source_hash": packet_receipt["source_hash"],
        "project_hash": packet_receipt["project_hash"],
        "packet_id": packet_receipt["packet_id"],
        "scene_snapshot_id": packet_receipt["scene_snapshot_id"],
        "scene_geometry_hash": packet_receipt["scene_geometry_hash"],
        "flow_id": packet_receipt["flow_id"],
        "current_story_states": packet_receipt["current_story_states"],
        "visible_line_count": packet_receipt["visible_line_count"],
        "fixed_run_count": packet_receipt["fixed_run_count"],
        "story_overset": packet_receipt["story_overset"],
        "renderer": renderer,
        "artifact": {
            "format": "pdf",
            "sha256": artifact_sha256,
            "byte_len": len(artifact),
            "header": artifact[:8].decode("ascii", errors="replace"),
        },
        "invariants": {
            "current_editor_project_authoritative": True,
            "source_reparse_after_edit_count": 0,
            "renderer_received_source_bytes": False,
            "native_pub_write_used": False,
            "pdf_renderer_reimplemented_in_rar": False,
            "raw_text_emitted": False,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument(
        "--implementation",
        default="rar-editor-fixed-pdf-producer-boundary-v1",
    )
    parser.add_argument("--commit-or-build", required=True)
    parser.add_argument(
        "renderer_command",
        nargs=argparse.REMAINDER,
        help="external renderer command; prefix with -- if needed",
    )
    args = parser.parse_args()

    renderer_command = list(args.renderer_command)
    if renderer_command and renderer_command[0] == "--":
        renderer_command = renderer_command[1:]

    try:
        current_state_input = _load_input(args.input)
        receipt = build_receipt(
            renderer_command,
            current_state_input=current_state_input,
            implementation=args.implementation,
            commit_or_build=args.commit_or_build,
        )
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(receipt, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(json.dumps({
            "status": "valid",
            "artifact_sha256": receipt["artifact"]["sha256"],
            "artifact_byte_len": receipt["artifact"]["byte_len"],
            "packet_id": receipt["packet_id"],
            "receipt": str(args.output),
        }, sort_keys=True))
        return 0
    except (
        OSError,
        json.JSONDecodeError,
        EditorFixedOutputError,
        EditorFixedPdfReceiptError,
    ) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
