#!/usr/bin/env python3
"""Source-free structural novelty replay for the admitted 407-SHA container delta."""
from __future__ import annotations

import argparse
import hashlib
import itertools
import json
import os
import sys
import tempfile
import time
import zipfile
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import pub_container_extract as containers  # type: ignore
import structural_novelty as novelty  # type: ignore

SCHEMA = "chaptera.container-delta-structural-novelty.v1"
EXPECTED_SHA = 407
EXPECTED_ROOTS = 12
EXPECTED_COVER_ROOTS = 6
EXPECTED_COVER_BYTES = 762_499_072


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def download_receipt(artifact_id: int, artifact_sha256: str, work: Path) -> list[dict]:
    archive = work / "receipt.zip"
    novelty.download_artifact("HeisLuka/rar", artifact_id, os.environ.get("GITHUB_TOKEN", ""), archive)
    actual = sha256_file(archive)
    if actual != artifact_sha256:
        raise ValueError(f"receipt artifact digest mismatch expected={artifact_sha256} actual={actual}")
    dest = work / "receipt"
    dest.mkdir()
    novelty.safe_extract_zip(archive, dest)
    rows = json.loads((dest / "container-net-new.json").read_text(encoding="utf-8"))
    if not isinstance(rows, list):
        raise ValueError("container-net-new.json must contain a list")
    if len(rows) != EXPECTED_SHA or len({r.get("sha256") for r in rows}) != EXPECTED_SHA:
        raise ValueError(f"expected {EXPECTED_SHA} exact SHA rows")
    return rows


def observation_key(p: dict) -> tuple:
    return (
        str(p.get("container_url") or ""),
        str(p.get("root_sha256") or ""),
        str(p.get("archive_member") or ""),
    )


def root_inventory(rows: list[dict]) -> tuple[set[str], dict[tuple, set[str]], dict[tuple, list[tuple[str, dict]]]]:
    all_sha: set[str] = set()
    root_sets: dict[tuple, set[str]] = defaultdict(set)
    root_obs: dict[tuple, list[tuple[str, dict]]] = defaultdict(list)
    for row in rows:
        sha = str(row.get("sha256") or "").lower()
        if len(sha) != 64:
            raise ValueError("malformed delta SHA")
        all_sha.add(sha)
        provenance = row.get("provenance")
        if not isinstance(provenance, list) or not provenance:
            raise ValueError(f"missing provenance for {sha}")
        for p in provenance:
            url = str(p.get("container_url") or "").strip()
            root_sha = str(p.get("root_sha256") or "").lower()
            size = int(p.get("root_size_bytes") or 0)
            depth = int(p.get("container_depth") or 0)
            member = str(p.get("archive_member") or "").strip()
            if not url or len(root_sha) != 64 or size <= 0 or not member:
                raise ValueError(f"incomplete root locator for {sha}")
            if depth != 0:
                raise ValueError(f"delta replay currently requires depth=0; got {depth} for {sha}")
            key = (url, root_sha, size)
            root_sets[key].add(sha)
            root_obs[key].append((sha, p))
    if len(all_sha) != EXPECTED_SHA:
        raise ValueError(f"expected {EXPECTED_SHA} delta SHA, got {len(all_sha)}")
    if len(root_sets) != EXPECTED_ROOTS:
        raise ValueError(f"expected {EXPECTED_ROOTS} exact roots, got {len(root_sets)}")
    return all_sha, dict(root_sets), dict(root_obs)


def minimum_cover(universe: set[str], root_sets: dict[tuple, set[str]]) -> list[tuple]:
    roots = sorted(root_sets)
    best = None
    for mask in range(1, 1 << len(roots)):
        selected = [roots[i] for i in range(len(roots)) if mask & (1 << i)]
        byte_cost = sum(int(key[2]) for key in selected)
        if best is not None and byte_cost > best[0]:
            continue
        covered = set().union(*(root_sets[key] for key in selected))
        if covered != universe:
            continue
        candidate = (byte_cost, len(selected), selected)
        if best is None or candidate[:2] < best[:2]:
            best = candidate
    if best is None:
        raise ValueError("no exact root cover exists")
    byte_cost, count, selected = best
    if count != EXPECTED_COVER_ROOTS or byte_cost != EXPECTED_COVER_BYTES:
        raise ValueError(
            f"cover drift: roots={count} bytes={byte_cost}; "
            f"expected roots={EXPECTED_COVER_ROOTS} bytes={EXPECTED_COVER_BYTES}"
        )
    return selected


def assign_sha(universe: set[str], selected: list[tuple], root_sets: dict[tuple, set[str]]) -> dict[tuple, set[str]]:
    assigned = {key: set() for key in selected}
    for sha in sorted(universe):
        candidates = [key for key in selected if sha in root_sets[key]]
        if not candidates:
            raise ValueError(f"selected cover misses {sha}")
        key = min(candidates, key=lambda x: (int(x[2]), str(x[0]), str(x[1])))
        assigned[key].add(sha)
    if set().union(*assigned.values()) != universe:
        raise ValueError("assignment does not cover universe")
    return assigned


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


