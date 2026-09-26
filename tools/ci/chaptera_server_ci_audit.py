#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

PACKAGE_WORKFLOW = ".github/workflows/chaptera-server-package-integrity-v1.yml"
BASELINE_PATH = Path("tools/ci/chaptera_server_ci_before_v1.json")
DEFAULT_OUT = Path("target/ci/chaptera-server-routing-receipt.json")

FMT_RE = re.compile(r"cargo\s+fmt[^\n]*chaptera-server")
CLIPPY_RE = re.compile(r"cargo\s+clippy\s+-p\s+chaptera-server\s+--all-targets\s+--\s+-D\s+warnings")
FULL_TEST_RE = re.compile(
    r"^\s*run:\s*['\"]?cargo\s+test\s+-p\s+chaptera-server['\"]?\s*$",
    re.MULTILINE,
)
FULL_MANIFEST_TEST_RE = re.compile(
    r"^\s*run:\s*['\"]?cargo\s+test\s+--manifest-path\s+apps/chaptera-server/Cargo\.toml['\"]?\s*$",
    re.MULTILINE,
)
EXPECTED_CONCURRENCY = (
    "group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.run_id }}",
    "cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
)


def github_path_match(pattern: str, path: str) -> bool:
    token = "\0DOUBLESTAR\0"
    escaped = re.escape(pattern.replace("**", token))
    escaped = escaped.replace(re.escape(token), ".*")
    escaped = escaped.replace(r"\*", "[^/]*")
    escaped = escaped.replace(r"\?", "[^/]")
    return re.fullmatch(escaped, path) is not None


def parse_pull_request_paths(text: str) -> tuple[bool, list[str] | None]:
    in_on = False
    in_pull_request = False
    in_paths = False
    saw_pull_request = False
    saw_paths = False
    paths: list[str] = []

    for raw in text.splitlines():
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        indent = len(raw) - len(raw.lstrip(" "))
        stripped = raw.strip()

        if indent == 0:
            if stripped == "on:":
                in_on = True
                in_pull_request = False
                in_paths = False
                continue
            if in_on:
                break

        if not in_on:
            continue

        if indent == 2:
            if stripped.startswith("pull_request:"):
                saw_pull_request = True
                in_pull_request = True
                in_paths = False
            else:
                in_pull_request = False
                in_paths = False
            continue

        if in_pull_request and indent == 4 and stripped.startswith("paths:"):
            saw_paths = True
            inline = stripped[len("paths:"):].strip()
            if inline:
                if not (inline.startswith("[") and inline.endswith("]")):
                    raise ValueError(f"unsupported inline paths syntax: {inline}")
                for item in inline[1:-1].split(","):
                    value = item.strip().strip('"').strip("'")
                    if value:
                        paths.append(value)
                in_paths = False
            else:
                in_paths = True
            continue

        if in_paths:
            if indent >= 6 and stripped.startswith("-"):
                value = stripped[1:].strip().strip('"').strip("'")
                paths.append(value)
                continue
            if indent <= 4:
                in_paths = False

    return saw_pull_request, paths if saw_paths else None


def workflow_triggers_for_path(text: str, changed_path: str) -> bool:
    has_pr, paths = parse_pull_request_paths(text)
    if not has_pr:
        return False
    if paths is None:
        return True
    return any(github_path_match(pattern, changed_path) for pattern in paths)


def package_command_summary(text: str) -> dict[str, int]:
    return {
        "fmt": len(FMT_RE.findall(text)),
        "clippy": len(CLIPPY_RE.findall(text)),
        "full_package_test": len(FULL_TEST_RE.findall(text))
        + len(FULL_MANIFEST_TEST_RE.findall(text)),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", type=Path, default=BASELINE_PATH)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args()

    baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    representative_path = baseline["representative_changed_path"]
    feature_workflows: list[str] = baseline["feature_workflows"]
    violations: list[str] = []
    after_feature_commands: dict[str, dict[str, int]] = {}

    for workflow_name in feature_workflows:
        path = Path(".github/workflows") / workflow_name
        text = path.read_text(encoding="utf-8")
        commands = package_command_summary(text)
        after_feature_commands[workflow_name] = commands
        if any(commands.values()):
            violations.append(
                f"{workflow_name}: repeats package-wide fmt/clippy/full-test gate: {commands}"
            )
        if 'apps/chaptera-server/**' in text:
            violations.append(
                f"{workflow_name}: retains broad apps/chaptera-server/** admission"
            )
        if 'apps/chaptera-server/src/lib.rs' in text:
            violations.append(
                f"{workflow_name}: retains shared lib.rs admission"
            )
        for expected in EXPECTED_CONCURRENCY:
            if expected not in text:
                violations.append(
                    f"{workflow_name}: missing latest-head concurrency line: {expected}"
                )

    package_path = Path(PACKAGE_WORKFLOW)
    package_text = package_path.read_text(encoding="utf-8")
    required_package_commands = (
        "cargo fmt -p chaptera-server -- --check",
        "cargo clippy -p chaptera-server --all-targets -- -D warnings",
        "cargo test -p chaptera-server",
    )
    for command in required_package_commands:
        if command not in package_text:
            violations.append(
                f"{PACKAGE_WORKFLOW}: missing authoritative command {command!r}"
            )
    for expected in EXPECTED_CONCURRENCY:
        if expected not in package_text:
            violations.append(
                f"{PACKAGE_WORKFLOW}: missing latest-head concurrency line: {expected}"
            )

    triggered: list[str] = []
    triggered_command_workflows: dict[str, dict[str, int]] = {}
    workflow_dir = Path(".github/workflows")
    for path in sorted([*workflow_dir.glob("*.yml"), *workflow_dir.glob("*.yaml")]):
        text = path.read_text(encoding="utf-8")
        if workflow_triggers_for_path(text, representative_path):
            rel = str(path).replace("\\", "/")
            triggered.append(rel)
            commands = package_command_summary(text)
            if any(commands.values()):
                triggered_command_workflows[rel] = commands

    cloud_slice = {f".github/workflows/{name}" for name in feature_workflows}
    cloud_slice.add(PACKAGE_WORKFLOW)
    shared_gate_in_slice = {
        workflow_name: commands
        for workflow_name, commands in triggered_command_workflows.items()
        if workflow_name in cloud_slice and any(commands.values())
    }
    if set(shared_gate_in_slice) != {PACKAGE_WORKFLOW}:
        violations.append(
            "representative lib.rs routing must have exactly one package-wide gate "
            f"inside the Cloud/server slice; found {sorted(shared_gate_in_slice)}"
        )

    receipt = {
        "schema_version": "chaptera.ci.chaptera-server-routing-receipt.v1",
        "representative_changed_path": representative_path,
        "baseline": baseline,
        "after_static": {
            "routing_parser_supports_inline_paths": True,
            "triggered_workflow_count": len(triggered),
            "triggered_workflows": triggered,
            "cloud_server_shared_gate_workflows": shared_gate_in_slice,
            "all_triggered_workflows_with_package_commands": triggered_command_workflows,
            "feature_package_command_counts": after_feature_commands,
            "after_measured_runner_minutes": None,
            "measurement_state": "pending_live_representative_lib_rs_change_after_merge",
        },
        "violations": violations,
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 1 if violations else 0


if __name__ == "__main__":
    raise SystemExit(main())
