#!/usr/bin/env python3
"""Static production guard for Reader/Viewer active-content inertness.

This is intentionally narrow: it prevents document-open/render code from
gaining COM/OLE activation, shell/process activation, or direct network
transport without an explicit security review. It does not classify OLE
payload semantics; that remains a separate source-neutral disposition slice.
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

RUNTIME_SOURCE_ROOTS = (
    Path("apps/chaptera-desktop/src"),
    Path("vendor/producer-a/crates/pub-viewer/src"),
    Path("vendor/producer-a/crates/pub-editor/src"),
    Path("crates/chaptera-viewer-render-plan/src"),
)

RUNTIME_MANIFESTS = (
    Path("apps/chaptera-desktop/Cargo.toml"),
    Path("vendor/producer-a/crates/pub-viewer/Cargo.toml"),
    Path("vendor/producer-a/crates/pub-editor/Cargo.toml"),
    Path("crates/chaptera-viewer-render-plan/Cargo.toml"),
)

FORBIDDEN_RUNTIME_TOKENS = (
    "OleRun",
    "OleLoad",
    "OleCreate",
    "CoCreateInstance",
    "CoGetClassObject",
    "CLSIDFromProgID",
    "ProgIDFromCLSID",
    "ShellExecute",
    "WinExec",
    "CreateProcessA",
    "CreateProcessW",
    "URLDownloadToFile",
    "InternetOpen",
    "WinHttpOpen",
    "TcpStream::connect",
    "UdpSocket::bind",
    "reqwest::",
    "ureq::",
    "hyper::Client",
    "curl::",
)

NETWORK_DEPENDENCY_RE = re.compile(
    r"(?m)^\s*(reqwest|ureq|hyper|curl|webbrowser)\s*="
)
FORBIDDEN_NETWORK_PACKAGES = {"reqwest", "ureq", "hyper", "curl", "webbrowser"}

PROCESS_LAUNCH_TOKENS = ("Command::new(", "std::process::Command::new(")
ALLOWED_DESKTOP_PROCESS_CONTEXT = (
    '--product-smoke-v1',
    'activated Reader health smoke',
)


def iter_rust_sources(repo_root: Path):
    for relative_root in RUNTIME_SOURCE_ROOTS:
        root = repo_root / relative_root
        if not root.is_dir():
            raise RuntimeError(f"missing runtime source root: {relative_root}")
        yield from sorted(root.rglob("*.rs"))


def scan_forbidden_runtime_tokens(repo_root: Path) -> list[str]:
    violations: list[str] = []
    for path in iter_rust_sources(repo_root):
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(repo_root)
        for token in FORBIDDEN_RUNTIME_TOKENS:
            if token in text:
                violations.append(f"{relative}: forbidden runtime token {token!r}")
    return violations


def _production_dependency_tables(manifest: dict) -> list[tuple[str, dict]]:
    tables: list[tuple[str, dict]] = []
    dependencies = manifest.get("dependencies", {})
    if isinstance(dependencies, dict):
        tables.append(("dependencies", dependencies))

    targets = manifest.get("target", {})
    if isinstance(targets, dict):
        for target_name, target in targets.items():
            if not isinstance(target, dict):
                continue
            target_dependencies = target.get("dependencies", {})
            if isinstance(target_dependencies, dict):
                tables.append((f"target.{target_name}.dependencies", target_dependencies))
    return tables


def _declared_package_name(dependency_name: str, specification: object) -> str:
    if isinstance(specification, dict):
        package = specification.get("package")
        if isinstance(package, str) and package:
            return package
    return dependency_name


def scan_network_dependencies(repo_root: Path) -> list[str]:
    violations: list[str] = []
    for relative in RUNTIME_MANIFESTS:
        path = repo_root / relative
        if not path.is_file():
            raise RuntimeError(f"missing runtime manifest: {relative}")
        try:
            manifest = tomllib.loads(path.read_text(encoding="utf-8"))
        except (OSError, tomllib.TOMLDecodeError) as error:
            raise RuntimeError(f"cannot parse runtime manifest {relative}: {error}") from error

        for table_name, dependencies in _production_dependency_tables(manifest):
            for dependency_name, specification in dependencies.items():
                package_name = _declared_package_name(dependency_name, specification)
                if package_name in FORBIDDEN_NETWORK_PACKAGES:
                    violations.append(
                        f"{relative} [{table_name}]: direct network dependency "
                        f"{dependency_name!r} resolves to forbidden package {package_name!r}"
                    )
    return violations


def scan_desktop_process_launch(repo_root: Path) -> list[str]:
    desktop_root = repo_root / "apps/chaptera-desktop/src"
    occurrences: list[tuple[Path, int, str]] = []
    for path in sorted(desktop_root.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for token in PROCESS_LAUNCH_TOKENS:
            start = 0
            while True:
                index = text.find(token, start)
                if index < 0:
                    break
                occurrences.append((path, index, token))
                start = index + len(token)

    if len(occurrences) != 1:
        rendered = ", ".join(
            f"{path.relative_to(repo_root)}:{token}" for path, _, token in occurrences
        ) or "none"
        return [
            "desktop production process-launch surface changed: expected exactly one "
            f"reviewed updater health-smoke launch, found {len(occurrences)} ({rendered})"
        ]

    path, index, token = occurrences[0]
    relative = path.relative_to(repo_root)
    if relative != Path("apps/chaptera-desktop/src/main.rs"):
        return [
            f"{relative}: reviewed process launch moved outside the admitted updater "
            "health-smoke location"
        ]

    text = path.read_text(encoding="utf-8")
    window = text[max(0, index - 2500) : index + 3500]
    missing = [marker for marker in ALLOWED_DESKTOP_PROCESS_CONTEXT if marker not in window]
    if missing:
        return [
            f"{relative}: the sole {token} no longer proves the fixed Chaptera "
            f"product-smoke context; missing markers: {missing}"
        ]
    return []


def scan(repo_root: Path) -> list[str]:
    violations: list[str] = []
    violations.extend(scan_forbidden_runtime_tokens(repo_root))
    violations.extend(scan_network_dependencies(repo_root))
    violations.extend(scan_desktop_process_launch(repo_root))
    return violations


def self_test() -> None:
    for token in ("CoCreateInstance", "ShellExecute", "reqwest::"):
        assert any(token == candidate for candidate in FORBIDDEN_RUNTIME_TOKENS)
    assert NETWORK_DEPENDENCY_RE.search('reqwest = "0.12"')
    assert NETWORK_DEPENDENCY_RE.search('hyper = { version = "1" }')
    assert not NETWORK_DEPENDENCY_RE.search('description = "hyperlink support"')
    assert "hyperlink" not in FORBIDDEN_RUNTIME_TOKENS

    alias_manifest = tomllib.loads(
        """
        [dependencies]
        harmless-name = { package = "reqwest", version = "0.12" }

        [target.'cfg(windows)'.dependencies]
        inherited-http = { package = "hyper", workspace = true }

        [dev-dependencies]
        reqwest = "0.12"
        """
    )
    production = _production_dependency_tables(alias_manifest)
    found = {
        _declared_package_name(name, spec)
        for _, table in production
        for name, spec in table.items()
    }
    assert found == {"reqwest", "hyper"}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        print("reader active-content guard self-test: ok")
        return 0

    try:
        violations = scan(REPO_ROOT)
    except RuntimeError as error:
        print(f"reader active-content guard configuration error: {error}", file=sys.stderr)
        return 2

    if violations:
        print("Reader active-content inertness guard failed:", file=sys.stderr)
        for violation in violations:
            print(f"- {violation}", file=sys.stderr)
        return 1

    print(
        "Reader active-content inertness guard: no COM/OLE/shell/network activation "
        "surface found; one fixed Chaptera updater health-smoke process launch admitted."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
