#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import harvest_pub  # type: ignore
import structural_novelty as novelty  # type: ignore
import structural_novelty_container_phase2 as phase2  # type: ignore
import version_identity_matrix as vim  # type: ignore
import wayback_pub_seed as wb  # type: ignore

SECTOR = 2048
UA = "rar-version-identity-finalize/1.0"

PHASE1_REPAIRS = {
    "7bbd0a50b896a55f8d1c58fb48cb72982cb898274f49c17fb45021bc0d2c20f0": {
        "kind": "direct",
        "url": "https://www.brucegroveprimary.com/attachments/documents.asp?id=14",
        "referer": "https://www.brucegroveprimary.com/6f-london-eye/",
    },
    "eed047711b48e28ceebed1ecb5d4bbba55ab32ec8e753724f9dfca5ae825c22e": {
        "kind": "direct",
        "url": "https://www.brucegroveprimary.com/attachments/documents.asp?id=15",
        "referer": "https://www.brucegroveprimary.com/6f-london-eye/",
    },
    "b497a8cc97cb7834f9cd4625492ddba0fa48dec9a86c09883ab0e3eecaf73a95": {
        "kind": "direct",
        "url": "https://www.brucegroveprimary.com/attachments/documents.asp?id=16",
        "referer": "https://www.brucegroveprimary.com/6f-london-eye/",
    },
    "ffe49f7b0a8ebced082bb30319ac6b12c154565727c17081ecc4c7ff830e1fdb": {
        "kind": "common_crawl",
        "row": {
            "direct_url": "http://stmichaelvisionprivateschool.com/uploads/Leaflet%20include%20Uniform.pub",
            "cc_crawl": "CC-MAIN-2024-42",
            "cc_timestamp": "20241003222222",
            "cc_digest": "NNAM3O33JIGT4HFVEBK6QX5QAQKE2G4W",
            "cc_warc_filename": "crawl-data/CC-MAIN-2024-42/segments/1727944253246.33/warc/CC-MAIN-20241003220556-20241004010556-00883.warc.gz",
            "cc_warc_offset": "29097196",
            "cc_warc_length": "86427",
            "cc_http_content_encoding": "",
            "cc_http_transfer_encoding": "",
        },
    },
}


