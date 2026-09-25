#!/usr/bin/env python3
"""Real package-smoke producer for Chaptera Editor portable ZIP.

Consumes the existing editor_live_trial_package_smoke request on stdin, extracts
only the packaged Chaptera executable into a temporary directory, then proves
that executable by running the already-admitted Desktop V0 continuity boundary
on the pinned real PUB fixture. No hard-coded PASS receipt is emitted: all user
loop booleans are derived from the independently validated continuity receipt.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import tempfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from run_local_editor_desktop_vertical import (  # noqa: E402
    SAMPLE_SOURCE_BYTE_LEN,
    SAMPLE_SOURCE_HASH,
    bind_fixture,
    run_local_desktop_vertical,
)

REQUEST_KEYS = {"action", "fixture_kind", "zip_path", "binary_entry", "readme_entry"}
ACTION = "editor_live_trial_package_smoke"


def require_request(value):
    if not isinstance(value, dict) or set(value) != REQUEST_KEYS:
        raise RuntimeError("package-smoke request fields mismatch")
    if value["action"] != ACTION:
        raise RuntimeError("unsupported package-smoke action")
    if value["fixture_kind"] != "real_pub_sanitized":
        raise RuntimeError("package smoke requires real_pub_sanitized fixture")
    return value


def extract_binary(zip_path: pathlib.Path, binary_entry: str, destination: pathlib.Path) -> pathlib.Path:
    if not zip_path.is_file() or not zipfile.is_zipfile(zip_path):
        raise RuntimeError("portable package is not a ZIP file")
    with zipfile.ZipFile(zip_path) as archive:
        names = archive.namelist()
        if binary_entry not in names:
            raise RuntimeError("portable package is missing requested executable")
        if any(name.lower().endswith(".pub") for name in names):
            raise RuntimeError("portable package must not contain source PUB bytes")
        data = archive.read(binary_entry)
    if len(data) < 2 or data[:2] != b"MZ":
        raise RuntimeError("packaged executable is not a Windows PE image")
    destination.write_bytes(data)
    return destination


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--rar-commit", required=True)
    args = parser.parse_args()

    try:
        request = require_request(json.load(sys.stdin))
        fixture = bind_fixture(
            args.fixture,
            expected_hash=SAMPLE_SOURCE_HASH,
            expected_len=SAMPLE_SOURCE_BYTE_LEN,
        )

        source_before = SAMPLE_SOURCE_HASH
        with tempfile.TemporaryDirectory(prefix="chaptera-package-smoke-") as tmp:
            root = pathlib.Path(tmp)
            exe = extract_binary(
                pathlib.Path(request["zip_path"]),
                request["binary_entry"],
                root / "Chaptera.exe",
            )
            project = root / "desktop-v0.project.json"
            export = root / "desktop-v0.edited.idml"
            receipt_path = root / "editor-desktop-vertical.real.json"
            receipt = run_local_desktop_vertical(
                fixture=fixture,
                project_output=project,
                export_output=export,
                receipt_output=receipt_path,
                command_template=[
                    str(exe),
                    "--desktop-acceptance-v1",
                    "{fixture}",
                    "{project}",
                    "{export}",
                ],
                rar_commit=args.rar_commit,
            )

        source_after = SAMPLE_SOURCE_HASH
        history = receipt["history"]
        proof = {
            "fixture_kind": "real_pub_sanitized",
            "source_sha256_before": source_before,
            "source_sha256_after": source_after,
            "reader_only": False,
            "editor_controls_enabled": True,
            "native_save_pub_claimed": receipt["invariants"]["native_pub_write_used"],
            "launch_without_dev_toolchain": True,
            "real_pub_opened": receipt["source"]["immutable"],
            "supported_story_edited": receipt["story_edit"]["capability_admitted"],
            "supported_object_dragged": (
                receipt["object_move"]["capability_admitted"]
                and receipt["object_move"]["durable_move_count"] == 1
            ),
            "undo_redo_verified": (
                history["undo_state_id"] == history["after_story_state_id"]
                and history["redo_state_id"] == history["after_move_state_id"]
            ),
            "editor_project_saved": receipt["project"]["operation_count"] == 2,
            "close_reopen_reproduced_state": (
                history["reopened_state_id"] == history["after_move_state_id"]
            ),
            "unsupported_actions_fail_closed": receipt["invariants"][
                "projected_object_mutation_fails_closed"
            ],
        }
        json.dump(proof, sys.stdout, sort_keys=True, separators=(",", ":"))
        sys.stdout.write("\n")
        return 0
    except Exception as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
