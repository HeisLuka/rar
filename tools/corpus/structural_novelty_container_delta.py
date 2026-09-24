#!/usr/bin/env python3
"""Structural novelty replay for the retained 407-SHA container delta.

The replay is deliberately independent of expiring Actions artifacts.  The
target SHA set is loaded from the five permanent receipts merged by rar#177.
Six public root containers are pinned by URL, exact SHA-256, and byte length.
Each root is downloaded once, statically inventoried with the existing 7z
primitive, and only .pub members whose exact SHA belongs to the retained
407-SHA set are probed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tempfile
import time
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import pub_container_extract as containers  # type: ignore
import structural_novelty as novelty  # type: ignore

SCHEMA = "chaptera.container-delta-structural-novelty.v2"
EXPECTED_SHA = 407
EXPECTED_COVER_ROOTS = 6
EXPECTED_COVER_BYTES = 762_499_072
HEX64 = re.compile(r"^[0-9a-f]{64}$")

SHA_RECEIPTS = [
    HERE / "receipts" / f"container-net-new-407-part-{i:02d}.sha256.txt"
    for i in range(1, 6)
]

ROOTS = [
    {
        "url": "https://archive.org/download/microsoft-home-collection-1993-1995-/Productivity/Microsoft%20Publisher%203.0%20for%20Windows%2095.iso",
        "sha256": "36abfed421c586737cf852f4c3179210cd8f93e7d2d8c56e07e2ef825b5d7a7a",
        "size_bytes": 47_730_688,
        "expected_target_sha": 128,
    },
    {
        "url": "https://archive.org/download/microsoft-publisher-2.0_202011/microsoft-publisher-2.0.iso",
        "sha256": "8f1e46dd43be728d7b39f53cf4f642149b19eebe0c74103270fc14ecae1291c4",
        "size_bytes": 23_851_008,
        "expected_target_sha": 20,
    },
    {
        "url": "https://archive.org/download/microsoft-publisher-97-cd-deluxe/microsoft-publisher-97-cd-deluxe.iso",
        "sha256": "69043e0d2d23345163a27806914c99382490a27e5710fdb6af82ee7df33b4ce8",
        "size_bytes": 138_719_232,
        "expected_target_sha": 13,
    },
    {
        "url": "https://archive.org/download/pub-40-cd/PUB_40_CD.iso",
        "sha256": "3abd372fd3a1def03cf17ca6277533a8a1bcfbe88b56a213f9c389035020a78b",
        "size_bytes": 138_719_232,
        "expected_target_sha": 16,
    },
    {
        "url": "https://archive.org/download/pub97no/PUB_40_CD.ISO",
        "sha256": "c44fc471b42e3381efeec1d069161c05331d1ebd883d764e9c04a5c6392177c7",
        "size_bytes": 142_080_000,
        "expected_target_sha": 238,
    },
    {
        "url": "https://archive.org/download/video-professor-learn-publisher/Learn_Publisher_Disc_3.iso",
        "sha256": "e6e86f2fd5d0430437c5fdb06db5958776475c84ec80b5085d45fe96d3a5d5cd",
        "size_bytes": 271_398_912,
        "expected_target_sha": 4,
    },
]


def load_target_shas() -> set[str]:
    out: set[str] = set()
    for path in SHA_RECEIPTS:
        if not path.is_file():
            raise ValueError(f"missing retained SHA receipt: {path}")
        for line in path.read_text(encoding="utf-8").splitlines():
            sha = line.strip().lower()
            if not sha:
                continue
            if not HEX64.fullmatch(sha):
                raise ValueError(f"malformed SHA in {path.name}: {sha!r}")
            if sha in out:
                raise ValueError(f"duplicate retained SHA across receipt parts: {sha}")
            out.add(sha)
    if len(out) != EXPECTED_SHA:
        raise ValueError(f"expected {EXPECTED_SHA} retained SHA, got {len(out)}")
    return out


def validate_roots() -> None:
    if len(ROOTS) != EXPECTED_COVER_ROOTS:
        raise ValueError(f"expected {EXPECTED_COVER_ROOTS} pinned roots")
    if sum(int(root["size_bytes"]) for root in ROOTS) != EXPECTED_COVER_BYTES:
        raise ValueError("pinned root byte total drift")
    seen = set()
    for root in ROOTS:
        if root["url"] in seen:
            raise ValueError("duplicate pinned root URL")
        seen.add(root["url"])
        if not HEX64.fullmatch(str(root["sha256"])):
            raise ValueError("malformed pinned root SHA")
        if int(root["size_bytes"]) <= 0 or int(root["expected_target_sha"]) <= 0:
            raise ValueError("invalid pinned root metadata")


def fetch_root(url: str, path: Path, timeout: float, max_bytes: int, attempts: int = 3) -> dict:
    last = None
    for attempt in range(attempts):
        try:
            return containers.fetch(url, path, timeout, max_bytes)
        except Exception as exc:
            last = exc
            path.unlink(missing_ok=True)
            if attempt + 1 < attempts:
                time.sleep(2 ** attempt)
    assert last is not None
    raise last


def candidate_members(archive: Path, command_timeout: int, max_member_bytes: int) -> list[tuple[str, int]]:
    out: list[tuple[str, int]] = []
    for entry in containers.list_7z(archive, command_timeout):
        name = str(entry.get("Path") or "")
        if containers.ext_kind(name) != "pub" or not containers.safe_member(name):
            continue
        if entry.get("Encrypted", "-") == "+":
            continue
        if "L" in str(entry.get("Attributes") or ""):
            continue
        try:
            size = int(entry.get("Size", "0") or 0)
        except ValueError:
            continue
        if size < 0 or size > max_member_bytes:
            continue
        out.append((name, size))
    out.sort()
    return out


def extract_member_bytes(
    archive: Path,
    member: str,
    expected_size: int,
    td: Path,
    command_timeout: int,
    max_member_bytes: int,
) -> bytes:
    token = hashlib.sha256(member.encode("utf-8")).hexdigest()[:16]
    path = td / f"member-{token}.pub"
    try:
        actual_size = containers.extract_member(
            archive, member, path, command_timeout, max_member_bytes
        )
        data = path.read_bytes()
    finally:
        path.unlink(missing_ok=True)
    if actual_size != expected_size:
        raise ValueError(
            f"member size mismatch expected={expected_size} actual={actual_size}"
        )
    return data


def scan_root(args: argparse.Namespace) -> int:
    target = load_target_shas()
    validate_roots()
    if not (0 <= args.root_index < len(ROOTS)):
        raise ValueError("root-index outside pinned root set")
    root = ROOTS[args.root_index]
    args.out.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="rar-delta407-") as raw:
        td = Path(raw)
        archive = td / "root-container.bin"
        meta = fetch_root(
            str(root["url"]), archive, args.timeout, args.max_container_bytes
        )
        if meta["sha256"] != root["sha256"]:
            raise ValueError(
                f"root SHA mismatch expected={root['sha256']} actual={meta['sha256']}"
            )
        if int(meta["size"]) != int(root["size_bytes"]):
            raise ValueError(
                f"root size mismatch expected={root['size_bytes']} actual={meta['size']}"
            )

        matched: dict[str, dict] = {}
        inventoried = candidate_members(
            archive, args.command_timeout, args.max_member_bytes
        )
        for member, declared_size in inventoried:
            data = extract_member_bytes(
                archive,
                member,
                declared_size,
                td,
                args.command_timeout,
                args.max_member_bytes,
            )
            sha = hashlib.sha256(data).hexdigest()
            if sha not in target:
                continue
            first = novelty.cfb_probe(data)
            second = novelty.cfb_probe(data)
            if first != second:
                raise RuntimeError(f"probe_nondeterministic:{sha}")
            row = {
                "sha256": sha,
                "sources": ["container_net_new_407"],
                "filenames": [Path(member).name],
                "container_url": root["url"],
                "root_sha256": root["sha256"],
                "archive_member": member,
                **first,
                "status": "ok",
                "rehydrated_from": "container_delta_407",
            }
            prior = matched.get(sha)
            if prior is not None:
                keys = (
                    "path_fingerprint_sha256",
                    "topology_fingerprint_sha256",
                    "size_bucket_fingerprint_sha256",
                    "content_topology_fingerprint_sha256",
                )
                if any(prior.get(k) != row.get(k) for k in keys):
                    raise ValueError(f"same-SHA fingerprint disagreement inside root: {sha}")
            matched.setdefault(sha, row)

        if len(matched) != int(root["expected_target_sha"]):
            raise ValueError(
                f"root target-set drift: expected {root['expected_target_sha']} "
                f"retained SHA, found {len(matched)}"
            )

        rows = [matched[k] for k in sorted(matched)]
        (args.out / "fingerprints.json").write_text(
            json.dumps(rows, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        summary = {
            "schema": SCHEMA,
            "root_index": args.root_index,
            "root_url": root["url"],
            "root_sha256": root["sha256"],
            "root_size_bytes": root["size_bytes"],
            "candidate_pub_members": len(inventoried),
            "matched_retained_sha": len(matched),
            "status": {"ok": len(matched)},
        }
        (args.out / "summary.json").write_text(
            json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        print(json.dumps(summary, indent=2))
    return 0


def aggregate(args: argparse.Namespace) -> int:
    target = load_target_shas()
    validate_roots()
    observations = []
    for path in args.input.rglob("fingerprints.json"):
        payload = json.loads(path.read_text(encoding="utf-8"))
        if isinstance(payload, list):
            observations.extend(payload)

    by_sha: dict[str, dict] = {}
    for row in observations:
        sha = str(row.get("sha256") or "")
        if sha not in target:
            raise ValueError(f"replay emitted non-target SHA: {sha}")
        prior = by_sha.get(sha)
        if prior is not None:
            keys = (
                "path_fingerprint_sha256",
                "topology_fingerprint_sha256",
                "size_bucket_fingerprint_sha256",
                "content_topology_fingerprint_sha256",
            )
            if any(prior.get(k) != row.get(k) for k in keys):
                raise ValueError(f"cross-root fingerprint mismatch for {sha}")
        by_sha.setdefault(sha, row)

    missing = sorted(target - set(by_sha))
    if missing:
        raise ValueError(f"selected six-root replay missed {len(missing)} retained SHA")
    if len(by_sha) != EXPECTED_SHA:
        raise ValueError(f"expected {EXPECTED_SHA} unique replay rows, got {len(by_sha)}")

    canonical = [by_sha[k] for k in sorted(by_sha)]
    temp = args.out / "_source"
    temp.mkdir(parents=True, exist_ok=True)
    (temp / "fingerprints.json").write_text(
        json.dumps(canonical, indent=2, ensure_ascii=False), encoding="utf-8"
    )
    novelty.aggregate(argparse.Namespace(input=temp, out=args.out))
    (temp / "fingerprints.json").unlink()
    temp.rmdir()

    summary_path = args.out / "summary.json"
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    summary.update(
        {
            "schema": SCHEMA,
            "cohort": "container_net_new_407",
            "expected_sha": EXPECTED_SHA,
            "selected_cover_root_count": EXPECTED_COVER_ROOTS,
            "selected_cover_total_bytes": EXPECTED_COVER_BYTES,
            "observation_rows": len(observations),
        }
    )
    summary_path.write_text(
        json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8"
    )
    print(json.dumps(summary, indent=2))
    return 0


def self_test() -> int:
    target = load_target_shas()
    validate_roots()
    assert len(target) == 407
    assert sum(int(root["size_bytes"]) for root in ROOTS) == 762_499_072
    assert sum(int(root["expected_target_sha"]) for root in ROOTS) >= 407
    print("container delta 407 retained-input self-test ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("scan-root")
    s.add_argument("--root-index", type=int, required=True)
    s.add_argument("--out", type=Path, required=True)
    s.add_argument("--timeout", type=float, default=60.0)
    s.add_argument("--command-timeout", type=int, default=45)
    s.add_argument("--max-container-bytes", type=int, default=320 * 1024 * 1024)
    s.add_argument("--max-member-bytes", type=int, default=100 * 1024 * 1024)

    a = sub.add_parser("aggregate")
    a.add_argument("--input", type=Path, required=True)
    a.add_argument("--out", type=Path, required=True)

    sub.add_parser("self-test")

    args = ap.parse_args()
    if args.cmd == "scan-root":
        return scan_root(args)
    if args.cmd == "aggregate":
        return aggregate(args)
    return self_test()


if __name__ == "__main__":
    raise SystemExit(main())