class RangeISO:
    def __init__(self, archive_url: str):
        self.archive_url = archive_url
        self.delivery_urls = self._delivery_urls(archive_url)
        self.delivery_url = self._select_delivery()
        self.dir_cache = {}

    def _delivery_urls(self, url: str) -> list[str]:
        p = urllib.parse.urlparse(url)
        parts = p.path.split("/")
        if p.netloc != "archive.org" or len(parts) < 4 or parts[1] != "download":
            raise ValueError(f"unsupported archive URL: {url}")
        identifier = parts[2]
        filename = "/".join(parts[3:])
        req = urllib.request.Request(
            f"https://archive.org/metadata/{identifier}",
            headers={"User-Agent": UA},
        )
        with urllib.request.urlopen(req, timeout=30) as response:
            meta = json.load(response)
        directory = str(meta.get("dir") or "")
        urls = []
        for key in ("d1", "d2"):
            host = str(meta.get(key) or "")
            if host and directory:
                urls.append(
                    "https://" + host + directory.rstrip("/") + "/" +
                    urllib.parse.quote(filename, safe="/")
                )
        urls.extend([url, url + "?download=1"])
        return list(dict.fromkeys(urls))

    def _read_from(self, url: str, start: int, length: int) -> bytes:
        req = urllib.request.Request(
            url,
            headers={
                "User-Agent": UA,
                "Range": f"bytes={start}-{start + length - 1}",
                "Accept-Encoding": "identity",
            },
        )
        with urllib.request.urlopen(req, timeout=45) as response:
            status = getattr(response, "status", None)
            content_range = response.headers.get("Content-Range")
            if status != 206 and not content_range:
                raise ValueError(f"range unsupported status={status} url={url}")
            data = response.read(length + 1)
        if len(data) != length:
            raise ValueError(f"range length drift {len(data)} != {length}")
        return data

    def _select_delivery(self) -> str:
        last = None
        for url in self.delivery_urls:
            for attempt in range(4):
                try:
                    pvd = self._read_from(url, 16 * SECTOR, SECTOR)
                    if pvd[0] != 1 or pvd[1:6] != b"CD001":
                        raise ValueError("not ISO9660 primary volume descriptor")
                    print(f"range delivery selected: {url}", file=sys.stderr)
                    return url
                except Exception as exc:
                    last = exc
                    if attempt < 3:
                        time.sleep(2 ** attempt)
        assert last is not None
        raise last

    def read(self, start: int, length: int) -> bytes:
        last = None
        for attempt in range(5):
            try:
                return self._read_from(self.delivery_url, start, length)
            except Exception as exc:
                last = exc
                if attempt < 4:
                    time.sleep(2 ** attempt)
        assert last is not None
        raise last

    @staticmethod
    def _record(rec: bytes):
        extent = int.from_bytes(rec[2:6], "little")
        size = int.from_bytes(rec[10:14], "little")
        flags = rec[25]
        n = rec[32]
        raw = rec[33:33 + n]
        name = raw.decode("ascii", errors="strict")
        return name, extent, size, flags

    def _read_dir(self, extent: int, size: int):
        key = (extent, size)
        if key in self.dir_cache:
            return self.dir_cache[key]
        data = self.read(extent * SECTOR, size)
        out = {}
        pos = 0
        while pos < len(data):
            n = data[pos]
            if n == 0:
                pos = ((pos // SECTOR) + 1) * SECTOR
                continue
            rec = data[pos:pos + n]
            name, ex, sz, flags = self._record(rec)
            if name not in ("\x00", "\x01"):
                out[name.split(";")[0].upper()] = (ex, sz, flags)
            pos += n
        self.dir_cache[key] = out
        return out

    def member_bytes(self, path: str) -> bytes:
        pvd = self.read(16 * SECTOR, SECTOR)
        root_len = pvd[156]
        _, extent, size, flags = self._record(pvd[156:156 + root_len])
        parts = [x.upper() for x in path.strip("/").split("/") if x]
        for i, part in enumerate(parts):
            listing = self._read_dir(extent, size)
            key = part.split(";")[0]
            if key not in listing:
                raise KeyError(f"ISO member missing: {path} at {part}")
            extent, size, flags = listing[key]
            if i + 1 < len(parts) and not (flags & 0x02):
                raise ValueError(f"expected directory in path: {part}")
        if flags & 0x02:
            raise ValueError(f"target is directory: {path}")
        return self.read(extent * SECTOR, size)


def load_fingerprint_rows(root: Path) -> dict[str, dict]:
    out = {}
    for path in sorted(root.rglob("fingerprints.json")):
        payload = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(payload, list):
            continue
        for row in payload:
            if not isinstance(row, dict):
                continue
            sha = str(row.get("sha256") or "").lower()
            if not sha:
                continue
            current = out.get(sha)
            if current is not None and current != row:
                raise ValueError(f"duplicate conflicting fingerprint row {sha}")
            out[sha] = row
    return out


def wayback_exact_sha(original_url: str, expected_sha: str) -> bytes:
    rows = wb.exact_query(original_url, limit=100, timeout=60, retries=4)
    if not rows:
        raise ValueError(f"Wayback has no 200 captures for {original_url}")
    errors = []
    for row in rows:
        ts = str(row.get("timestamp") or "").strip()
        original = str(row.get("original") or "").strip()
        if not ts or not original:
            continue
        replay = wb.replay_url(ts, original)
        try:
            req = urllib.request.Request(
                replay,
                headers={
                    "User-Agent": UA,
                    "Accept": "*/*",
                    "Accept-Encoding": "identity",
                },
            )
            with urllib.request.urlopen(req, timeout=90) as response:
                data = response.read(100 * 1024 * 1024 + 1)
            if len(data) > 100 * 1024 * 1024:
                raise ValueError("Wayback payload exceeds bounded size")
            actual = hashlib.sha256(data).hexdigest()
            if actual == expected_sha:
                print(
                    f"Wayback exact-SHA repair selected {ts} {original_url}",
                    file=sys.stderr,
                )
                return data
            errors.append(f"{ts}:sha={actual}")
        except Exception as exc:
            errors.append(f"{ts}:{type(exc).__name__}:{exc}")
    raise ValueError(
        f"no Wayback capture matched exact SHA {expected_sha} for {original_url}; "
        f"attempts={errors[:8]}"
    )


def repair_phase1(inputs: Path, out: Path) -> dict:
    rows = load_fingerprint_rows(inputs)
    if len(rows) != 525:
        raise ValueError(f"expected 525 phase1 rows, found {len(rows)}")

    for sha, spec in PHASE1_REPAIRS.items():
        prior = rows.get(sha)
        if prior is None:
            raise ValueError(f"repair SHA absent from phase1 rows: {sha}")
        if prior.get("status") != "rehydrate_failed":
            print(f"repair SHA already available: {sha[:12]} {prior.get('status')}", file=sys.stderr)
            continue

        if spec["kind"] == "direct":
            data = wayback_exact_sha(spec["url"], sha)
        else:
            last = None
            for attempt in range(8):
                try:
                    data, _ = harvest_pub.common_crawl_fetch(
                        spec["row"], timeout=90.0, max_bytes=100 * 1024 * 1024
                    )
                    break
                except Exception as exc:
                    last = exc
                    if attempt < 7:
                        time.sleep(min(30, 2 ** attempt))
            else:
                assert last is not None
                raise last

        actual = hashlib.sha256(data).hexdigest()
        if actual != sha:
            raise ValueError(f"repair SHA mismatch expected={sha} actual={actual}")
        first = novelty.cfb_probe(data)
        second = novelty.cfb_probe(data)
        if first != second:
            raise ValueError(f"repair probe nondeterministic: {sha}")
        rows[sha] = {
            "sha256": sha,
            "sources": prior.get("sources") or [],
            "filenames": prior.get("filenames") or [],
            **first,
            "status": "ok",
            "rehydrated_from": f"bounded_repair:{spec['kind']}",
        }

    result = [rows[k] for k in sorted(rows)]
    status = Counter(str(row.get("status") or "") for row in result)
    if status != Counter({"ok": 513, "probe_failed": 12}):
        raise ValueError(f"phase1 repaired status drift: {dict(status)}")

    source = out / "_source"
    source.mkdir(parents=True, exist_ok=True)
    (source / "fingerprints.json").write_text(
        json.dumps(result, indent=2, ensure_ascii=False), encoding="utf-8"
    )
    novelty.aggregate(argparse.Namespace(input=source, out=out))
    shutil.rmtree(source)
    summary = json.loads((out / "summary.json").read_text(encoding="utf-8"))
    summary["repair_sha_count"] = len(PHASE1_REPAIRS)
    summary["rehydrate_failed_after_repair"] = 0
    (out / "summary.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8"
    )
    return summary


def range_container425_root0(out: Path) -> dict:
    with tempfile.TemporaryDirectory(prefix="rar-c425-range-") as td_raw:
        td = Path(td_raw)
        ledger, _ = novelty.load_ledger(
            HERE / "structural_novelty_sources.json", td / "ledger"
        )
        urls, observations = phase2.container_observations(ledger)
        if len(urls) != 3:
            raise ValueError(f"expected 3 original container roots, found {len(urls)}")
        url = urls[0]
        obs = observations[url]
        expected_roots = {str(row.get("root_sha256") or "") for _, row in obs}
        expected_roots.discard("")
        if len(expected_roots) != 1:
            raise ValueError(f"root0 SHA ambiguity: {sorted(expected_roots)}")
        expected_root = next(iter(expected_roots))
        if len({item["sha256"] for item, _ in obs}) != 246:
            raise ValueError("root0 expected 246 unique SHA")

        reader = RangeISO(url)
        rows = []
        by_sha = {}
        for item, row in sorted(
            obs, key=lambda pair: (pair[0]["sha256"], str(pair[1].get("archive_member") or ""))
        ):
            sha = item["sha256"]
            member = str(row.get("archive_member") or "")
            expected_size = int(row.get("size_bytes") or row.get("declared_size") or 0)
            data = reader.member_bytes(member)
            if expected_size and len(data) != expected_size:
                raise ValueError(f"member size mismatch {member}: {len(data)} != {expected_size}")
            actual = hashlib.sha256(data).hexdigest()
            if actual != sha:
                raise ValueError(f"member SHA mismatch {member}: {actual} != {sha}")
            first = novelty.cfb_probe(data)
            second = novelty.cfb_probe(data)
            if first != second:
                raise ValueError(f"probe nondeterministic: {sha}")
            candidate = {
                "sha256": sha,
                "sources": item["sources"],
                "filenames": item["filenames"],
                "container_url": url,
                "root_sha256": expected_root,
                "archive_member": member,
                **first,
                "status": "ok",
                "rehydrated_from": "container_first_wave_range",
                "root_identity_mode": "prior-full-root-sha-receipt-plus-current-exact-member-sha",
            }
            prior = by_sha.get(sha)
            if prior is not None:
                keys = (
                    "path_fingerprint_sha256",
                    "topology_fingerprint_sha256",
                    "size_bucket_fingerprint_sha256",
                    "content_topology_fingerprint_sha256",
                    "contents_family",
                    "contents_serialization_revision",
                )
                if any(prior.get(k) != candidate.get(k) for k in keys):
                    raise ValueError(f"same SHA disagreement in root0: {sha}")
            by_sha.setdefault(sha, candidate)

        rows = [by_sha[k] for k in sorted(by_sha)]
        if len(rows) != 246:
            raise ValueError(f"expected 246 canonical root0 rows, got {len(rows)}")
        out.mkdir(parents=True, exist_ok=True)
        (out / "fingerprints.json").write_text(
            json.dumps(rows, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        summary = {
            "root_url": url,
            "root_sha256_authority": expected_root,
            "delivery_url": reader.delivery_url,
            "unique_sha": 246,
            "status": {"ok": 246},
            "rehydration_mode": "ISO9660 HTTP Range; prior root SHA authority + current exact member SHA/size",
        }
        (out / "summary.json").write_text(
            json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        return summary


def build_final(args: argparse.Namespace) -> int:
    args.out.mkdir(parents=True, exist_ok=True)

    phase1_out = args.out / "phase1"
    repair_phase1(args.inputs / "phase1", phase1_out)

    root0 = args.out / "c425_root0"
    range_container425_root0(root0)

    c425_shards = args.out / "c425_shards"
    c425_shards.mkdir(parents=True, exist_ok=True)
    shutil.copytree(root0, c425_shards / "root0")
    shutil.copytree(args.inputs / "c425_root1", c425_shards / "root1")
    shutil.copytree(args.inputs / "c425_root2", c425_shards / "root2")
    c425_out = args.out / "container425"
    phase2.aggregate_phase2(argparse.Namespace(input=c425_shards, out=c425_out))

    cohorts = args.out / "cohorts"
    cohorts.mkdir(parents=True, exist_ok=True)
    shutil.copytree(phase1_out, cohorts / "phase1")
    shutil.copytree(c425_out, cohorts / "container425")
    shutil.copytree(args.inputs / "container407", cohorts / "container407")
    shutil.copytree(args.inputs / "crossarm129", cohorts / "crossarm129")

    result = vim.build(vim.load_rows(cohorts))
    matrix_out = args.out / "matrix"
    vim.write_outputs(result, matrix_out)
    summary = result["summary"]

    expected = {
        "physical_sha_count": 1486,
        "successful_physical_sha_count": 1474,
        "logical_identity_count": 1314,
        "surplus_physical_sha_count": 160,
    }
    for key, value in expected.items():
        if summary.get(key) != value:
            raise ValueError(f"matrix {key} drift: {summary.get(key)} != {value}")
    status = summary.get("status") or {}
    if status.get("ok") != 1474 or status.get("probe_failed") != 12:
        raise ValueError(f"matrix status drift: {status}")
    if status.get("rehydrate_failed", 0) != 0:
        raise ValueError(f"matrix retained rehydrate failures: {status}")

    print(json.dumps(summary, indent=2, ensure_ascii=False, sort_keys=True))
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--inputs", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    return build_final(args)


if __name__ == "__main__":
    raise SystemExit(main())
