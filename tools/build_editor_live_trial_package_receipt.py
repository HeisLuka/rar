#!/usr/bin/env python3
"""Build a source-free Chaptera Editor portable-package receipt from a real local run.

The public Rar repository owns the receipt contract and this orchestration.
The authorized local/private producer owns the real Windows runtime smoke.

Sensitive local paths and source-document identity are allowed only inside the
local producer exchange. They are deliberately not serialized into the public
receipt.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
import zipfile
from typing import Any, Sequence

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
README_CONTRACT = "chaptera.editor-live-trial-readme.v1"
RECEIPT_VERSION = "chaptera.editor-live-trial-package-receipt.v1"
SHA_CHARS = set("0123456789abcdef")

RUNTIME_KEYS = {
    "fixture_kind",
    "source_sha256_before",
    "source_sha256_after",
    "reader_only",
    "editor_controls_enabled",
    "native_save_pub_claimed",
    "launch_without_dev_toolchain",
    "real_pub_opened",
    "supported_story_edited",
    "supported_object_dragged",
    "undo_redo_verified",
    "editor_project_saved",
    "close_reopen_reproduced_state",
    "unsupported_actions_fail_closed",
}


def _sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _require_sha256(value: Any, label: str) -> str:
    if (
        not isinstance(value, str)
        or len(value) != 64
        or any(ch not in SHA_CHARS for ch in value)
    ):
        raise RuntimeError(f"{label} must be lowercase SHA-256")
    return value


def _require_exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise RuntimeError(f"{label} must be a JSON object")
    actual = set(value)
    if actual != expected:
        raise RuntimeError(
            f"{label} fields mismatch: missing={sorted(expected - actual)} "
            f"extra={sorted(actual - expected)}"
        )
    return value


def _invoke(command: Sequence[str], payload: dict[str, Any]) -> dict[str, Any]:
    completed = subprocess.run(
        list(command),
        input=json.dumps(payload, ensure_ascii=False, separators=(",", ":")),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "Editor live-trial local producer failed"
            + (f"\nstderr:\n{completed.stderr}" if completed.stderr else "")
        )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError("Editor live-trial local producer returned invalid JSON") from exc
    return _require_exact_keys(value, RUNTIME_KEYS, "runtime producer response")


def _inspect_package(
    zip_path: pathlib.Path,
    *,
    binary_entry: str,
    readme_entry: str,
) -> tuple[str, str]:
    if not zip_path.is_file():
        raise RuntimeError(f"portable ZIP does not exist: {zip_path}")
    if zip_path.suffix.lower() != ".zip":
        raise RuntimeError("portable package must be a .zip file")

    zip_sha = _sha256_file(zip_path)
    with zipfile.ZipFile(zip_path, "r") as archive:
        names = archive.namelist()
        if binary_entry not in names:
            raise RuntimeError(f"Chaptera executable entry is missing: {binary_entry}")
        if not binary_entry.lower().endswith(".exe"):
            raise RuntimeError("Chaptera binary entry must be a Windows .exe")
        if readme_entry not in names:
            raise RuntimeError(f"trial README entry is missing: {readme_entry}")

        lowered = [name.lower() for name in names]
        forbidden_installers = (
            ".msi",
            ".msix",
            ".msixbundle",
            ".appx",
            ".appxbundle",
        )
        if any(name.endswith(forbidden_installers) for name in lowered):
            raise RuntimeError("portable ZIP unexpectedly contains an installer package")
        if any(name.endswith(".pub") for name in lowered):
            raise RuntimeError("portable ZIP must not bundle a source PUB")

        binary_bytes = archive.read(binary_entry)
        if not binary_bytes:
            raise RuntimeError("Chaptera executable entry is empty")
        binary_sha = _sha256_bytes(binary_bytes)

        try:
            readme = archive.read(readme_entry).decode("utf-8")
        except UnicodeDecodeError as exc:
            raise RuntimeError("trial README must be UTF-8") from exc
        if README_CONTRACT not in readme:
            raise RuntimeError("trial README contract marker is missing")

    if binary_sha == zip_sha:
        raise RuntimeError("binary and ZIP identities must be distinct")
    return binary_sha, zip_sha


def build_receipt(
    producer_command: Sequence[str],
    *,
    zip_path: pathlib.Path,
    binary_entry: str,
    readme_entry: str,
    chaptera_version: str,
    fixture_kind: str,
) -> dict[str, Any]:
    if fixture_kind not in {"synthetic_integration", "real_pub_sanitized"}:
        raise RuntimeError("unsupported fixture_kind")
    if not chaptera_version or len(chaptera_version) > 64:
        raise RuntimeError("chaptera_version is required")

    binary_sha, zip_sha = _inspect_package(
        zip_path,
        binary_entry=binary_entry,
        readme_entry=readme_entry,
    )

    proof = _invoke(
        producer_command,
        {
            "action": "editor_live_trial_package_smoke",
            "fixture_kind": fixture_kind,
            "zip_path": str(zip_path.resolve()),
            "binary_entry": binary_entry,
            "readme_entry": readme_entry,
        },
    )
    if proof["fixture_kind"] != fixture_kind:
        raise RuntimeError("runtime producer fixture_kind mismatch")

    before = _require_sha256(proof["source_sha256_before"], "source_sha256_before")
    after = _require_sha256(proof["source_sha256_after"], "source_sha256_after")
    if before != after:
        raise RuntimeError("runtime smoke changed immutable source PUB identity")

    if proof["reader_only"] is not False:
        raise RuntimeError("packaged Chaptera build is still reader-only")
    if proof["editor_controls_enabled"] is not True:
        raise RuntimeError("packaged Chaptera build does not expose Editor controls")
    if proof["native_save_pub_claimed"] is not False:
        raise RuntimeError("portable V0 must not claim native Save PUB")

    runtime_keys = [
        "launch_without_dev_toolchain",
        "real_pub_opened",
        "supported_story_edited",
        "supported_object_dragged",
        "undo_redo_verified",
        "editor_project_saved",
        "close_reopen_reproduced_state",
        "unsupported_actions_fail_closed",
    ]
    failed = [key for key in runtime_keys if proof[key] is not True]
    if failed:
        raise RuntimeError("runtime smoke is incomplete: " + ", ".join(failed))

    receipt = {
        "receipt_version": RECEIPT_VERSION,
        "product": "Chaptera Editor",
        "build": {
            "chaptera_version": chaptera_version,
            "platform": "windows",
            "arch": "x86_64",
            "package_kind": "portable_zip",
            "binary_sha256": binary_sha,
            "zip_sha256": zip_sha,
        },
        "fixture_kind": fixture_kind,
        "editor_boundary": {
            "reader_only": False,
            "editor_controls_enabled": True,
            "native_save_pub_claimed": False,
            "source_pub_immutable": True,
        },
        "runtime_smoke": {key: True for key in runtime_keys},
        "package_contents": {
            "chaptera_executable_present": True,
            "trial_readme_present": True,
            "trial_readme_contract": README_CONTRACT,
            "installer_included": False,
            "code_signing_claimed": False,
            "auto_update_claimed": False,
        },
        "privacy": {
            "pub_bytes_in_receipt": False,
            "pub_filename_in_receipt": False,
            "local_path_in_receipt": False,
            "document_text_in_receipt": False,
            "customer_identity_in_receipt": False,
        },
    }

    sys.path.insert(0, str(TOOLS))
    from validate_editor_live_trial_package_receipt import validate_receipt

    validate_receipt(receipt)
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--zip", required=True, type=pathlib.Path)
    parser.add_argument("--binary-entry", required=True)
    parser.add_argument("--readme-entry", default="TRIAL-README.md")
    parser.add_argument("--chaptera-version", required=True)
    parser.add_argument(
        "--fixture-kind",
        choices=("synthetic_integration", "real_pub_sanitized"),
        default="real_pub_sanitized",
    )
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("producer_command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = list(args.producer_command)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("local producer command is required after --")

    receipt = build_receipt(
        command,
        zip_path=args.zip,
        binary_entry=args.binary_entry,
        readme_entry=args.readme_entry,
        chaptera_version=args.chaptera_version,
        fixture_kind=args.fixture_kind,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(receipt, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "receipt": str(args.output),
                "status": "valid",
                "fixture_kind": args.fixture_kind,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
