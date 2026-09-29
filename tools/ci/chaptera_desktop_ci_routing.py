#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

WORKFLOWS = {
    "reader_windows": Path(".github/workflows/chaptera-reader-windows.yml"),
    "editor_windows": Path(".github/workflows/chaptera-desktop-windows.yml"),
    "reader_installer": Path(".github/workflows/chaptera-reader-installer.yml"),
    "suite_handoff": Path(".github/workflows/chaptera-suite-handoff-v1.yml"),
}

UI_ONLY_PATH = "apps/chaptera-desktop/src/main.rs"
RECEIPT_SCHEMA = "chaptera.desktop-ci-routing.v1"


def extract_pull_request_paths(path: Path) -> list[str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    in_pull_request = False
    in_paths = False
    out: list[str] = []

    for line in lines:
        if line == "  pull_request:":
            in_pull_request = True
            in_paths = False
            continue
        if in_pull_request and line.startswith("  ") and not line.startswith("    ") and line.strip():
            break
        if in_pull_request and line == "    paths:":
            in_paths = True
            continue
        if in_paths:
            match = re.match(r'^      -\s+["\']?(.+?)["\']?$', line)
            if match:
                out.append(match.group(1))
                continue
            if line.strip() and not line.startswith("      "):
                break

    if not out:
        raise ValueError(f"{path}: pull_request paths not found")
    return out


def github_path_match(pattern: str, path: str) -> bool:
    token = "__DOUBLESTAR__"
    escaped = re.escape(pattern.replace("**", token))
    escaped = escaped.replace(re.escape(token), ".*")
    escaped = escaped.replace(r"\*", "[^/]*")
    escaped = escaped.replace(r"\?", "[^/]")
    return re.fullmatch(escaped, path) is not None


def routes(patterns: list[str], path: str) -> bool:
    return any(github_path_match(pattern, path) for pattern in patterns)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--out",
        type=Path,
        default=Path("target/ci/chaptera-desktop-ci-routing-receipt.json"),
    )
    args = parser.parse_args()

    workflow_paths = {
        name: extract_pull_request_paths(path)
        for name, path in WORKFLOWS.items()
    }

    ui_routes = {
        name: routes(patterns, UI_ONLY_PATH)
        for name, patterns in workflow_paths.items()
    }

    violations: list[str] = []
    expected_ui_routes = {
        "reader_windows": True,
        "editor_windows": True,
        "reader_installer": False,
        "suite_handoff": False,
    }
    for name, expected in expected_ui_routes.items():
        if ui_routes[name] != expected:
            violations.append(
                f"{name}: {UI_ONLY_PATH} route={ui_routes[name]} expected={expected}"
            )

    positive_controls = {
        "reader_installer": [
            "apps/chaptera-desktop/Cargo.toml",
            "packages/product/reader-portable/README.md",
            "installer/windows/chaptera-reader.iss",
            ".github/workflows/chaptera-reader-installer.yml",
        ],
        "suite_handoff": [
            "Cargo.toml",
            "apps/chaptera-desktop/Cargo.toml",
            "apps/chaptera-desktop/src/diagnostic_sweep.rs",
            "apps/chaptera-desktop/src/suite_handoff_cli.rs",
            "crates/chaptera-suite-handoff/src/lib.rs",
            "apps/chaptera-rescue/src/main.rs",
            "packages/product/desktop-suite/v1/handoff-schema.json",
            "tools/validate_chaptera_suite_handoff.py",
            ".github/workflows/chaptera-suite-handoff-v1.yml",
        ],
    }
    positive_results: dict[str, dict[str, bool]] = {}
    for workflow_name, paths in positive_controls.items():
        positive_results[workflow_name] = {}
        for path in paths:
            admitted = routes(workflow_paths[workflow_name], path)
            positive_results[workflow_name][path] = admitted
            if not admitted:
                violations.append(f"{workflow_name}: positive control not admitted: {path}")

    main_source = Path("apps/chaptera-desktop/src/main.rs").read_text(encoding="utf-8")
    handoff_source = Path("apps/chaptera-desktop/src/suite_handoff_cli.rs").read_text(
        encoding="utf-8"
    )
    diagnostic_source = Path("apps/chaptera-desktop/src/diagnostic_sweep.rs").read_text(
        encoding="utf-8"
    )

    source_contract = {
        "main_declares_suite_handoff_module": "mod suite_handoff_cli;" in main_source,
        "main_routes_suite_handoff_adapter": (
            "suite_handoff_cli::try_handle(first_arg.as_deref(), &mut args)" in main_source
        ),
        "main_has_no_handoff_flag_implementation": (
            "--handoff-create-v1" not in main_source
            and "--handoff-accept-v1" not in main_source
        ),
        "adapter_owns_create_flag": "--handoff-create-v1" in handoff_source,
        "adapter_owns_accept_flag": "--handoff-accept-v1" in handoff_source,
        "diagnostic_sweep_owns_smoke_check": "pub fn smoke_check(" in diagnostic_source,
    }
    for key, ok in source_contract.items():
        if not ok:
            violations.append(f"source contract failed: {key}")

    receipt = {
        "schema": RECEIPT_SCHEMA,
        "ui_only_probe": {
            "path": UI_ONLY_PATH,
            "routes": ui_routes,
            "expected": expected_ui_routes,
        },
        "positive_controls": positive_results,
        "source_contract": source_contract,
        "historical_baseline": {
            "pull_request": 822,
            "sample_kind": "desktop_ui_only_zoom",
            "summed_runner_minutes": 32.35,
            "reader_installer_runner_minutes": 8.72,
            "suite_handoff_runner_minutes": 7.90,
            "avoidable_installer_plus_handoff_runner_minutes": 16.62,
            "note": "historical bounded sample; not a universal savings claim",
        },
        "violations": violations,
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2, sort_keys=True))

    if violations:
        for violation in violations:
            print(f"::error::{violation}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
