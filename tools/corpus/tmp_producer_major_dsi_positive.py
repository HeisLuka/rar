#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import os
import tempfile
import time
import urllib.parse
import urllib.request
import zipfile
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

import olefile

HERE = Path(__file__).resolve().parent
import sys
sys.path.insert(0, str(HERE))
import structural_novelty as novelty  # type: ignore

DSI_PATH = "/\x05DocumentSummaryInformation"
DSI_OLE_NAME = "\x05DocumentSummaryInformation"
T645_ARTIFACT_ID = 10853776519
POST_M1 = {
    "post_m1_delta:class_templates": (10811670607, 52),
    "post_m1_delta:training": (10811710784, 15),
    "post_m1_delta:mvp_matrix": (10811134424, 62),
}
KNOWN_MAJOR_BY_DSI = {
    "ac4f8d12127c45f757b9681ca0af31bc12e74d1bf1cf7677e2e82bdeebad8ec1": 12,
    "c1e61d45dd70af3fb76a32acd3f52f8d20777d725c513e1928be2e61763db021": 14,
}
UA = "rar-t694-dsi-producer-major/1.0"
SECTOR = 2048


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def u16(data: bytes, off: int) -> int:
    if off < 0 or off + 2 > len(data):
        raise ValueError("u16 out of bounds")
    return int.from_bytes(data[off:off + 2], "little")


def u32(data: bytes, off: int) -> int:
    if off < 0 or off + 4 > len(data):
        raise ValueError("u32 out of bounds")
    return int.from_bytes(data[off:off + 4], "little")


def decode_piddsi_stream(raw: bytes) -> dict:
    base = {"dsi_stream_len": len(raw), "dsi_stream_sha256": sha256(raw)}
    if not raw:
        return {**base, "state": "empty_stream"}
    try:
        if len(raw) < 28:
            raise ValueError("property stream shorter than header")
        if raw[0:2] != b"\xfe\xff":
            raise ValueError(f"unexpected byte order {raw[0:2].hex()}")
        section_count = u32(raw, 24)
        if section_count < 1 or section_count > 8:
            raise ValueError(f"invalid section count {section_count}")
        if 28 + section_count * 20 > len(raw):
            raise ValueError("section descriptor table truncated")

        hits = []
        for si in range(section_count):
            desc = 28 + si * 20
            sec_off = u32(raw, desc + 16)
            if sec_off + 8 > len(raw):
                raise ValueError(f"section {si} header out of bounds")
            sec_size = u32(raw, sec_off)
            prop_count = u32(raw, sec_off + 4)
            if sec_size < 8 or sec_off + sec_size > len(raw):
                raise ValueError(f"section {si} size invalid")
            if prop_count > 4096 or sec_off + 8 + prop_count * 8 > sec_off + sec_size:
                raise ValueError(f"section {si} property table invalid")
            for pi in range(prop_count):
                ent = sec_off + 8 + pi * 8
                pid = u32(raw, ent)
                rel = u32(raw, ent + 4)
                if pid != 0x17:
                    continue
                val_off = sec_off + rel
                if val_off + 4 > sec_off + sec_size:
                    hits.append({
                        "state": "malformed_value",
                        "section": si,
                        "reason": "typed value header out of bounds",
                    })
                    continue
                vt = u16(raw, val_off)
                if vt != 3:
                    hits.append({
                        "state": "malformed_type",
                        "section": si,
                        "vt": vt,
                    })
                    continue
                if val_off + 8 > sec_off + sec_size:
                    hits.append({
                        "state": "malformed_value",
                        "section": si,
                        "vt": vt,
                        "reason": "VT_I4 value out of bounds",
                    })
                    continue
                value = u32(raw, val_off + 4)
                hits.append({
                    "state": "valid_vt_i4",
                    "section": si,
                    "vt": vt,
                    "value_u32": value,
                    "value_hex": f"0x{value:08X}",
                    "producer_major": (value >> 16) & 0xFFFF,
                    "producer_build_or_minor": value & 0xFFFF,
                })

        if not hits:
            return {**base, "state": "property_absent"}
        valid = [h for h in hits if h["state"] == "valid_vt_i4"]
        if valid:
            values = {h["value_u32"] for h in valid}
            if len(values) != 1 or len(valid) != len(hits):
                return {
                    **base,
                    "state": "malformed_value",
                    "error": "multiple/conflicting PIDDSI_VERSION observations",
                    "observations": hits,
                }
            return {**base, **valid[0], "observation_count": len(valid)}
        return {**base, **hits[0], "observations": hits}
    except Exception as exc:
        return {
            **base,
            "state": "malformed_value",
            "error": f"{type(exc).__name__}:{exc}",
        }