def extract_exact(archive: Path, member: str, expected_sha: str, expected_size: int, td: Path, command_timeout: int, max_member_bytes: int) -> bytes:
    out = td / ("member-" + hashlib.sha256((expected_sha + member).encode()).hexdigest()[:16] + ".pub")
    try:
        actual_size = containers.extract_member(archive, member, out, command_timeout, max_member_bytes)
        data = out.read_bytes()
    finally:
        out.unlink(missing_ok=True)
    if expected_size and actual_size != expected_size:
        raise ValueError(f"member size mismatch expected={expected_size} actual={actual_size}")
    actual_sha = hashlib.sha256(data).hexdigest()
    if actual_sha != expected_sha:
        raise ValueError(f"member SHA mismatch expected={expected_sha} actual={actual_sha}")
    return data


def pick_observation(sha: str, key: tuple, root_obs: dict[tuple, list[tuple[str, dict]]]) -> dict:
    matches = [p for row_sha, p in root_obs[key] if row_sha == sha]
    if not matches:
        raise ValueError(f"no observation for {sha} in selected root")
    return sorted(matches, key=observation_key)[0]


def scan_root(args: argparse.Namespace) -> int:
    args.out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rar-delta407-") as raw:
        td = Path(raw)
        rows = download_receipt(args.receipt_artifact_id, args.receipt_artifact_sha256, td)
        universe, root_sets, root_obs = root_inventory(rows)
        selected = minimum_cover(universe, root_sets)
        assigned = assign_sha(universe, selected, root_sets)
        if not (0 <= args.root_index < len(selected)):
            raise ValueError("root-index outside selected cover")
        key = selected[args.root_index]
        url, expected_root_sha, expected_root_size = key
        archive = td / "root-container.bin"
        meta = fetch_root(url, archive, args.timeout, args.max_container_bytes)
        if meta["sha256"] != expected_root_sha:
            raise ValueError(
                f"root SHA mismatch expected={expected_root_sha} actual={meta['sha256']}"
            )
        if int(meta["size"]) != int(expected_root_size):
            raise ValueError(
                f"root size mismatch expected={expected_root_size} actual={meta['size']}"
            )

        results = []
        for sha in sorted(assigned[key]):
            p = pick_observation(sha, key, root_obs)
            base = {
                "sha256": sha,
                "sources": ["container_net_new_407"],
                "filenames": [Path(str(p["archive_member"])).name],
                "container_url": url,
                "root_sha256": expected_root_sha,
                "archive_member": str(p["archive_member"]),
            }
            try:
                data = extract_exact(
                    archive,
                    str(p["archive_member"]),
                    sha,
                    int(p.get("size_bytes") or 0),
                    td,
                    args.command_timeout,
                    args.max_member_bytes,
                )
                first = novelty.cfb_probe(data)
                second = novelty.cfb_probe(data)
                if first != second:
                    raise RuntimeError("probe_nondeterministic")
                results.append({**base, **first, "status": "ok", "rehydrated_from": "container_delta_407"})
            except Exception as exc:
                results.append({
                    **base,
                    "status": "probe_failed",
                    "errors": [f"{type(exc).__name__}:{novelty.safe_text(exc)}"],
                })

        results.sort(key=lambda r: r["sha256"])
        (args.out / "fingerprints.json").write_text(
            json.dumps(results, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        summary = {
            "schema": SCHEMA,
            "root_index": args.root_index,
            "selected_cover_root_count": len(selected),
            "selected_cover_total_bytes": sum(int(x[2]) for x in selected),
            "root_url": url,
            "root_sha256": expected_root_sha,
            "assigned_sha": len(assigned[key]),
            "status": dict(Counter(r["status"] for r in results)),
        }
        (args.out / "summary.json").write_text(
            json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        print(json.dumps(summary, indent=2))
    return 0


def aggregate(args: argparse.Namespace) -> int:
    rows = []
    for path in args.input.rglob("fingerprints.json"):
        payload = json.loads(path.read_text(encoding="utf-8"))
        if isinstance(payload, list):
            rows.extend(payload)
    by_sha = {row["sha256"]: row for row in rows}
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
    summary.update({
        "schema": SCHEMA,
        "cohort": "container_net_new_407",
        "expected_sha": EXPECTED_SHA,
        "selected_cover_root_count": EXPECTED_COVER_ROOTS,
        "selected_cover_total_bytes": EXPECTED_COVER_BYTES,
    })
    summary_path.write_text(json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0


def self_test() -> int:
    universe = {"a", "b", "c", "d"}
    roots = {
        ("u1", "1" * 64, 10): {"a", "b"},
        ("u2", "2" * 64, 12): {"c", "d"},
        ("u3", "3" * 64, 50): {"a", "b", "c", "d"},
    }
    # Exercise assignment separately; production cover has immutable cardinality/byte assertions.
    assigned = assign_sha(universe, [("u1", "1" * 64, 10), ("u2", "2" * 64, 12)], roots)
    assert set().union(*assigned.values()) == universe
    with tempfile.TemporaryDirectory(prefix="delta407-test-") as raw:
        td = Path(raw)
        archive = td / "sample.zip"
        payload = b"container-delta-replay"
        with zipfile.ZipFile(archive, "w") as zf:
            zf.writestr("x.pub", payload)
        sha = hashlib.sha256(payload).hexdigest()
        out = extract_exact(archive, "x.pub", sha, len(payload), td, 20, 1024 * 1024)
        assert out == payload
    print("container delta 407 self-test ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("scan-root")
    s.add_argument("--receipt-artifact-id", type=int, required=True)
    s.add_argument("--receipt-artifact-sha256", required=True)
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
