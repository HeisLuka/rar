#!/usr/bin/env python3
"""Verify that one retained editable export contains the canonical moved Node geometry.

The export artifact stays local/private. This tool emits only a compact proof and
never copies package bytes or filesystem paths into the proof.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import zipfile
from dataclasses import dataclass
from pathlib import Path
import xml.etree.ElementTree as ET

EMU_PER_POINT = 12_700
UUID_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")


@dataclass(frozen=True)
class RectEmu:
    x: int
    y: int
    width: int
    height: int

    @property
    def right(self) -> int:
        return self.x + self.width

    @property
    def bottom(self) -> int:
        return self.y + self.height


def canonical_node_hex(node_id: str) -> str:
    if not UUID_RE.fullmatch(node_id):
        raise ValueError("node_id must be canonical lowercase UUID text")
    return node_id.replace("-", "")


def format_emu_points(value: int) -> str:
    if value == 0:
        return "0"
    negative = value < 0
    numerator = abs(value)
    whole, remainder = divmod(numerator, EMU_PER_POINT)
    result = ("-" if negative else "") + str(whole)
    if remainder == 0:
        return result

    digits = []
    for _ in range(15):
        remainder *= 10
        digit, remainder = divmod(remainder, EMU_PER_POINT)
        digits.append(str(digit))
        if remainder == 0:
            break

    fraction = "".join(digits).rstrip("0")
    return result if not fraction else result + "." + fraction


def local_name(value: str) -> str:
    return value.rsplit("}", 1)[-1]


def attr_by_local(element: ET.Element, name: str) -> str | None:
    for key, value in element.attrib.items():
        if local_name(key) == name:
            return value
    return None


def expected_idml_anchors(rect: RectEmu) -> list[str]:
    x = format_emu_points(rect.x)
    y = format_emu_points(rect.y)
    right = format_emu_points(rect.right)
    bottom = format_emu_points(rect.bottom)
    return [
        f"{x} {y}",
        f"{x} {bottom}",
        f"{right} {bottom}",
        f"{right} {y}",
    ]


def verify_idml(package: zipfile.ZipFile, node_id: str, rect: RectEmu) -> dict:
    frame_self = "uf" + canonical_node_hex(node_id)
    matches = []

    for name in package.namelist():
        if not (name.startswith("Spreads/") and name.endswith(".xml")):
            continue
        root = ET.fromstring(package.read(name))
        for element in root.iter():
            if local_name(element.tag) != "TextFrame":
                continue
            if element.attrib.get("Self") != frame_self:
                continue
            anchors = [
                child.attrib["Anchor"]
                for child in element.iter()
                if local_name(child.tag) == "PathPointType" and "Anchor" in child.attrib
            ]
            matches.append((name, anchors))

    if len(matches) != 1:
        raise AssertionError(
            f"expected exactly one IDML TextFrame for {node_id}, found {len(matches)}"
        )

    part, anchors = matches[0]
    expected = expected_idml_anchors(rect)
    if anchors != expected:
        raise AssertionError(
            f"IDML geometry mismatch for {node_id}: anchors={anchors} expected={expected}"
        )

    return {
        "identity_binding": frame_self,
        "package_part": part,
        "geometry_form": "idml_path_geometry",
        "observed": {"anchors_pt": anchors},
    }


def verify_odg(package: zipfile.ZipFile, node_id: str, rect: RectEmu) -> dict:
    if "content.xml" not in package.namelist():
        raise AssertionError("ODG package has no content.xml")

    frame_name = "Frame_" + canonical_node_hex(node_id)
    root = ET.fromstring(package.read("content.xml"))
    matches = [
        element
        for element in root.iter()
        if local_name(element.tag) == "frame"
        and attr_by_local(element, "name") == frame_name
    ]
    if len(matches) != 1:
        raise AssertionError(
            f"expected exactly one ODG frame for {node_id}, found {len(matches)}"
        )

    frame = matches[0]
    observed = {
        "x": attr_by_local(frame, "x"),
        "y": attr_by_local(frame, "y"),
        "width": attr_by_local(frame, "width"),
        "height": attr_by_local(frame, "height"),
    }
    expected = {
        "x": format_emu_points(rect.x) + "pt",
        "y": format_emu_points(rect.y) + "pt",
        "width": format_emu_points(rect.width) + "pt",
        "height": format_emu_points(rect.height) + "pt",
    }
    if observed != expected:
        raise AssertionError(
            f"ODG geometry mismatch for {node_id}: observed={observed} expected={expected}"
        )

    return {
        "identity_binding": frame_name,
        "package_part": "content.xml",
        "geometry_form": "odg_frame_attributes",
        "observed": observed,
    }


def sha256_path(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_export(path: Path, target: str, node_id: str, rect: RectEmu) -> dict:
    if target not in {"idml", "odg"}:
        raise ValueError("target must be idml or odg")
    canonical_node_hex(node_id)

    with zipfile.ZipFile(path, "r") as package:
        bad_member = package.testzip()
        if bad_member is not None:
            raise AssertionError(f"export package CRC failed for member {bad_member}")
        detail = (
            verify_idml(package, node_id, rect)
            if target == "idml"
            else verify_odg(package, node_id, rect)
        )

    return {
        "receipt_kind": "chaptera.local-editable-export-geometry-proof.v1",
        "target": target,
        "artifact_sha256": sha256_path(path),
        "node_id": node_id,
        "expected_rect_emu": {
            "x": rect.x,
            "y": rect.y,
            "width": rect.width,
            "height": rect.height,
        },
        "geometry_matches_edit": True,
        **detail,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True, choices=["idml", "odg"])
    parser.add_argument("--artifact", required=True, type=Path)
    parser.add_argument("--node-id", required=True)
    parser.add_argument("--x-emu", required=True, type=int)
    parser.add_argument("--y-emu", required=True, type=int)
    parser.add_argument("--width-emu", required=True, type=int)
    parser.add_argument("--height-emu", required=True, type=int)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    rect = RectEmu(args.x_emu, args.y_emu, args.width_emu, args.height_emu)
    if rect.width <= 0 or rect.height <= 0:
        raise SystemExit("width/height must be positive")

    receipt = verify_export(args.artifact, args.target, args.node_id, rect)
    encoded = json.dumps(receipt, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
