#!/usr/bin/env python3
"""Create one deterministic portable Chaptera Reader ZIP."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import stat
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_README = ROOT / "packages" / "product" / "reader-portable" / "v1" / "README.md"
README_CONTRACT = "chaptera.reader-portable-readme.v1"
FIXED_TIME = (1980, 1, 1, 0, 0, 0)


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def zip_entry(name: str, *, executable: bool = False) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(name, FIXED_TIME)
    info.compress_type = zipfile.ZIP_DEFLATED
    mode = 0o755 if executable else 0o644
    info.external_attr = (stat.S_IFREG | mode) << 16
    return info


def package_reader(
    reader_exe: pathlib.Path,
    output_zip: pathlib.Path,
    *,
    binary_entry: str = "Chaptera-Reader.exe",
    readme: pathlib.Path = DEFAULT_README,
    readme_entry: str = "README.md",
) -> dict[str, str | int]:
    if not reader_exe.is_file():
        raise RuntimeError(f"Reader executable does not exist: {reader_exe}")
    binary = reader_exe.read_bytes()
    if len(binary) < 2 or binary[:2] != b"MZ":
        raise RuntimeError("Reader binary does not look like a Windows PE executable")
    if not readme.is_file():
        raise RuntimeError(f"Reader README does not exist: {readme}")
    readme_bytes = readme.read_bytes()
    try:
        readme_text = readme_bytes.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise RuntimeError("Reader README must be UTF-8") from exc
    if README_CONTRACT not in readme_text:
        raise RuntimeError("Reader README contract marker is missing")

    for label, value in (("binary_entry", binary_entry), ("readme_entry", readme_entry)):
        pure = pathlib.PurePosixPath(value.replace("\\", "/"))
        if pure.is_absolute() or ".." in pure.parts or len(pure.parts) != 1:
            raise RuntimeError(f"{label} must be one safe ZIP entry")
    if binary_entry != "Chaptera-Reader.exe":
        raise RuntimeError("Reader package entry must be Chaptera-Reader.exe")

    output_zip.parent.mkdir(parents=True, exist_ok=True)
    if output_zip.exists():
        output_zip.unlink()
    with zipfile.ZipFile(output_zip, "w") as archive:
        archive.writestr(zip_entry(binary_entry, executable=True), binary)
        archive.writestr(zip_entry(readme_entry), readme_bytes)

    result = {
        "schema_version": "chaptera.reader-portable-package.v1",
        "product_id": "chaptera.reader",
        "zip": str(output_zip),
        "binary_entry": binary_entry,
        "readme_entry": readme_entry,
        "binary_sha256": hashlib.sha256(binary).hexdigest(),
        "zip_sha256": sha256_file(output_zip),
        "binary_size": len(binary),
        "zip_size": output_zip.stat().st_size,
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reader-exe", required=True, type=pathlib.Path)
    parser.add_argument("--output-zip", required=True, type=pathlib.Path)
    parser.add_argument("--manifest", required=True, type=pathlib.Path)
    parser.add_argument("--readme", type=pathlib.Path, default=DEFAULT_README)
    args = parser.parse_args()

    result = package_reader(
        args.reader_exe,
        args.output_zip,
        readme=args.readme,
    )
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