def parse_pub_dsi(pub: bytes, expected_dsi_sha: str, expected_dsi_len: int) -> dict:
    with olefile.OleFileIO(io.BytesIO(pub)) as ole:
        paths = ole.listdir(streams=True, storages=False)
        target = None
        for path in paths:
            if path and path[-1] == DSI_OLE_NAME:
                target = path
                break
        if target is None:
            raise ValueError("expected DocumentSummaryInformation stream is missing")
        raw = ole.openstream(target).read()

    actual_sha = sha256(raw)
    if actual_sha != expected_dsi_sha:
        raise ValueError(f"DSI SHA mismatch {actual_sha} != {expected_dsi_sha}")
    if len(raw) != expected_dsi_len:
        raise ValueError(f"DSI length mismatch {len(raw)} != {expected_dsi_len}")

    decoded = decode_piddsi_stream(raw)

    # Independent olefile parser cross-check when PID 0x17 is valid.
    if decoded.get("state") == "valid_vt_i4":
        with olefile.OleFileIO(io.BytesIO(pub)) as ole:
            props = ole.getproperties([DSI_OLE_NAME], convert_time=False, no_conversion=[0x17])
            ov = props.get(0x17)
        if isinstance(ov, bytes) and len(ov) == 4:
            ov = int.from_bytes(ov, "little")
        if isinstance(ov, int) and ov != decoded["value_u32"]:
            raise ValueError(
                f"olefile/raw PIDDSI disagreement {ov} != {decoded['value_u32']}"
            )
    return decoded


def load_t645(t645_dir: Path) -> tuple[list[dict], dict, dict]:
    rows_by_sha: dict[str, dict] = {}
    for path in sorted((t645_dir / "cohorts").glob("*/fingerprints.json")):
        payload = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(payload, list):
            raise ValueError(f"fingerprint payload not a list: {path}")
        for row in payload:
            if row.get("status") != "ok":
                continue
            physical = str(row.get("sha256") or "").lower()
            if not physical:
                raise ValueError(f"missing physical SHA in {path}")
            prior = rows_by_sha.get(physical)
            if prior is not None and prior != row:
                raise ValueError(f"conflicting physical fingerprint row {physical}")
            rows_by_sha[physical] = row

    rows = [rows_by_sha[k] for k in sorted(rows_by_sha)]
    if len(rows) != 1474:
        raise ValueError(f"expected 1474 successful physical rows, got {len(rows)}")

    logical_members: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        logical = str(row.get("content_topology_fingerprint_sha256") or "")
        if not logical:
            raise ValueError(f"missing logical identity for {row['sha256']}")
        logical_members[logical].append(row)
    if len(logical_members) != 1314:
        raise ValueError(f"expected 1314 logical identities, got {len(logical_members)}")

    logical_rows = {}
    dsi_groups: dict[str, dict] = {}
    positive = 0
    for logical, members in sorted(logical_members.items()):
        dsi_descriptors = []
        for member in members:
            dsi = next(
                (s for s in member.get("streams", []) if s.get("path") == DSI_PATH),
                None,
            )
            dsi_descriptors.append(dsi)
        present = [d for d in dsi_descriptors if d is not None]
        if not present:
            logical_rows[logical] = {
                "logical_identity": logical,
                "dsi_present": False,
                "physical_member_count": len(members),
            }
            continue
        if len(present) != len(members):
            raise ValueError(f"mixed DSI presence inside logical identity {logical}")
        sigs = {(int(d["len"]), str(d["sha256"]).lower()) for d in present}
        if len(sigs) != 1:
            raise ValueError(f"mixed DSI stream identity inside logical identity {logical}")
        dsi_len, dsi_sha = next(iter(sigs))
        positive += 1
        logical_rows[logical] = {
            "logical_identity": logical,
            "dsi_present": True,
            "dsi_sha256": dsi_sha,
            "dsi_len": dsi_len,
            "physical_member_count": len(members),
        }
        group = dsi_groups.setdefault(
            dsi_sha,
            {
                "dsi_sha256": dsi_sha,
                "dsi_len": dsi_len,
                "logical_identities": set(),
                "physical_candidates": {},
                "families": set(),
                "revisions": set(),
                "sources": set(),
                "filenames": set(),
            },
        )
        if group["dsi_len"] != dsi_len:
            raise ValueError(f"same DSI SHA has two lengths: {dsi_sha}")
        group["logical_identities"].add(logical)
        for member in members:
            group["physical_candidates"][member["sha256"]] = member
            group["families"].add(str(member.get("contents_family")))
            group["revisions"].add(member.get("contents_serialization_revision"))
            for source in member.get("sources") or []:
                group["sources"].add(str(source))
            for filename in member.get("filenames") or []:
                group["filenames"].add(str(filename))

    if positive != 551:
        raise ValueError(f"expected 551 DSI-positive logical identities, got {positive}")
    if len(dsi_groups) != 98:
        raise ValueError(f"expected 98 unique DSI hashes, got {len(dsi_groups)}")

    return rows, logical_rows, dsi_groups


