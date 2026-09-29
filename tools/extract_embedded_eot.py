#!/usr/bin/env python3
"""Extract one exact embedded EOT record from a pinned Publisher PUB fixture."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path

import olefile


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def u16(data: bytes, offset: int) -> int:
    return struct.unpack_from("<H", data, offset)[0]


def u32(data: bytes, offset: int) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def find_eot_candidates(contents: bytes) -> list[dict[str, int]]:
    candidates: list[dict[str, int]] = []
    cursor = 0
    while True:
        record_offset = contents.find(b"\x0c\x80", cursor)
        if record_offset < 0:
            break
        cursor = record_offset + 1
        if record_offset + 42 > len(contents):
            continue

        block_data_length = u32(contents, record_offset + 2)
        eot_offset = record_offset + 6
        eot_size = u32(contents, eot_offset)
        font_data_size = u32(contents, eot_offset + 4)
        version = u32(contents, eot_offset + 8)
        flags = u32(contents, eot_offset + 12)
        charset = contents[eot_offset + 26]
        weight = u32(contents, eot_offset + 28)
        fs_type = u16(contents, eot_offset + 32)
        magic = u16(contents, eot_offset + 34)

        if magic != 0x504C:
            continue
        if block_data_length != eot_size + 4:
            continue
        if eot_size < 36 or eot_offset + eot_size > len(contents):
            continue
        if font_data_size > eot_size:
            continue

        candidates.append(
            {
                "record_offset": record_offset,
                "block_data_length": block_data_length,
                "eot_offset": eot_offset,
                "eot_size": eot_size,
                "font_data_size": font_data_size,
                "version": version,
                "flags": flags,
                "charset": charset,
                "weight": weight,
                "fs_type": fs_type,
                "magic": magic,
            }
        )
    return candidates


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pub", required=True, type=Path)
    parser.add_argument("--out-eot", required=True, type=Path)
    parser.add_argument("--receipt", required=True, type=Path)
    parser.add_argument("--expected-pub-sha256", required=True)
    parser.add_argument("--expected-eot-sha256", required=True)
    parser.add_argument("--expected-font-name", required=True)
    args = parser.parse_args()

    pub_bytes = args.pub.read_bytes()
    pub_sha256 = sha256_hex(pub_bytes)
    if pub_sha256 != args.expected_pub_sha256.lower():
        raise SystemExit(
            f"PUB SHA-256 mismatch: expected {args.expected_pub_sha256.lower()} got {pub_sha256}"
        )

    with olefile.OleFileIO(str(args.pub)) as compound:
        if not compound.exists("Contents"):
            raise SystemExit("pinned PUB has no Contents stream")
        contents = compound.openstream("Contents").read()

    candidates = find_eot_candidates(contents)
    if len(candidates) != 1:
        raise SystemExit(
            f"expected exactly one structurally valid EOT record, found {len(candidates)}"
        )

    candidate = candidates[0]
    eot_offset = candidate["eot_offset"]
    eot_size = candidate["eot_size"]
    eot = contents[eot_offset : eot_offset + eot_size]
    eot_sha256 = sha256_hex(eot)
    if eot_sha256 != args.expected_eot_sha256.lower():
        raise SystemExit(
            f"EOT SHA-256 mismatch: expected {args.expected_eot_sha256.lower()} got {eot_sha256}"
        )

    stored_name_bytes = args.expected_font_name.encode("utf-16le") + b"\x00\x00"
    search_start = max(0, candidate["record_offset"] - 4096)
    stored_name_offset = contents.rfind(
        stored_name_bytes, search_start, candidate["record_offset"]
    )
    if stored_name_offset < 0:
        raise SystemExit(
            f"stored UTF-16LE font name {args.expected_font_name!r} not found before EOT record"
        )

    args.out_eot.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.out_eot.write_bytes(eot)

    receipt = {
        "schema": "chaptera.embedded-font-eot-extraction.v1",
        "source": {
            "filename": args.pub.name,
            "sha256": pub_sha256,
            "byte_len": len(pub_bytes),
            "contents_byte_len": len(contents),
        },
        "font": {
            "stored_name": args.expected_font_name,
            "stored_name_offset_in_contents": stored_name_offset,
            "carrier_record_offset_in_contents": candidate["record_offset"],
            "eot_offset_in_contents": eot_offset,
            "eot_size": eot_size,
            "eot_sha256": eot_sha256,
            "font_data_size": candidate["font_data_size"],
            "version": f"0x{candidate['version']:08x}",
            "flags": f"0x{candidate['flags']:08x}",
            "charset": candidate["charset"],
            "weight": candidate["weight"],
            "fs_type": f"0x{candidate['fs_type']:04x}",
            "magic": f"0x{candidate['magic']:04x}",
        },
        "claims": {
            "exact_embedded_eot_bytes": True,
            "decoded_font_bytes_yet": False,
            "publisher_exact_layout_yet": False,
        },
    }
    args.receipt.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
