#!/usr/bin/env python3
"""Validate retained Chaptera desktop UX screenshots for the bounded V0 shell."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import struct

PNG_SIG = b"\x89PNG\r\n\x1a\n"


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def png_size(path: pathlib.Path) -> tuple[int, int]:
    raw = path.read_bytes()
    if len(raw) < 24 or raw[:8] != PNG_SIG or raw[12:16] != b"IHDR":
        raise RuntimeError(f"{path} is not a PNG with a valid IHDR")
    width, height = struct.unpack(">II", raw[16:24])
    if width == 0 or height == 0:
        raise RuntimeError(f"{path} has zero PNG dimensions")
    return width, height


def inspect(path: pathlib.Path, expected: tuple[int, int]) -> dict[str, object]:
    if not path.is_file():
        raise RuntimeError(f"missing UX screenshot: {path}")
    if path.stat().st_size < 16_384:
        raise RuntimeError(f"UX screenshot is implausibly small: {path}")
    actual = png_size(path)
    if actual != expected:
        raise RuntimeError(
            f"{path} dimensions mismatch: expected={expected[0]}x{expected[1]} "
            f"actual={actual[0]}x{actual[1]}"
        )
    return {
        "file": path.name,
        "width": actual[0],
        "height": actual[1],
        "byte_len": path.stat().st_size,
        "sha256": sha256_file(path),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--default", required=True, type=pathlib.Path)
    parser.add_argument("--minimum", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    args = parser.parse_args()

    default = inspect(args.default, (1280, 820))
    minimum = inspect(args.minimum, (900, 600))
    if default["sha256"] == minimum["sha256"]:
        raise RuntimeError("default and minimum UX screenshots must be distinct renders")

    receipt = {
        "receipt_version": "chaptera.editor-desktop-ux-snapshots.v1",
        "renderer": "chaptera-egui-kittest-wgpu-headless-windows",
        "render_target": "headless",
        "pixels_per_point": 1.0,
        "screenshots": {
            "default_1280x820": default,
            "minimum_900x600": minimum,
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