def artifact_extract(repo: str, artifact_id: int, token: str, out: Path) -> None:
    archive = out.with_suffix(".zip")
    novelty.download_artifact(repo, artifact_id, token, archive)
    out.mkdir(parents=True, exist_ok=True)
    novelty.safe_extract_zip(archive, out)


def build_post_m1_raw_index(repo: str, token: str, work: Path) -> dict[str, tuple[str, bytes]]:
    receipt = HERE / "receipts" / "corpus-post-m1-delta-2026-09-24.tsv"
    admitted_by_lane: dict[str, set[str]] = defaultdict(set)
    with receipt.open("r", encoding="utf-8", newline="") as fh:
        for row in csv.DictReader(fh, delimiter="\t"):
            admitted_by_lane[str(row["lane"])].add(str(row["sha256"]).lower())

    out: dict[str, tuple[str, bytes]] = {}
    for source_name, (artifact_id, expected) in POST_M1.items():
        lane = source_name.split(":", 1)[1]
        wanted = admitted_by_lane[lane]
        if len(wanted) != expected:
            raise ValueError(f"post-M1 admission drift for {lane}: {len(wanted)} != {expected}")
        source_dir = work / f"post_{lane}"
        artifact_extract(repo, artifact_id, token, source_dir)
        raw_dir = source_dir / "novel"
        seen = set()
        for path in sorted(raw_dir.glob("*.pub")):
            data = path.read_bytes()
            physical = sha256(data)
            if physical not in wanted:
                continue
            seen.add(physical)
            prior = out.get(physical)
            if prior is not None and prior[1] != data:
                raise ValueError(f"same post-M1 physical SHA has conflicting bytes: {physical}")
            out[physical] = (source_name, data)
        if seen != wanted:
            missing = sorted(wanted - seen)
            raise ValueError(f"post-M1 artifact {lane} missing {len(missing)} admitted SHA")
    if len(out) != 129:
        raise ValueError(f"expected 129 post-M1 raw files, got {len(out)}")
    return out


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
        with urllib.request.urlopen(req, timeout=45) as response:
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
        with urllib.request.urlopen(req, timeout=60) as response:
            status = getattr(response, "status", None)
            if status != 206 and not response.headers.get("Content-Range"):
                raise ValueError(f"range unsupported status={status}")
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
                    if pvd[:6] != b"\x01CD001":
                        raise ValueError("not ISO9660 PVD")
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
                raise ValueError(f"expected directory at {part}")
        if flags & 0x02:
            raise ValueError(f"target is directory: {path}")
        return self.read(extent * SECTOR, size)


