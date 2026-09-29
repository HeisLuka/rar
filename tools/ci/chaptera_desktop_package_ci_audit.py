#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

PACKAGE_WORKFLOW = ".github/workflows/chaptera-desktop-package-integrity-v1.yml"
BASELINE_PATH = Path("tools/ci/chaptera_desktop_package_ci_before_v1.json")
DEFAULT_OUT = Path("target/ci/chaptera-desktop-package-routing-receipt.json")

FEATURE_WORKFLOWS = (
    "chaptera-desktop-fallback-font-resource-v1.yml",
    "editor-desktop-text-session-restore-v1.yml",
    "viewer-editor-created-node-scene-sync-v1.yml",
)

EXPECTED_CONCURRENCY = (
    "group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.run_id }}",
    "cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
)

FMT_RE = re.compile(
    r"cargo\s+fmt(?:\s+--manifest-path\s+apps/chaptera-desktop/Cargo\.toml|\s+-p\s+chaptera-desktop|\s+--package\s+chaptera-desktop)[^\n]*"
)
CHECK_RE = re.compile(
    r"cargo\s+check(?:\s+--manifest-path\s+apps/chaptera-desktop/Cargo\.toml|\s+-p\s+chaptera-desktop|\s+--package\s+chaptera-desktop)[^\n]*"
)
CLIPPY_RE = re.compile(
    r"cargo\s+clippy(?:\s+--manifest-path\s+apps/chaptera-desktop/Cargo\.toml|\s+-p\s+chaptera-desktop|\s+--package\s+chaptera-desktop)[^\n]*"
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


def desktop_package_commands(text: str) -> dict[str, int]:
    return {
        "fmt": len(FMT_RE.findall(text)),
        "check": len(CHECK_RE.findall(text)),
        "clippy": len(CLIPPY_RE.findall(text)),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", type=Path, default=BASELINE_PATH)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args()

    baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    representative_path = baseline["representative_changed_path"]
    violations: list[str] = []
    feature_after: dict[str, dict[str, object]] = {}

    expected_main_rs_routes = {
        "chaptera-desktop-fallback-font-resource-v1.yml": False,
        "editor-desktop-text-session-restore-v1.yml": True,
        "viewer-editor-created-node-scene-sync-v1.yml": True,
    }

    for workflow_name in FEATURE_WORKFLOWS:
        path = Path(".github/workflows") / workflow_name
        text = path.read_text(encoding="utf-8")
        commands = desktop_package_commands(text)
        routes_main = workflow_triggers_for_path(text, representative_path)
        feature_after[workflow_name] = {
            "desktop_package_commands": commands,
            "main_rs_admitted": routes_main,
        }
        if any(commands.values()):
            violations.append(
                f"{workflow_name}: repeats shared Desktop fmt/check/clippy commands: {commands}"
            )
        expected_route = expected_main_rs_routes[workflow_name]
        if routes_main != expected_route:
            violations.append(
                f"{workflow_name}: {representative_path} route={routes_main} expected={expected_route}"
            )

    fallback_text = Path(
        ".github/workflows/chaptera-desktop-fallback-font-resource-v1.yml"
    ).read_text(encoding="utf-8")
    if 'apps/chaptera-desktop/**' in fallback_text:
        violations.append(
            "chaptera-desktop-fallback-font-resource-v1.yml retains broad apps/chaptera-desktop/** admission"
        )

    package_text = Path(PACKAGE_WORKFLOW).read_text(encoding="utf-8")
    required_package_commands = (
        "cargo fmt --manifest-path apps/chaptera-desktop/Cargo.toml -- --check",
        "cargo check -p chaptera-desktop --all-targets",
        "cargo clippy -p chaptera-desktop --all-targets --",
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
    if not workflow_triggers_for_path(package_text, representative_path):
        violations.append(
            f"{PACKAGE_WORKFLOW}: does not admit representative path {representative_path}"
        )

    package_commands = desktop_package_commands(package_text)
    if package_commands != {"fmt": 1, "check": 1, "clippy": 1}:
        violations.append(
            f"{PACKAGE_WORKFLOW}: expected exactly one fmt/check/clippy owner, got {package_commands}"
        )

    slice_workflows = {
        PACKAGE_WORKFLOW: package_text,
        **{
            f".github/workflows/{name}": (
                Path(".github/workflows") / name
            ).read_text(encoding="utf-8")
            for name in FEATURE_WORKFLOWS
        },
    }
    representative_routes = {
        name: workflow_triggers_for_path(text, representative_path)
        for name, text in slice_workflows.items()
    }
    shared_command_owners = {
        name: desktop_package_commands(text)
        for name, text in slice_workflows.items()
        if workflow_triggers_for_path(text, representative_path)
        and any(desktop_package_commands(text).values())
    }
    if set(shared_command_owners) != {PACKAGE_WORKFLOW}:
        violations.append(
            "representative main.rs routing must have exactly one shared Desktop package command owner; "
            f"found {sorted(shared_command_owners)}"
        )

    receipt = {
        "schema_version": "chaptera.ci.desktop-package-routing-receipt.v1",
        "representative_changed_path": representative_path,
        "baseline": baseline,
        "after_static": {
            "feature_workflows": feature_after,
            "package_workflow_commands": package_commands,
            "representative_routes": representative_routes,
            "representative_shared_command_owners": shared_command_owners,
            "measurement_state": "pending_live_representative_main_rs_change_after_merge",
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
