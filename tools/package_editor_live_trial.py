#!/usr/bin/env python3
"""Stage one bounded Chaptera Editor portable ZIP from an existing Windows binary.

This tool owns package mechanics only. It does not build Chaptera, exercise the
editor, or decide whether a local/private binary is product-authoritative.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import stat
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_README = ROOT / "packages" / "product" / "editor-live-trial" / "v1" / "TRIAL-README.md"
DEFAULT_AGENT_CATALOG = ROOT / "packages" / "protocol" / "editor-agent-control" / "v1.catalog.json"
README_CONTRACT = "chaptera.editor-live-trial-readme.v1"
AGENT_CATALOG_SCHEMA = "chaptera.agent-control.catalog.v1"
AGENT_PROTOCOL_VERSION = "chaptera.agent-control.v1"
AGENT_EXECUTABLE = "chaptera-editor.exe"
FIXED_TIME = (1980, 1, 1, 0, 0, 0)


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _zip_entry(name: str, *, executable: bool = False) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(name, FIXED_TIME)
    info.compress_type = zipfile.ZIP_DEFLATED
    mode = 0o755 if executable else 0o644
    info.external_attr = (stat.S_IFREG | mode) << 16
    return info


def package_editor(
    editor_exe: pathlib.Path,
    output_zip: pathlib.Path,
    *,
    binary_entry: str = "Chaptera-Editor.exe",
    readme: pathlib.Path = DEFAULT_README,
    readme_entry: str = "TRIAL-README.md",
    agent_catalog: pathlib.Path = DEFAULT_AGENT_CATALOG,
    agent_catalog_entry: str = "agent-control-v1.catalog.json",
) -> dict[str, str | int]:
    if not editor_exe.is_file():
        raise RuntimeError(f"Editor executable does not exist: {editor_exe}")
    if editor_exe.suffix.lower() != ".exe":
        raise RuntimeError("Editor binary must be a Windows .exe")

    binary = editor_exe.read_bytes()
    if len(binary) < 2 or binary[:2] != b"MZ":
        raise RuntimeError("Editor binary does not look like a PE executable (missing MZ header)")

    if not readme.is_file():
        raise RuntimeError(f"trial README does not exist: {readme}")
    readme_bytes = readme.read_bytes()
    try:
        readme_text = readme_bytes.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise RuntimeError("trial README must be UTF-8") from exc
    if README_CONTRACT not in readme_text:
        raise RuntimeError("trial README contract marker is missing")

    if not agent_catalog.is_file():
        raise RuntimeError(f"Agent V1 catalog does not exist: {agent_catalog}")
    agent_catalog_bytes = agent_catalog.read_bytes()
    try:
        agent_catalog_value = json.loads(agent_catalog_bytes.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise RuntimeError("Agent V1 catalog must be valid UTF-8 JSON") from exc
    if not isinstance(agent_catalog_value, dict):
        raise RuntimeError("Agent V1 catalog must be a JSON object")
    if agent_catalog_value.get("schema") != AGENT_CATALOG_SCHEMA:
        raise RuntimeError("Agent V1 catalog schema mismatch")
    if agent_catalog_value.get("protocol_version") != AGENT_PROTOCOL_VERSION:
        raise RuntimeError("Agent V1 catalog protocol version mismatch")
    if agent_catalog_value.get("executable") != AGENT_EXECUTABLE:
        raise RuntimeError("Agent V1 catalog executable identity mismatch")
    laws = agent_catalog_value.get("global_laws")
    if not isinstance(laws, dict):
        raise RuntimeError("Agent V1 catalog global_laws missing")
    if laws.get("native_pub_write") is not False:
        raise RuntimeError("Agent V1 catalog must not claim native PUB write")
    if laws.get("source_pub_immutable") is not True:
        raise RuntimeError("Agent V1 catalog must require immutable source PUB")

    for label, value in (
        ("binary_entry", binary_entry),
        ("readme_entry", readme_entry),
        ("agent_catalog_entry", agent_catalog_entry),
    ):
        pure = pathlib.PurePosixPath(value.replace("\\", "/"))
        if pure.is_absolute() or ".." in pure.parts or len(pure.parts) != 1:
            raise RuntimeError(f"{label} must be a single safe ZIP entry name")
    if not binary_entry.lower().endswith(".exe"):
        raise RuntimeError("binary_entry must end with .exe")
    if any(
        entry.lower().endswith(".pub")
        for entry in (binary_entry, readme_entry, agent_catalog_entry)
    ):
        raise RuntimeError("portable package must not contain a PUB entry")
    if len({binary_entry.casefold(), readme_entry.casefold(), agent_catalog_entry.casefold()}) != 3:
        raise RuntimeError("portable package entry names must be distinct")

    output_zip.parent.mkdir(parents=True, exist_ok=True)
    if output_zip.exists():
        output_zip.unlink()

    with zipfile.ZipFile(output_zip, "w") as archive:
        archive.writestr(_zip_entry(binary_entry, executable=True), binary)
        archive.writestr(_zip_entry(readme_entry), readme_bytes)
        archive.writestr(_zip_entry(agent_catalog_entry), agent_catalog_bytes)

    binary_sha = hashlib.sha256(binary).hexdigest()
    agent_catalog_sha = hashlib.sha256(agent_catalog_bytes).hexdigest()
    zip_sha = sha256_file(output_zip)
    if binary_sha == zip_sha:
        raise RuntimeError("binary and ZIP identities must be distinct")

    return {
        "zip": str(output_zip),
        "binary_entry": binary_entry,
        "readme_entry": readme_entry,
        "agent_catalog_entry": agent_catalog_entry,
        "agent_catalog_sha256": agent_catalog_sha,
        "agent_catalog_size": len(agent_catalog_bytes),
        "binary_sha256": binary_sha,
        "zip_sha256": zip_sha,
        "binary_size": len(binary),
        "zip_size": output_zip.stat().st_size,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--editor-exe", required=True, type=pathlib.Path)
    parser.add_argument("--output-zip", required=True, type=pathlib.Path)
    parser.add_argument("--binary-entry", default="Chaptera-Editor.exe")
    parser.add_argument("--readme", type=pathlib.Path, default=DEFAULT_README)
    parser.add_argument("--readme-entry", default="TRIAL-README.md")
    parser.add_argument("--agent-catalog", type=pathlib.Path, default=DEFAULT_AGENT_CATALOG)
    parser.add_argument("--agent-catalog-entry", default="agent-control-v1.catalog.json")
    parser.add_argument("--manifest", type=pathlib.Path)
    args = parser.parse_args()

    result = package_editor(
        args.editor_exe,
        args.output_zip,
        binary_entry=args.binary_entry,
        readme=args.readme,
        readme_entry=args.readme_entry,
        agent_catalog=args.agent_catalog,
        agent_catalog_entry=args.agent_catalog_entry,
    )
    if args.manifest is not None:
        args.manifest.parent.mkdir(parents=True, exist_ok=True)
        args.manifest.write_text(
            json.dumps(result, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