def physical_rank(row: dict, post_index: dict[str, tuple[str, bytes]], base_ledger: dict[str, dict]):
    physical = str(row["sha256"]).lower()
    if physical in post_index:
        return (0, int(row.get("byte_len") or 0), physical)
    if physical in base_ledger and not base_ledger[physical].get("deferred"):
        return (1, int(row.get("byte_len") or 0), physical)
    if "container_net_new_407" in (row.get("sources") or []):
        return (2, int(row.get("byte_len") or 0), physical)
    return (9, int(row.get("byte_len") or 0), physical)


def recover_candidate(
    row: dict,
    expected_dsi_sha: str,
    expected_dsi_len: int,
    post_index: dict[str, tuple[str, bytes]],
    base_ledger: dict[str, dict],
    iso_cache: dict[str, RangeISO],
) -> tuple[str, bytes]:
    physical = str(row["sha256"]).lower()
    expected_len = int(row.get("byte_len") or 0)

    if physical in post_index:
        source_name, data = post_index[physical]
        if sha256(data) != physical:
            raise ValueError("post-M1 physical SHA mismatch")
        if expected_len and len(data) != expected_len:
            raise ValueError("post-M1 byte length mismatch")
        parse_pub_dsi(data, expected_dsi_sha, expected_dsi_len)
        return source_name, data

    item = base_ledger.get(physical)
    if item is not None and not item.get("deferred"):
        errors = []
        for source_name, source_row in item["rows"]:
            if source_name == "container_first_wave":
                continue
            try:
                data = novelty.rehydrate(
                    source_name, source_row, timeout=90.0,
                    max_bytes=100 * 1024 * 1024,
                    max_archive_bytes=500 * 1024 * 1024,
                )
                if sha256(data) != physical:
                    raise ValueError("base physical SHA mismatch")
                if expected_len and len(data) != expected_len:
                    raise ValueError("base byte length mismatch")
                parse_pub_dsi(data, expected_dsi_sha, expected_dsi_len)
                return source_name, data
            except Exception as exc:
                errors.append(f"{source_name}:{type(exc).__name__}:{exc}")
        raise ValueError("base rehydrate failed: " + " | ".join(errors[:8]))

    if "container_net_new_407" in (row.get("sources") or []):
        url = str(row.get("container_url") or "")
        member = str(row.get("archive_member") or "")
        root_sha = str(row.get("root_sha256") or "")
        if not url or not member or not root_sha:
            raise ValueError("container row lacks exact coordinates")
        reader = iso_cache.get(url)
        if reader is None:
            reader = RangeISO(url)
            iso_cache[url] = reader
        data = reader.member_bytes(member)
        if expected_len and len(data) != expected_len:
            raise ValueError(f"container byte length mismatch {len(data)} != {expected_len}")
        actual = sha256(data)
        if actual != physical:
            raise ValueError(f"container member SHA mismatch {actual} != {physical}")
        parse_pub_dsi(data, expected_dsi_sha, expected_dsi_len)
        return f"container_net_new_407:{root_sha}", data

    raise ValueError("no supported exact rehydration route for physical candidate")


