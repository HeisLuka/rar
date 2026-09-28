#!/usr/bin/env python3
from __future__ import annotations

import argparse
import pathlib
import re
import sys

PROTECTED_WORKFLOWS = (
    ".github/workflows/chaptera-local-portable-windows.yml",
    ".github/workflows/chaptera-reader-installer.yml",
    ".github/workflows/chaptera-server-package-integrity-v1.yml",
    ".github/workflows/editor-live-trial-package.yml",
    ".github/workflows/pub-ksu-publication4-provenance.yml",
    ".github/workflows/cloud-deploy-v0.yml",
)

USES_RE = re.compile(r"^\s*-\s*uses:\s*([^\s#]+)")
FULL_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
WRITE_PERMISSION_RE = re.compile(
    r"^\s+(actions|attestations|checks|contents|deployments|id-token|issues|packages|pages|pull-requests|security-events|statuses):\s*write\s*(?:#.*)?$"
)


def active_lines(text: str) -> list[str]:
    return [line for line in text.splitlines() if not line.lstrip().startswith("#")]


def has_trigger(lines: list[str], trigger: str) -> bool:
    needle = f"{trigger}:"
    return any(
        line.startswith("  ")
        and not line.startswith("    ")
        and line.strip() == needle
        for line in lines
    )


def audit(path: pathlib.Path) -> list[str]:
    rel = path.as_posix()
    text = path.read_text(encoding="utf-8")
    lines = active_lines(text)
    errors: list[str] = []

    if not any(line.startswith("permissions:") for line in lines):
        errors.append(f"{rel}: missing explicit top-level permissions block")
    if any(line.strip() == "permissions: write-all" for line in lines):
        errors.append(f"{rel}: permissions: write-all is forbidden")
    if has_trigger(lines, "pull_request_target"):
        errors.append(f"{rel}: pull_request_target is forbidden in protected workflows")

    if has_trigger(lines, "pull_request"):
        for lineno, line in enumerate(lines, 1):
            if WRITE_PERMISSION_RE.match(line):
                errors.append(
                    f"{rel}:{lineno}: write/OIDC authority is forbidden when protected workflow runs on pull_request"
                )
            if line.strip() == "secrets: inherit":
                errors.append(
                    f"{rel}:{lineno}: secrets: inherit is forbidden when protected workflow runs on pull_request"
                )

    for lineno, line in enumerate(lines, 1):
        match = USES_RE.match(line)
        if not match:
            continue
        target = match.group(1)
        if target.startswith("./"):
            continue
        if "@" not in target:
            errors.append(
                f"{rel}:{lineno}: external action/reusable workflow lacks @ref: {target}"
            )
            continue
        owner_path, ref = target.rsplit("@", 1)
        if not FULL_SHA_RE.fullmatch(ref):
            errors.append(
                f"{rel}:{lineno}: external action/reusable workflow must be pinned to full 40-char commit SHA: {owner_path}@{ref}"
            )

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path("."))
    args = parser.parse_args()

    root = args.root.resolve()
    errors: list[str] = []
    for rel in PROTECTED_WORKFLOWS:
        path = root / rel
        if not path.is_file():
            errors.append(f"{rel}: protected workflow missing")
            continue
        errors.extend(audit(path))

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1

    print(
        f"CHAPTERA-CI-TRUST-01 PASS: {len(PROTECTED_WORKFLOWS)} protected workflows"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
