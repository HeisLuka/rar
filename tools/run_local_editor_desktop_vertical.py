#!/usr/bin/env python3
"""Rar-owned local/native closure runner for EDITOR-DESKTOP-VERTICAL-01.

The authorized desktop producer remains local/native and may consume private PUB
bytes. Rar owns the admission/proof boundary. The producer must execute the real
desktop continuity path and:
- write the final EditorProject sidecar to {project};
- write edited IDML/ODG to {export};
- print exactly one source-free observation JSON to stdout.

This runner independently binds the pinned source, verifies source immutability,
inspects the final EditorProject operation stream, validates the export package,
checks exact undo/redo/reopen state equalities, and emits only a sanitized
acceptance receipt.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import platform
import subprocess
import sys
import zipfile
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from validate_editor_desktop_vertical_receipt import (  # noqa: E402
    validate_schema,
    validate_semantics,
)

SAMPLE_SOURCE_HASH = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
SAMPLE_SOURCE_BYTE_LEN = 291840
OBSERVATION_VERSION = "chaptera.editor-desktop-vertical-observation.v1"
RECEIPT_VERSION = "chaptera.editor-desktop-vertical-acceptance.v1"
MAX_PROJECT_BYTES = 64 * 1024 * 1024
MAX_EXPORT_BYTES = 512 * 1024 * 1024


class DesktopVerticalError(RuntimeError):
    pass


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def bind_fixture(
    fixture: pathlib.Path,
    *,
    expected_hash: str,
    expected_len: int,
) -> pathlib.Path:
    path = fixture.expanduser().resolve(strict=True)
    if not path.is_file():
        raise DesktopVerticalError("fixture path is not a regular file")
    actual_len = path.stat().st_size
    if actual_len != expected_len:
        raise DesktopVerticalError(
            f"fixture byte length mismatch: expected={expected_len} actual={actual_len}"
        )
    actual_hash = sha256_file(path)
    if actual_hash != expected_hash:
        raise DesktopVerticalError(
            f"fixture SHA-256 mismatch: expected={expected_hash} actual={actual_hash}"
        )
    return path


def git_head() -> str:
    completed = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    value = completed.stdout.strip()
    if completed.returncode != 0 or len(value) != 40 or any(
        ch not in "0123456789abcdef" for ch in value
    ):
        raise DesktopVerticalError("cannot bind receipt to current Rar git commit")
    return value


def require_exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise DesktopVerticalError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise DesktopVerticalError(
            f"{label} fields mismatch: missing={sorted(expected-actual)} "
            f"extra={sorted(actual-expected)}"
        )
    return value


def render_command(
    template: list[str],
    *,
    fixture: pathlib.Path,
    project_output: pathlib.Path,
    export_output: pathlib.Path,
) -> list[str]:
    if not template:
        raise DesktopVerticalError("desktop producer command is empty")
    replacements = {
        "{fixture}": str(fixture),
        "{project}": str(project_output),
        "{export}": str(export_output),
    }
    for placeholder in replacements:
        count = sum(part.count(placeholder) for part in template)
        if count != 1:
            raise DesktopVerticalError(
                f"desktop producer command must contain {placeholder} exactly once"
            )
    rendered = list(template)
    for placeholder, replacement in replacements.items():
        rendered = [part.replace(placeholder, replacement) for part in rendered]
    return rendered


def validate_observation(
    value: Any,
    *,
    expected_hash: str,
    expected_commit: str,
    export_format: str,
) -> dict[str, Any]:
    observation = require_exact_keys(
        value,
        {
            "protocol_version",
            "source_hash",
            "rar_commit",
            "story_edit",
            "object_move",
            "history",
            "capability_loss",
            "export",
            "invariants",
        },
        "desktop observation",
    )
    if observation["protocol_version"] != OBSERVATION_VERSION:
        raise DesktopVerticalError("desktop observation protocol_version mismatch")
    if observation["source_hash"] != expected_hash:
        raise DesktopVerticalError("desktop observation source_hash mismatch")
    if observation["rar_commit"] != expected_commit:
        raise DesktopVerticalError("desktop producer ran a different Rar commit")

    story = require_exact_keys(
        observation["story_edit"],
        {
            "story_id",
            "operation_kind",
            "capability_admitted",
            "before_state_id",
            "after_state_id",
        },
        "story_edit",
    )
    move = require_exact_keys(
        observation["object_move"],
        {
            "instance_id",
            "projection_kind",
            "origin_node_id",
            "capability_admitted",
            "geometry_sync_policy",
            "before",
            "after",
            "durable_move_count",
            "transient_geometry_operation_count",
        },
        "object_move",
    )
    history = require_exact_keys(
        observation["history"],
        {
            "after_story_state_id",
            "after_move_state_id",
            "undo_state_id",
            "redo_state_id",
            "reopened_state_id",
            "story_state_after_move",
            "story_state_after_undo",
            "story_state_after_redo",
            "story_state_reopened",
        },
        "history",
    )
    capability = require_exact_keys(
        observation["capability_loss"],
        {
            "observed_before_export",
            "blocking_loss_count",
            "approximations_explicit",
            "unsupported_partial_semantics_explicit",
        },
        "capability_loss",
    )
    export = require_exact_keys(
        observation["export"],
        {"format", "edited_story_present", "moved_geometry_present"},
        "export",
    )
    invariants = require_exact_keys(
        observation["invariants"],
        {
            "native_pub_write_used",
            "no_hidden_network_upload",
            "direct_page_local_gate_used",
            "projected_object_mutation_fails_closed",
            "reopen_used_fresh_session",
        },
        "invariants",
    )

    if export["format"] != export_format:
        raise DesktopVerticalError("desktop observation export format mismatch")

    return {
        "story_edit": story,
        "object_move": move,
        "history": history,
        "capability_loss": capability,
        "export": export,
        "invariants": invariants,
    }


def load_and_verify_project(
    path: pathlib.Path,
    *,
    source_hash: str,
    story_id: str,
    moved_node_id: str,
    before_rect: dict[str, Any],
    after_rect: dict[str, Any],
) -> tuple[bytes, dict[str, Any], int, int]:
    if not path.is_file():
        raise DesktopVerticalError("desktop producer did not write EditorProject")
    raw = path.read_bytes()
    if not raw or len(raw) > MAX_PROJECT_BYTES:
        raise DesktopVerticalError("EditorProject byte length outside bounded range")
    try:
        project = json.loads(raw)
    except json.JSONDecodeError as error:
        raise DesktopVerticalError("EditorProject is not valid JSON") from error
    if not isinstance(project, dict):
        raise DesktopVerticalError("EditorProject must be a JSON object")
    if project.get("source_hash") != source_hash:
        raise DesktopVerticalError("EditorProject source identity mismatch")
    schema_version = project.get("schema_version")
    if schema_version not in {"pub-editor-v0.2", "pub-editor-v0.3", "pub-editor-v0.4"}:
        raise DesktopVerticalError("unsupported EditorProject schema_version")
    operations = project.get("operations")
    if not isinstance(operations, list):
        raise DesktopVerticalError("EditorProject.operations must be an array")

    story_ops = [
        operation
        for operation in operations
        if isinstance(operation, dict) and operation.get("kind") == "replace_story_range"
    ]
    move_ops = [
        operation
        for operation in operations
        if isinstance(operation, dict) and operation.get("kind") == "move_node"
    ]
    if len(story_ops) != 1 or len(move_ops) != 1 or len(operations) != 2:
        raise DesktopVerticalError(
            "Desktop V0 EditorProject must contain exactly one Story edit and one MoveNode"
        )
    if story_ops[0].get("story_id") != story_id:
        raise DesktopVerticalError("EditorProject Story operation identity mismatch")
    move = move_ops[0]
    if move.get("node_id") != moved_node_id:
        raise DesktopVerticalError("EditorProject MoveNode identity mismatch")
    if move.get("before") != before_rect or move.get("after") != after_rect:
        raise DesktopVerticalError("EditorProject MoveNode RectEmu differs from observation")

    return raw, project, len(story_ops), len(move_ops)


def verify_export_package(path: pathlib.Path, export_format: str) -> bytes:
    if not path.is_file():
        raise DesktopVerticalError("desktop producer did not write edited export")
    raw = path.read_bytes()
    if not raw or len(raw) > MAX_EXPORT_BYTES:
        raise DesktopVerticalError("export byte length outside bounded range")
    if not zipfile.is_zipfile(path):
        raise DesktopVerticalError(f"{export_format} output is not a ZIP package")

    with zipfile.ZipFile(path) as archive:
        names = set(archive.namelist())
        if export_format == "idml":
            if "designmap.xml" not in names:
                raise DesktopVerticalError("IDML package is missing designmap.xml")
            if not any(name.startswith("Stories/") and name.endswith(".xml") for name in names):
                raise DesktopVerticalError("IDML package has no Story XML")
        elif export_format == "odg":
            if "mimetype" not in names or "content.xml" not in names:
                raise DesktopVerticalError("ODG package is missing mimetype/content.xml")
            if archive.read("mimetype") != b"application/vnd.oasis.opendocument.graphics":
                raise DesktopVerticalError("ODG mimetype mismatch")
        else:
            raise DesktopVerticalError("unsupported edited export format")
    return raw


def run_local_desktop_vertical(
    *,
    fixture: pathlib.Path,
    project_output: pathlib.Path,
    export_output: pathlib.Path,
    receipt_output: pathlib.Path,
    command_template: list[str],
    expected_hash: str = SAMPLE_SOURCE_HASH,
    expected_len: int = SAMPLE_SOURCE_BYTE_LEN,
    rar_commit: str | None = None,
) -> dict[str, Any]:
    fixture = bind_fixture(
        fixture,
        expected_hash=expected_hash,
        expected_len=expected_len,
    )
    rar_commit = rar_commit or git_head()

    export_output = export_output.expanduser().resolve()
    project_output = project_output.expanduser().resolve()
    receipt_output = receipt_output.expanduser().resolve()
    suffix = export_output.suffix.lower()
    if suffix not in {".idml", ".odg"}:
        raise DesktopVerticalError("export output must end in .idml or .odg")
    export_format = suffix[1:]

    for output in (project_output, export_output, receipt_output):
        output.parent.mkdir(parents=True, exist_ok=True)
        if output.exists():
            output.unlink()

    command = render_command(
        command_template,
        fixture=fixture,
        project_output=project_output,
        export_output=export_output,
    )
    env = dict(os.environ)
    env["CHAPTERA_RAR_COMMIT"] = rar_commit
    env["CHAPTERA_SOURCE_HASH"] = expected_hash
    env["CHAPTERA_DESKTOP_ACCEPTANCE_V1"] = "1"

    completed = subprocess.run(
        command,
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace").strip()
        raise DesktopVerticalError(
            "local desktop producer failed" + (f": {detail}" if detail else "")
        )

    try:
        decoded = completed.stdout.decode("utf-8")
        observation_raw = json.loads(decoded)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise DesktopVerticalError(
            "desktop producer stdout is not exactly one UTF-8 JSON value"
        ) from error

    observation = validate_observation(
        observation_raw,
        expected_hash=expected_hash,
        expected_commit=rar_commit,
        export_format=export_format,
    )

    if fixture.stat().st_size != expected_len or sha256_file(fixture) != expected_hash:
        raise DesktopVerticalError("source PUB changed during desktop acceptance")

    story = observation["story_edit"]
    move = observation["object_move"]
    project_raw, project, story_count, move_count = load_and_verify_project(
        project_output,
        source_hash=expected_hash,
        story_id=story["story_id"],
        moved_node_id=move["origin_node_id"],
        before_rect=move["before"],
        after_rect=move["after"],
    )
    export_raw = verify_export_package(export_output, export_format)

    project_sha256 = hashlib.sha256(project_raw).hexdigest()
    export_sha256 = hashlib.sha256(export_raw).hexdigest()

    receipt = {
        "receipt_version": RECEIPT_VERSION,
        "receipt_kind": "real_local",
        "producer": {
            "implementation": "rar-desktop-vertical-local-boundary-v1",
            "commit_or_build": f"rar:{rar_commit}",
            "core_integration": True,
        },
        "source": {
            "sha256": expected_hash,
            "byte_len": expected_len,
            "immutable": True,
        },
        "story_edit": story,
        "object_move": move,
        "history": observation["history"],
        "project": {
            "schema_version": project["schema_version"],
            "sha256": project_sha256,
            "byte_len": len(project_raw),
            "operation_count": len(project["operations"]),
            "story_operation_count": story_count,
            "move_operation_count": move_count,
        },
        "capability_loss": observation["capability_loss"],
        "export": {
            "format": export_format,
            "sha256": export_sha256,
            "byte_len": len(export_raw),
            "package_valid": True,
            "edited_story_present": observation["export"]["edited_story_present"],
            "moved_geometry_present": observation["export"]["moved_geometry_present"],
        },
        "environment": {
            "rar_commit": rar_commit,
            "os": platform.system() or "unknown",
            "arch": platform.machine() or "unknown",
        },
        "invariants": {
            "source_pub_immutable": True,
            "native_pub_write_used": observation["invariants"]["native_pub_write_used"],
            "no_hidden_network_upload": observation["invariants"]["no_hidden_network_upload"],
            "raw_document_text_emitted": False,
            "raw_source_bytes_emitted": False,
            "direct_page_local_gate_used": observation["invariants"]["direct_page_local_gate_used"],
            "projected_object_mutation_fails_closed": observation["invariants"][
                "projected_object_mutation_fails_closed"
            ],
            "reopen_used_fresh_session": observation["invariants"][
                "reopen_used_fresh_session"
            ],
        },
    }

    try:
        validate_schema(receipt)
        validate_semantics(receipt)
    except AssertionError as error:
        raise DesktopVerticalError(str(error)) from error

    receipt_output.write_text(
        json.dumps(receipt, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run and independently verify the Chaptera Desktop V0 continuity loop"
    )
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--project-output", required=True, type=pathlib.Path)
    parser.add_argument("--export-output", required=True, type=pathlib.Path)
    parser.add_argument("--receipt-output", required=True, type=pathlib.Path)
    parser.add_argument("desktop_command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = list(args.desktop_command)
    if command and command[0] == "--":
        command = command[1:]

    try:
        receipt = run_local_desktop_vertical(
            fixture=args.fixture,
            project_output=args.project_output,
            export_output=args.export_output,
            receipt_output=args.receipt_output,
            command_template=command,
        )
    except (DesktopVerticalError, OSError) as error:
        print(str(error), file=sys.stderr)
        return 2

    print(
        json.dumps(
            {
                "status": "valid",
                "source_hash": receipt["source"]["sha256"],
                "project_sha256": receipt["project"]["sha256"],
                "export_format": receipt["export"]["format"],
                "export_sha256": receipt["export"]["sha256"],
                "rar_commit": receipt["environment"]["rar_commit"],
                "receipt": str(args.receipt_output),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
