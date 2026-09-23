#!/usr/bin/env python3
"""Fail-closed validator for PUB native research experiment packets."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path


SCHEMA = "pub-research-experiment.v1"
ENV_RE = re.compile(r"^publisher-[0-9]{4}$")
SAFE_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{1,127}$")


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"packet validation failed: {message}")


def resolve_inside(root: Path, value: str, field: str) -> Path:
    candidate = (root / value).resolve()
    try:
        candidate.relative_to(root)
    except ValueError:
        fail(f"{field} escapes repository root: {value}")
    return candidate


def require_string_list(value: object, field: str) -> list[str]:
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        fail(f"{field} must be an array of strings")
    return value


def require_safe_id(value: object, field: str) -> str:
    if not isinstance(value, str) or not SAFE_ID_RE.fullmatch(value):
        fail(f"{field} must be a 2-128 character safe identifier")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--packet", required=True)
    parser.add_argument("--expected-environment")
    args = parser.parse_args()

    root = Path.cwd().resolve()
    packet_path = resolve_inside(root, args.packet, "packet path")
    if not packet_path.is_file():
        fail(f"packet does not exist: {args.packet}")

    try:
        packet = json.loads(packet_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {args.packet}: {exc}")

    if not isinstance(packet, dict):
        fail("packet root must be an object")
    if packet.get("schema") != SCHEMA:
        fail(f"schema must be {SCHEMA!r}")

    experiment_id = packet.get("id")
    if not isinstance(experiment_id, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{1,79}", experiment_id):
        fail("id must be 2-80 safe identifier characters")

    environment = packet.get("publisher_environment")
    if not isinstance(environment, str) or not ENV_RE.fullmatch(environment):
        fail("publisher_environment must look like publisher-YYYY")
    if args.expected_environment and environment != args.expected_environment:
        fail(
            f"packet environment {environment!r} does not match requested "
            f"{args.expected_environment!r}"
        )

    requires_publisher = packet.get("requires_publisher")
    if not isinstance(requires_publisher, bool):
        fail("requires_publisher must be boolean")

    operation = packet.get("operation")
    if not isinstance(operation, dict):
        fail("operation must be an object")
    if operation.get("shell") != "pwsh":
        fail("operation.shell must be 'pwsh' in v1")
    script = operation.get("script")
    if not isinstance(script, str) or not script.endswith(".ps1"):
        fail("operation.script must be a repository-relative .ps1 path")
    script_path = resolve_inside(root, script, "operation.script")
    if not script_path.is_file():
        fail(f"operation script does not exist: {script}")
    require_string_list(operation.get("args", []), "operation.args")

    evidence = packet.get("evidence")
    if not isinstance(evidence, dict):
        fail("evidence must be an object")
    required = require_string_list(evidence.get("required", []), "evidence.required")
    if not required:
        fail("evidence.required must contain at least one path")
    for item in required:
        if Path(item).is_absolute() or ".." in Path(item).parts:
            fail(f"evidence.required entry is not a safe relative path: {item}")

    fixture = packet.get("fixture")
    if fixture is not None:
        if not isinstance(fixture, dict):
            fail("fixture must be null or an object")
        source = fixture.get("source")
        if source not in {"repo", "runner-root"}:
            fail("fixture.source must be 'repo' or 'runner-root'")
        relative_path = fixture.get("relative_path")
        if not isinstance(relative_path, str) or not relative_path:
            fail("fixture.relative_path must be a non-empty string")
        if Path(relative_path).is_absolute() or ".." in Path(relative_path).parts:
            fail("fixture.relative_path must stay below its declared root")
        expected_sha = fixture.get("sha256")
        if expected_sha is not None and (
            not isinstance(expected_sha, str)
            or not re.fullmatch(r"[0-9a-fA-F]{64}", expected_sha)
        ):
            fail("fixture.sha256 must be null or a 64-character hex SHA-256")

    reset = packet.get("reset")
    reset_required = False
    reset_baseline_id = None
    reset_snapshot_id = None
    if reset is not None:
        if not isinstance(reset, dict):
            fail("reset must be null or an object")
        reset_required = reset.get("required")
        if not isinstance(reset_required, bool):
            fail("reset.required must be boolean")
        if reset_required:
            reset_baseline_id = require_safe_id(reset.get("baseline_id"), "reset.baseline_id")
            reset_snapshot_id = require_safe_id(reset.get("snapshot_id"), "reset.snapshot_id")
        else:
            for field in ("baseline_id", "snapshot_id"):
                value = reset.get(field)
                if value is not None:
                    require_safe_id(value, f"reset.{field}")

    publisher = packet.get("publisher", {})
    if not isinstance(publisher, dict):
        fail("publisher must be an object when present")
    version_prefix = publisher.get("version_prefix")
    if version_prefix is not None and not isinstance(version_prefix, str):
        fail("publisher.version_prefix must be a string when present")
    exe_sha = publisher.get("exe_sha256")
    if exe_sha is not None and (
        not isinstance(exe_sha, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", exe_sha)
    ):
        fail("publisher.exe_sha256 must be a 64-character hex SHA-256 when present")
    if requires_publisher and not version_prefix and not exe_sha:
        fail("Publisher experiments require publisher.version_prefix or publisher.exe_sha256")

    if fixture is not None and fixture.get("source") == "runner-root" and not fixture.get("sha256"):
        fail("runner-root fixtures require an explicit SHA-256")

    digest = hashlib.sha256(packet_path.read_bytes()).hexdigest()
    print(
        json.dumps(
            {
                "schema": SCHEMA,
                "id": experiment_id,
                "publisher_environment": environment,
                "requires_publisher": requires_publisher,
                "operation_script": script,
                "reset_required": reset_required,
                "reset_baseline_id": reset_baseline_id,
                "reset_snapshot_id": reset_snapshot_id,
                "packet_sha256": digest,
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
