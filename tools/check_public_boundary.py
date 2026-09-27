#!/usr/bin/env python3
from __future__ import annotations

import os
import pathlib
import re
import subprocess
import sys

FROZEN_COMPONENTS = {"yab", "miy", "pub-rs"}
BASE = os.environ.get("PUBLIC_BOUNDARY_BASE", "origin/main")
HEAD = os.environ.get("PUBLIC_BOUNDARY_HEAD", "HEAD")

RESEARCH_PREFIXES = (
    "research/",
    "packages/research/",
    "tools/",
)
TEXT_SUFFIXES = {
    ".c",
    ".cc",
    ".cpp",
    ".go",
    ".js",
    ".json",
    ".mjs",
    ".ps1",
    ".py",
    ".rs",
    ".sh",
    ".toml",
    ".ts",
    ".yaml",
    ".yml",
}
PRIVATE_PAYLOAD_SUFFIXES = {".pub", ".puz"}
SAFE_FIXTURE_MARKERS = {"public", "public-safe", "public_safe", "synthetic"}

WINDOWS_ABS_RE = re.compile(
    r"(?i)(?<![A-Za-z0-9_])([A-Z]:[\\/][^\s\"'<>|]+)"
)
UNIX_USER_RE = re.compile(
    r"(?<![A-Za-z0-9_])((?:/home|/Users)/[A-Za-z0-9._-]+/[^\s\"'<>|]+)"
)
PRIVATE_PATH_RE = re.compile(
    r"(?i)(?:^|[\\/])(?:private[-_]?corpus|private[-_]?evidence|local[-_]?corpus|"
    r"local[-_]?evidence|source[-_]?private|private[-_]?source)(?:[\\/])"
)
SECRET_RE = re.compile(
    r"(?:ghp_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|"
    r"sk-[A-Za-z0-9_-]{20,}|AKIA[0-9A-Z]{16})"
)
PRIVATE_KEY_RE = re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")
RAW_PUB_PAYLOAD_RE = re.compile(
    r"""(?is)(?:raw_pub|pub_payload|pub_bytes|payload_(?:hex|base64))["']?
        \s*[:=]\s*["']([A-Za-z0-9+/=\s]{512,})""",
    re.VERBOSE,
)

SAFE_WINDOWS_PREFIXES = (
    "c:\\windows\\",
    "c:/windows/",
    "c:\\program files\\",
    "c:/program files/",
    "c:\\program files (x86)\\",
    "c:/program files (x86)/",
    "c:\\programdata\\",
    "c:/programdata/",
)
SAFE_UNIX_PREFIXES = (
    "/home/runner/work/",
    "/Users/runner/work/",
)


def changed_paths() -> list[str]:
    out = subprocess.check_output(
        ["git", "diff", "--name-only", "--diff-filter=ACMR", f"{BASE}...{HEAD}"],
        text=True,
    )
    return [p.strip() for p in out.splitlines() if p.strip()]


def frozen_hits(path: str) -> list[str]:
    hits = set()
    for raw in pathlib.PurePosixPath(path).parts:
        part = raw.lower()
        for frozen in FROZEN_COMPONENTS:
            if (
                part == frozen
                or part.startswith(frozen + "-")
                or part.endswith("-" + frozen)
                or ("-" + frozen + "-") in part
            ):
                hits.add(frozen)
    return sorted(hits)


def is_research_surface(path: str) -> bool:
    normalized = pathlib.PurePosixPath(path).as_posix().lower()
    return any(normalized.startswith(prefix) for prefix in RESEARCH_PREFIXES)


def safe_fixture_path(path: str) -> bool:
    parts = {part.lower() for part in pathlib.PurePosixPath(path).parts}
    return "fixtures" in parts and bool(parts & SAFE_FIXTURE_MARKERS)


def path_violations(path: str) -> list[str]:
    violations = []
    hits = frozen_hits(path)
    if hits:
        violations.append(
            "forbidden frozen/private repository component(s): " + ", ".join(hits)
        )

    suffix = pathlib.PurePosixPath(path).suffix.lower()
    if (
        is_research_surface(path)
        and suffix in PRIVATE_PAYLOAD_SUFFIXES
        and not safe_fixture_path(path)
    ):
        violations.append(
            f"raw {suffix} research payload must live only in an explicitly public/synthetic fixture path"
        )
    return violations


def _safe_windows_path(value: str) -> bool:
    normalized = value.replace("\\\\", "\\").lower()
    return normalized.startswith(SAFE_WINDOWS_PREFIXES)


def _safe_unix_path(value: str) -> bool:
    return value.startswith(SAFE_UNIX_PREFIXES)


def content_violations(path: str, text: str) -> list[str]:
    if not is_research_surface(path):
        return []
    if pathlib.PurePosixPath(path).suffix.lower() not in TEXT_SUFFIXES:
        return []

    violations = []

    for match in WINDOWS_ABS_RE.finditer(text):
        value = match.group(1)
        if not _safe_windows_path(value):
            violations.append(f"local Windows absolute path: {value[:120]}")

    for match in UNIX_USER_RE.finditer(text):
        value = match.group(1)
        if not _safe_unix_path(value):
            violations.append(f"local user-home absolute path: {value[:120]}")

    if PRIVATE_PATH_RE.search(text):
        violations.append("private/local corpus or evidence path marker")

    if SECRET_RE.search(text):
        violations.append("credential/token-shaped literal")

    if PRIVATE_KEY_RE.search(text):
        violations.append("embedded private-key block")

    if RAW_PUB_PAYLOAD_RE.search(text):
        violations.append("large inline PUB/raw payload literal")

    return sorted(set(violations))


def file_violations(path: str) -> list[str]:
    violations = path_violations(path)
    if not is_research_surface(path):
        return violations
    if pathlib.PurePosixPath(path).suffix.lower() not in TEXT_SUFFIXES:
        return violations

    candidate = pathlib.Path(path)
    try:
        text = candidate.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        violations.append("text-like research file is not valid UTF-8")
        return violations
    except OSError as exc:
        violations.append(f"cannot inspect changed research file: {exc}")
        return violations

    violations.extend(content_violations(path, text))
    return sorted(set(violations))


def run_guard(paths: list[str] | None = None) -> list[tuple[str, list[str]]]:
    bad = []
    for path in changed_paths() if paths is None else paths:
        violations = file_violations(path)
        if violations:
            bad.append((path, violations))
    return bad


def main() -> int:
    bad = run_guard()
    if bad:
        print("PUBLIC BOUNDARY VIOLATION: changed research/tooling material is not public-safe.")
        print(
            "Rar may contain minimum public-safe slices, synthetic fixtures and source-free "
            "receipts, not private/local corpus state or repository mirrors."
        )
        for path, violations in bad:
            print(f"  {path}:")
            for violation in violations:
                print(f"    - {violation}")
        return 1

    print("public-boundary research/tooling guard: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