def execute(args: argparse.Namespace) -> int:
    token = os.environ.get("GITHUB_TOKEN", "")
    repo = os.environ.get("GITHUB_REPOSITORY", "HeisLuka/rar")
    args.out.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="rar-t694-") as td_raw:
        work = Path(td_raw)

        # Restore accepted source-free T645 authority.
        t645_dir = work / "t645"
        artifact_extract(repo, T645_ARTIFACT_ID, token, t645_dir)
        _, logical_rows, dsi_groups = load_t645(t645_dir)

        # Load exact acquisition coordinates for the original 950-SHA source union.
        base_ledger, base_meta = novelty.load_ledger(
            HERE / "structural_novelty_sources.json",
            work / "base_ledger",
        )
        if base_meta.get("union_count") != 950:
            raise ValueError(f"base source union drift: {base_meta}")

        # Restore the 129 admitted post-M1 payloads from their retained source artifacts.
        post_index = build_post_m1_raw_index(repo, token, work)

        iso_cache: dict[str, RangeISO] = {}
        unique_rows = []
        failures = []

        def one_group(dsi_sha: str, group: dict) -> dict:
            candidates = sorted(
                group["physical_candidates"].values(),
                key=lambda row: physical_rank(row, post_index, base_ledger),
            )
            errors = []
            for row in candidates:
                if physical_rank(row, post_index, base_ledger)[0] >= 9:
                    continue
                try:
                    source_name, data = recover_candidate(
                        row, dsi_sha, int(group["dsi_len"]),
                        post_index, base_ledger, iso_cache,
                    )
                    decoded = parse_pub_dsi(data, dsi_sha, int(group["dsi_len"]))
                    result = {
                        "dsi_sha256": dsi_sha,
                        "dsi_len": int(group["dsi_len"]),
                        "logical_identity_count": len(group["logical_identities"]),
                        "physical_candidate_count": len(group["physical_candidates"]),
                        "families": sorted(group["families"]),
                        "serialization_revisions": sorted(
                            group["revisions"],
                            key=lambda x: (-1 if x is None else int(x)),
                        ),
                        "source_classes": sorted(group["sources"]),
                        "representative_physical_sha256": row["sha256"],
                        "representative_filename": sorted(row.get("filenames") or [])[:3],
                        "representative_source": source_name,
                        "evidence_class": (
                            "known_writer_control+decoded_representative"
                            if dsi_sha in KNOWN_MAJOR_BY_DSI
                            else "decoded_representative"
                        ),
                        **decoded,
                    }
                    expected_major = KNOWN_MAJOR_BY_DSI.get(dsi_sha)
                    if expected_major is not None:
                        if result.get("state") != "valid_vt_i4":
                            raise ValueError(
                                f"known writer anchor {dsi_sha} did not decode valid VT_I4"
                            )
                        if result.get("producer_major") != expected_major:
                            raise ValueError(
                                f"known writer anchor major drift {result.get('producer_major')} != {expected_major}"
                            )
                    return result
                except Exception as exc:
                    errors.append({
                        "physical_sha256": row["sha256"],
                        "sources": row.get("sources") or [],
                        "error": f"{type(exc).__name__}:{exc}",
                    })
            return {
                "dsi_sha256": dsi_sha,
                "dsi_len": int(group["dsi_len"]),
                "logical_identity_count": len(group["logical_identities"]),
                "physical_candidate_count": len(group["physical_candidates"]),
                "families": sorted(group["families"]),
                "serialization_revisions": sorted(
                    group["revisions"],
                    key=lambda x: (-1 if x is None else int(x)),
                ),
                "source_classes": sorted(group["sources"]),
                "state": "rehydrate_failure",
                "errors": errors[:20],
            }

        # Keep modest concurrency: Common Crawl/Wayback are remote and should not be hammered.
        with ThreadPoolExecutor(max_workers=6) as pool:
            futures = {
                pool.submit(one_group, dsi_sha, group): dsi_sha
                for dsi_sha, group in sorted(dsi_groups.items())
            }
            for future in as_completed(futures):
                result = future.result()
                unique_rows.append(result)
                print(
                    result["dsi_sha256"][:12],
                    result.get("state"),
                    result.get("producer_major"),
                    result.get("logical_identity_count"),
                    flush=True,
                )

        unique_rows.sort(key=lambda row: row["dsi_sha256"])
        if len(unique_rows) != 98:
            raise ValueError(f"unique DSI output count drift: {len(unique_rows)}")

        by_hash = {row["dsi_sha256"]: row for row in unique_rows}
        logical_projection = []
        for logical, base in sorted(logical_rows.items()):
            row = dict(base)
            if row["dsi_present"]:
                decoded = by_hash[row["dsi_sha256"]]
                row["piddsi_state"] = decoded.get("state")
                for key in (
                    "value_u32",
                    "value_hex",
                    "producer_major",
                    "producer_build_or_minor",
                ):
                    if key in decoded:
                        row[key] = decoded[key]
            logical_projection.append(row)

        state_counts = Counter(row.get("state") for row in unique_rows)
        logical_state_counts = Counter()
        producer_major_counts = Counter()
        valid_logical = 0
        for row in logical_projection:
            if not row["dsi_present"]:
                logical_state_counts["dsi_absent"] += 1
                continue
            state = row.get("piddsi_state")
            logical_state_counts[state] += 1
            if state == "valid_vt_i4":
                valid_logical += 1
                producer_major_counts[str(row["producer_major"])] += 1

        cross_revision = [
            {
                "dsi_sha256": row["dsi_sha256"],
                "producer_major": row.get("producer_major"),
                "families": row["families"],
                "serialization_revisions": row["serialization_revisions"],
                "logical_identity_count": row["logical_identity_count"],
            }
            for row in unique_rows
            if len(row["families"]) > 1 or len(row["serialization_revisions"]) > 1
        ]

        summary = {
            "schema": "chaptera.corpus-producer-major-dsi-positive.v1",
            "source_safe": True,
            "raw_pub_retained": False,
            "logical_identity_count": len(logical_projection),
            "dsi_positive_logical_identity_count": sum(
                1 for row in logical_projection if row["dsi_present"]
            ),
            "dsi_absent_logical_identity_count": sum(
                1 for row in logical_projection if not row["dsi_present"]
            ),
            "unique_dsi_hash_count": len(unique_rows),
            "unique_dsi_state_counts": dict(sorted(state_counts.items())),
            "logical_state_counts": dict(sorted(logical_state_counts.items())),
            "valid_piddsi_logical_identity_count": valid_logical,
            "producer_major_logical_counts": dict(
                sorted(producer_major_counts.items(), key=lambda kv: int(kv[0]))
            ),
            "cross_revision_dsi_hash_count": len(cross_revision),
            "known_anchor_hashes_checked": KNOWN_MAJOR_BY_DSI,
            "base_source_union_count": base_meta.get("union_count"),
            "post_m1_raw_count": len(post_index),
            "interpretation_boundary": (
                "producer_major is the numeric PIDDSI_VERSION high word only; "
                "serialization family/revision and distribution media remain independent axes"
            ),
        }

        (args.out / "unique_dsi_rows.json").write_text(
            json.dumps(unique_rows, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        (args.out / "logical_projection.json").write_text(
            json.dumps(logical_projection, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        (args.out / "cross_revision_dsi_hashes.json").write_text(
            json.dumps(cross_revision, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        (args.out / "summary.json").write_text(
            json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8"
        )

        print(json.dumps(summary, indent=2, ensure_ascii=False, sort_keys=True))

        # Acceptance: preserve bounded census denominator. PID absence/malformed is allowed
        # as a research result, but raw acquisition failures are not.
        if summary["logical_identity_count"] != 1314:
            raise ValueError(summary)
        if summary["dsi_positive_logical_identity_count"] != 551:
            raise ValueError(summary)
        if summary["dsi_absent_logical_identity_count"] != 763:
            raise ValueError(summary)
        if summary["unique_dsi_hash_count"] != 98:
            raise ValueError(summary)
        if state_counts.get("rehydrate_failure", 0):
            failures = [row for row in unique_rows if row.get("state") == "rehydrate_failure"]
            raise ValueError(
                f"unresolved unique DSI hashes: {len(failures)}; "
                f"first={[(x['dsi_sha256'], x.get('errors', [])[:1]) for x in failures[:5]]}"
            )

    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    return execute(args)


if __name__ == "__main__":
    raise SystemExit(main())
