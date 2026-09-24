#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import zipfile
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from difflib import SequenceMatcher
from pathlib import Path
from urllib.request import Request, urlopen

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import harvest_pub  # type: ignore
import govdocs1_remote_zip_pub as govdocs  # type: ignore

SCHEMA = "rar-corpus-structural-novelty/v1"
UA = "rar-corpus-structural-novelty/1.0"
SOURCE_PRIORITY = {
    "positive_domain": 0,
    "github_history": 1,
    "forum_support": 2,
    "wayback": 3,
    "internet_archive": 4,
    "common_crawl": 5,
    "govdocs1": 6,
    "container_first_wave": 7,
}


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def download_artifact(repo: str, artifact_id: int, token: str, dest: Path) -> None:
    url = f"https://api.github.com/repos/{repo}/actions/artifacts/{artifact_id}/zip"
    headers = {
        "User-Agent": UA,
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = Request(url, headers=headers)
    with urlopen(req, timeout=60) as resp:
        data = resp.read(64 * 1024 * 1024 + 1)
    if len(data) > 64 * 1024 * 1024:
        raise ValueError(f"artifact {artifact_id} exceeds 64 MiB manifest-artifact cap")
    dest.write_bytes(data)


def safe_extract_zip(path: Path, dest: Path) -> None:
    with zipfile.ZipFile(path) as zf:
        for info in zf.infolist():
            p = Path(info.filename)
            if p.is_absolute() or ".." in p.parts:
                raise ValueError(f"unsafe artifact member {info.filename!r}")
        zf.extractall(dest)


def rows_from_json(path: Path) -> list[dict]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return []
    if not isinstance(data, list):
        return []
    rows = [x for x in data if isinstance(x, dict)]
    if not rows or not any("sha256" in x and "classification" in x for x in rows):
        return []
    return rows


def complete_cfb_rows(root: Path) -> list[dict]:
    rows: list[dict] = []
    for p in root.rglob("*.json"):
        for row in rows_from_json(p):
            if row.get("classification") != "cfb_publisher_hint":
                continue
            sha = str(row.get("sha256") or "").lower()
            if not re.fullmatch(r"[0-9a-f]{64}", sha):
                continue
            if str(row.get("cc_warc_truncated") or "").strip():
                continue
            rows.append(row)
    return rows


def load_ledger(config_path: Path, work: Path) -> tuple[dict[str, dict], dict]:
    config = json.loads(config_path.read_text(encoding="utf-8"))
    token = os.environ.get("GITHUB_TOKEN", "")
    repo = config["repository"]
    by_sha: dict[str, dict] = {}
    source_counts: dict[str, int] = {}
    source_config = {x["name"]: x for x in config["sources"]}

    for source in config["sources"]:
        name = source["name"]
        artifact_id = int(source["artifact_id"])
        source_dir = work / "artifacts" / name
        source_dir.mkdir(parents=True, exist_ok=True)
        archive = work / f"{name}.zip"
        download_artifact(repo, artifact_id, token, archive)
        safe_extract_zip(archive, source_dir)
        rows = complete_cfb_rows(source_dir)
        unique = {str(r["sha256"]).lower() for r in rows}
        source_counts[name] = len(unique)
        expected = int(source["expected_complete_cfb"])
        if len(unique) != expected:
            raise ValueError(
                f"source {name}: expected {expected} complete CFB SHA, found {len(unique)}"
            )
        for row in rows:
            sha = str(row["sha256"]).lower()
            item = by_sha.setdefault(
                sha,
                {
                    "sha256": sha,
                    "sources": set(),
                    "filenames": set(),
                    "rows": [],
                    "deferred": False,
                },
            )
            item["sources"].add(name)
            fn = str(row.get("candidate_filename") or row.get("archive_member") or "").strip()
            if fn:
                item["filenames"].add(Path(fn).name)
            item["rows"].append((name, row))

    if len(by_sha) != 950:
        raise ValueError(f"expected exact union 950 SHA, got {len(by_sha)}")

    for item in by_sha.values():
        item["sources"] = sorted(item["sources"])
        item["filenames"] = sorted(item["filenames"])
        item["rows"].sort(key=lambda x: SOURCE_PRIORITY.get(x[0], 99))
        item["deferred"] = all(
            bool(source_config[name].get("defer_rehydrate", False))
            for name, _ in item["rows"]
        )

    return by_sha, {"source_counts": source_counts, "union_count": len(by_sha)}


def fetch_direct(row: dict, timeout: float, max_bytes: int) -> bytes:
    url = str(
        row.get("direct_url") or row.get("resolved_url") or row.get("final_url") or ""
    ).strip()
    if not url:
        raise ValueError("no direct URL")
    data, _ = harvest_pub.fetch_with_retries(url, timeout, max_bytes, retries=2)
    return data


def fetch_zip_member(
    row: dict, timeout: float, max_bytes: int, max_archive_bytes: int
) -> bytes:
    url = str(row.get("parent_archive_url") or row.get("direct_url") or "").strip()
    member = str(row.get("archive_member") or "").strip()
    if not url or not member:
        raise ValueError("archive member coordinates missing")
    archive, _ = harvest_pub.fetch_with_retries(
        url, timeout, max_archive_bytes, retries=2
    )
    with zipfile.ZipFile(io.BytesIO(archive)) as zf:
        info = zf.getinfo(member)
        if info.file_size > max_bytes:
            raise ValueError(f"ZIP member exceeds max bytes: {info.file_size}")
        data = zf.read(info)
    if len(data) > max_bytes:
        raise ValueError("ZIP member decoded beyond max bytes")
    return data


def fetch_govdocs(row: dict, timeout: float, max_bytes: int) -> bytes:
    url = str(row.get("container_url") or "").strip()
    member = str(row.get("archive_member") or "").strip()
    entries, _ = govdocs.list_remote_zip(url, timeout)
    matches = [e for e in entries if e.name == member]
    if len(matches) != 1:
        raise ValueError(f"GovDocs member match count {len(matches)}")
    return govdocs.fetch_member(url, matches[0], timeout, max_bytes)


def rehydrate(
    name: str,
    row: dict,
    timeout: float,
    max_bytes: int,
    max_archive_bytes: int,
) -> bytes:
    if name == "common_crawl":
        data, _ = harvest_pub.common_crawl_fetch(row, timeout, max_bytes)
        return data
    if name == "govdocs1" or row.get("row_kind") == "govdocs1_zip_member":
        return fetch_govdocs(row, timeout, max_bytes)
    if row.get("row_kind") == "archive_member" and row.get("parent_archive_url"):
        return fetch_zip_member(row, timeout, max_bytes, max_archive_bytes)
    if row.get("row_kind") == "container_member":
        raise RuntimeError("deferred_container")
    return fetch_direct(row, timeout, max_bytes)


def normalize_error(text: str) -> str:
    text = re.sub(r"/tmp/[^\s:]+", "<tmp>", text)
    text = re.sub(r"/home/runner/work/[^\s:]+", "<runner>", text)
    return text[:800]


def run_probe(probe: Path, pub_path: Path) -> dict:
    proc = subprocess.run(
        [str(probe), str(pub_path)],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=120,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(normalize_error(proc.stderr or proc.stdout))
    return json.loads(proc.stdout)


def scan_one(
    item: dict,
    probe: Path,
    temp_root: Path,
    timeout: float,
    max_bytes: int,
    max_archive_bytes: int,
) -> dict:
    sha = item["sha256"]
    base = {
        "sha256": sha,
        "sources": item["sources"],
        "filenames": item["filenames"],
    }
    if item["deferred"]:
        return {**base, "status": "deferred_container"}

    errors = []
    data = None
    used_source = None
    for name, row in item["rows"]:
        if name == "container_first_wave":
            continue
        try:
            candidate = rehydrate(name, row, timeout, max_bytes, max_archive_bytes)
            actual = sha256_bytes(candidate)
            if actual != sha:
                raise ValueError(f"SHA mismatch expected={sha} actual={actual}")
            data = candidate
            used_source = name
            break
        except Exception as exc:
            errors.append(
                f"{name}:{type(exc).__name__}:{normalize_error(str(exc))}"
            )
    if data is None:
        return {**base, "status": "rehydrate_failed", "errors": errors}

    path = temp_root / f"{sha}.pub"
    path.write_bytes(data)
    try:
        first = run_probe(probe, path)
        second = run_probe(probe, path)
        if first != second:
            raise RuntimeError("probe_nondeterministic")
        if first.get("source_sha256") != sha:
            raise RuntimeError("probe_source_sha_mismatch")
        return {
            **base,
            "status": "ok",
            "rehydrated_from": used_source,
            "byte_len": first.get("byte_len"),
            "family": first.get("family"),
            "stream_count": first.get("stream_count"),
            "topology_fingerprint_sha256": first.get(
                "topology_fingerprint_sha256"
            ),
            "content_topology_fingerprint_sha256": first.get(
                "content_topology_fingerprint_sha256"
            ),
            "structural_base_status": first.get("structural_base_status"),
            "structural_base_error": first.get("structural_base_error"),
            "streams": first.get("streams", []),
            "mature": first.get("mature"),
        }
    except Exception as exc:
        return {
            **base,
            "status": "probe_failed",
            "rehydrated_from": used_source,
            "errors": [normalize_error(str(exc))],
        }
    finally:
        path.unlink(missing_ok=True)


def scan(args: argparse.Namespace) -> int:
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rar-novelty-") as td:
        root = Path(td)
        ledger, meta = load_ledger(args.sources, root)
        items = [ledger[k] for k in sorted(ledger)]
        nondeferred = [x for x in items if not x["deferred"]]
        if args.only_source:
            nondeferred = [
                x for x in nondeferred if args.only_source in x["sources"]
            ]
        selected = [
            item
            for idx, item in enumerate(nondeferred)
            if idx % args.shard_count == args.shard_index
        ]
        if args.limit > 0:
            selected = selected[: args.limit]

        results = []
        temp_raw = root / "raw"
        temp_raw.mkdir()
        with ThreadPoolExecutor(max_workers=args.workers) as pool:
            futures = {
                pool.submit(
                    scan_one,
                    item,
                    args.probe,
                    temp_raw,
                    args.timeout,
                    args.max_bytes,
                    args.max_archive_bytes,
                ): item["sha256"]
                for item in selected
            }
            for future in as_completed(futures):
                result = future.result()
                results.append(result)
                print(
                    f"{result['sha256'][:12]} {result['status']}",
                    file=sys.stderr,
                )
        results.sort(key=lambda r: r["sha256"])

        (out / "fingerprints.json").write_text(
            json.dumps(results, indent=2, ensure_ascii=False),
            encoding="utf-8",
        )
        summary = {
            "schema": SCHEMA,
            "shard_index": args.shard_index,
            "shard_count": args.shard_count,
            "source_union_count": meta["union_count"],
            "deferred_container_union_count": sum(x["deferred"] for x in items),
            "nondeferred_union_count": len(nondeferred),
            "selected": len(selected),
            "status": dict(Counter(r["status"] for r in results)),
            "family": dict(
                Counter(
                    r.get("family", "")
                    for r in results
                    if r.get("family")
                )
            ),
            "source_counts": meta["source_counts"],
        }
        (out / "summary.json").write_text(
            json.dumps(summary, indent=2, ensure_ascii=False),
            encoding="utf-8",
        )
        print(json.dumps(summary, indent=2))
    return 0


def best_name(row: dict) -> str:
    names = row.get("filenames") or []
    return names[0] if names else ""


def normalized_stem(name: str) -> str:
    stem = Path(name).stem.casefold()
    stem = re.sub(r"\b(?:19|20)\d{2}\b", "<year>", stem)
    stem = re.sub(r"[_\-]+", " ", stem)
    return re.sub(r"\s+", " ", stem).strip()


def aggregate(args: argparse.Namespace) -> int:
    rows = []
    for p in args.input.rglob("fingerprints.json"):
        data = json.loads(p.read_text(encoding="utf-8"))
        if isinstance(data, list):
            rows.extend(data)
    by_sha = {r["sha256"]: r for r in rows}
    rows = [by_sha[k] for k in sorted(by_sha)]

    clusters: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        if row.get("status") != "ok":
            continue
        mature = row.get("mature") or {}
        key = mature.get("layout_fingerprint_sha256") or row.get(
            "topology_fingerprint_sha256"
        )
        if key:
            clusters[key].append(row)

    cluster_rows = []
    pair_candidates = []
    for key, members in sorted(
        clusters.items(), key=lambda kv: (-len(kv[1]), kv[0])
    ):
        cluster_rows.append(
            {
                "fingerprint": key,
                "size": len(members),
                "families": dict(
                    Counter(x.get("family", "") for x in members)
                ),
                "sources": sorted(
                    {s for x in members for s in x.get("sources", [])}
                ),
                "sha256": [x["sha256"] for x in members],
            }
        )
        if 1 < len(members) <= 20:
            for i, left in enumerate(members):
                for right in members[i + 1 :]:
                    a = normalized_stem(best_name(left))
                    b = normalized_stem(best_name(right))
                    similarity = (
                        SequenceMatcher(None, a, b).ratio()
                        if a and b
                        else 0.0
                    )
                    if similarity >= 0.65:
                        pair_candidates.append(
                            {
                                "left_sha256": left["sha256"],
                                "right_sha256": right["sha256"],
                                "left_name": best_name(left),
                                "right_name": best_name(right),
                                "name_similarity": round(similarity, 4),
                                "structural_fingerprint": key,
                                "sources": sorted(
                                    set(left.get("sources", []))
                                    | set(right.get("sources", []))
                                ),
                            }
                        )

    status = Counter(r.get("status", "") for r in rows)
    family = Counter(
        r.get("family", "") for r in rows if r.get("family")
    )
    diagnostic_codes = Counter()
    shape_types = Counter()
    for row in rows:
        mature = row.get("mature") or {}
        diagnostic_codes.update(
            mature.get("diagnostic_code_counts") or {}
        )
        shape_types.update(mature.get("shape_type_counts") or {})

    summary = {
        "schema": SCHEMA,
        "analyzed_rows": len(rows),
        "status": dict(status),
        "family": dict(family),
        "structural_cluster_count": len(clusters),
        "singleton_cluster_count": sum(
            len(v) == 1 for v in clusters.values()
        ),
        "multi_member_cluster_count": sum(
            len(v) > 1 for v in clusters.values()
        ),
        "largest_cluster_size": max(
            (len(v) for v in clusters.values()), default=0
        ),
        "pair_candidate_count": len(pair_candidates),
        "diagnostic_code_counts": dict(diagnostic_codes),
        "shape_type_counts": dict(shape_types),
    }
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "fingerprints.json").write_text(
        json.dumps(rows, indent=2, ensure_ascii=False),
        encoding="utf-8",
    )
    (args.out / "clusters.json").write_text(
        json.dumps(cluster_rows, indent=2, ensure_ascii=False),
        encoding="utf-8",
    )
    (args.out / "pair_candidates.json").write_text(
        json.dumps(pair_candidates, indent=2, ensure_ascii=False),
        encoding="utf-8",
    )
    (args.out / "summary.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=False),
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2))
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    scan_p = sub.add_parser("scan")
    scan_p.add_argument("--sources", type=Path, required=True)
    scan_p.add_argument("--probe", type=Path, required=True)
    scan_p.add_argument("--out", type=Path, required=True)
    scan_p.add_argument("--shard-index", type=int, default=0)
    scan_p.add_argument("--shard-count", type=int, default=1)
    scan_p.add_argument("--workers", type=int, default=4)
    scan_p.add_argument("--limit", type=int, default=0)
    scan_p.add_argument("--only-source", default="")
    scan_p.add_argument("--timeout", type=float, default=30.0)
    scan_p.add_argument("--max-bytes", type=int, default=100 * 1024 * 1024)
    scan_p.add_argument(
        "--max-archive-bytes", type=int, default=120 * 1024 * 1024
    )

    agg_p = sub.add_parser("aggregate")
    agg_p.add_argument("--input", type=Path, required=True)
    agg_p.add_argument("--out", type=Path, required=True)

    args = ap.parse_args()
    if args.cmd == "scan":
        return scan(args)
    return aggregate(args)


if __name__ == "__main__":
    raise SystemExit(main())
