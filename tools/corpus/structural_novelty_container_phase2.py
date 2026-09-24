#!/usr/bin/env python3
"""Phase-2 structural novelty replay for container-only Publisher SHA.

Reuses the exact 950-SHA ledger from structural_novelty.py and the bounded 7z
container extraction primitive from pub_container_extract.py. Each job downloads
one already-admitted public root container, verifies its root SHA-256, extracts
only exact manifest-listed .pub members, verifies member SHA-256, runs the same
source-free deterministic CFB probe as phase 1, and retains no PUB bytes.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tempfile
import time
import zipfile
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import pub_container_extract as containers  # type: ignore
import structural_novelty as novelty  # type: ignore


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fetch_root_with_retries(url: str, path: Path, timeout: float, max_bytes: int, attempts: int = 3) -> dict:
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


def exact_member_bytes(archive: Path, member: str, expected_sha: str, expected_size: int, td: Path, command_timeout: int, max_member_bytes: int) -> bytes:
    safe = hashlib.sha256(member.encode("utf-8")).hexdigest()[:16]
    out = td / f"member-{safe}.pub"
    try:
        actual_size = containers.extract_member(
            archive, member, out, command_timeout, max_member_bytes
        )
        data = out.read_bytes()
    finally:
        out.unlink(missing_ok=True)
    if expected_size and actual_size != expected_size:
        raise ValueError(
            f"member size mismatch expected={expected_size} actual={actual_size}"
        )
    actual_sha = sha256_bytes(data)
    if actual_sha != expected_sha:
        raise ValueError(
            f"member SHA mismatch expected={expected_sha} actual={actual_sha}"
        )
    return data


def container_observations(ledger: dict[str, dict]) -> tuple[list[str], dict[str, list[tuple[dict, dict]]]]:
    by_url: dict[str, list[tuple[dict, dict]]] = {}
    for item in ledger.values():
        if not item.get("deferred"):
            continue
        for source_name, row in item["rows"]:
            if source_name != "container_first_wave":
                continue
            if row.get("row_kind") != "container_member":
                continue
            url = str(row.get("container_url") or "").strip()
            if not url:
                continue
            by_url.setdefault(url, []).append((item, row))
    urls = sorted(by_url)
    return urls, by_url


def scan_container(args: argparse.Namespace) -> int:
    args.out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rar-container-novelty-") as raw_td:
        td = Path(raw_td)
        ledger, meta = novelty.load_ledger(args.sources, td / "ledger")
        deferred = [item for item in ledger.values() if item.get("deferred")]
        if len(deferred) != 425:
            raise ValueError(f"expected 425 deferred container SHA, found {len(deferred)}")

        urls, observations = container_observations(ledger)
        if len(urls) != 3:
            raise ValueError(f"expected 3 admitted root containers, found {len(urls)}")
        if args.container_index < 0 or args.container_index >= len(urls):
            raise ValueError("container-index outside admitted root set")

        url = urls[args.container_index]
        obs = observations[url]
        expected_roots = {str(row.get("root_sha256") or "") for _, row in obs}
        expected_roots.discard("")
        if len(expected_roots) != 1:
            raise ValueError(f"root SHA ambiguity for {url}: {sorted(expected_roots)}")
        expected_root = next(iter(expected_roots))

        archive = td / "root-container.bin"
        root_meta = fetch_root_with_retries(
            url, archive, args.timeout, args.max_container_bytes
        )
        if root_meta["sha256"] != expected_root:
            raise ValueError(
                f"root SHA mismatch expected={expected_root} actual={root_meta['sha256']}"
            )

        results = []
        for item, row in sorted(obs, key=lambda pair: (pair[0]["sha256"], str(pair[1].get("archive_member") or ""))):
            sha = item["sha256"]
            base = {
                "sha256": sha,
                "sources": item["sources"],
                "filenames": item["filenames"],
                "container_url": url,
                "root_sha256": expected_root,
                "archive_member": str(row.get("archive_member") or ""),
            }
            try:
                data = exact_member_bytes(
                    archive,
                    base["archive_member"],
                    sha,
                    int(row.get("size_bytes") or row.get("declared_size") or 0),
                    td,
                    args.command_timeout,
                    args.max_member_bytes,
                )
                first = novelty.cfb_probe(data)
                second = novelty.cfb_probe(data)
                if first != second:
                    raise RuntimeError("probe_nondeterministic")
                if first["source_sha256"] != sha:
                    raise RuntimeError("probe_source_sha_mismatch")
                results.append(
                    {
                        **base,
                        **first,
                        "status": "ok",
                        "rehydrated_from": "container_first_wave",
                    }
                )
            except Exception as exc:
                results.append(
                    {
                        **base,
                        "status": "probe_failed",
                        "errors": [
                            f"{type(exc).__name__}:{novelty.safe_text(exc)}"
                        ],
                    }
                )

        results.sort(key=lambda r: (r["sha256"], r.get("archive_member", "")))
        (args.out / "fingerprints.json").write_text(
            json.dumps(results, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        unique_sha = {r["sha256"] for r in results}
        summary = {
            "schema": novelty.SCHEMA,
            "phase": "container_only_phase2",
            "container_index": args.container_index,
            "container_url": url,
            "root_sha256": expected_root,
            "source_union_count": meta["union_count"],
            "deferred_container_union_count": len(deferred),
            "observation_rows": len(results),
            "unique_sha_observed": len(unique_sha),
            "status": dict(Counter(r["status"] for r in results)),
        }
        (args.out / "summary.json").write_text(
            json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        print(json.dumps(summary, indent=2))
    return 0


def aggregate_phase2(args: argparse.Namespace) -> int:
    rows = []
    for path in args.input.rglob("fingerprints.json"):
        data = json.loads(path.read_text(encoding="utf-8"))
        if isinstance(data, list):
            rows.extend(data)

    observations = len(rows)
    successes: dict[str, dict] = {}
    failures: dict[str, list[dict]] = {}
    for row in rows:
        sha = row["sha256"]
        if row.get("status") == "ok":
            prior = successes.get(sha)
            if prior is not None:
                keys = (
                    "path_fingerprint_sha256",
                    "topology_fingerprint_sha256",
                    "size_bucket_fingerprint_sha256",
                    "content_topology_fingerprint_sha256",
                )
                if any(prior.get(k) != row.get(k) for k in keys):
                    raise ValueError(f"cross-container fingerprint mismatch for {sha}")
            successes.setdefault(sha, row)
        else:
            failures.setdefault(sha, []).append(row)

    canonical = []
    for sha in sorted(set(successes) | set(failures)):
        if sha in successes:
            canonical.append(successes[sha])
        else:
            candidates = failures[sha]
            merged = dict(candidates[0])
            merged["errors"] = sorted(
                {err for row in candidates for err in row.get("errors", [])}
            )
            canonical.append(merged)

    if len(canonical) != 425:
        raise ValueError(f"expected 425 unique container SHA, got {len(canonical)}")

    temp = args.out / "_source"
    temp.mkdir(parents=True, exist_ok=True)
    (temp / "fingerprints.json").write_text(
        json.dumps(canonical, indent=2, ensure_ascii=False), encoding="utf-8"
    )
    aggregate_args = argparse.Namespace(input=temp, out=args.out)
    novelty.aggregate(aggregate_args)
    (temp / "fingerprints.json").unlink()
    temp.rmdir()

    summary_path = args.out / "summary.json"
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    summary.update(
        {
            "phase": "container_only_phase2",
            "container_observation_rows": observations,
            "container_unique_sha": 425,
            "cross_container_duplicate_observations": observations - 425,
        }
    )
    summary_path.write_text(
        json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8"
    )
    print(json.dumps(summary, indent=2))
    return 0


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="rar-container-novelty-test-") as raw_td:
        td = Path(raw_td)
        archive = td / "sample.zip"
        payload = b"phase2-container-member"
        with zipfile.ZipFile(archive, "w") as zf:
            zf.writestr("nested/test.pub", payload)
        sha = sha256_bytes(payload)
        data = exact_member_bytes(
            archive,
            "nested/test.pub",
            sha,
            len(payload),
            td,
            20,
            1024 * 1024,
        )
        assert data == payload
        try:
            exact_member_bytes(
                archive,
                "nested/test.pub",
                "0" * 64,
                len(payload),
                td,
                20,
                1024 * 1024,
            )
        except ValueError as exc:
            assert "SHA mismatch" in str(exc)
        else:
            raise AssertionError("wrong member SHA did not fail closed")
    print("container phase2 self-test ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("scan-container")
    s.add_argument("--sources", type=Path, required=True)
    s.add_argument("--out", type=Path, required=True)
    s.add_argument("--container-index", type=int, required=True)
    s.add_argument("--timeout", type=float, default=60.0)
    s.add_argument("--command-timeout", type=int, default=45)
    s.add_argument("--max-container-bytes", type=int, default=220 * 1024 * 1024)
    s.add_argument("--max-member-bytes", type=int, default=100 * 1024 * 1024)

    a = sub.add_parser("aggregate")
    a.add_argument("--input", type=Path, required=True)
    a.add_argument("--out", type=Path, required=True)

    sub.add_parser("self-test")

    args = ap.parse_args()
    if args.cmd == "scan-container":
        return scan_container(args)
    if args.cmd == "aggregate":
        return aggregate_phase2(args)
    return self_test()


if __name__ == "__main__":
    raise SystemExit(main())
