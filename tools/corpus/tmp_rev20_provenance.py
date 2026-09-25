#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import io
import json
import re
import struct
import urllib.request
from collections import Counter
from datetime import datetime
from pathlib import Path

import olefile

UA = "rar-t704-rev20-provenance/1.0"
DSI = "\x05DocumentSummaryInformation"
SI = "\x05SummaryInformation"
FMTID_DOCSUMMARY = bytes.fromhex("02d5cdd59c2e1b10939708002b2cf9ae")
TARGET_TYPES = {0xF010: "ClientAnchor", 0xF011: "ClientData", 0xF00D: "ClientTextbox"}

SAMPLES = {
    "target_rev20": {
        "url": "https://archive.org/download/new-microsoft-publisher-document_202502/New%20Microsoft%20Publisher%20Document.pub",
        "expected_size": 59904,
        "expected_sha256": "e050ea777d910137fff7c160992ec026ab4f76832b6c96701b114e379abf4ca3",
        "source": "Internet Archive item new-microsoft-publisher-document_202502 / exact retained T608 source manifest",
    },
    "publisher2002_control": {
        "url": "https://raw.githubusercontent.com/digital-preservation/pronom-research-week/master/MicrosoftPublisher/Sample%20Files/MSPublisher2002.PUB",
        "expected_git_blob_sha1": "2a94f0f7777b4137a7dd89a3140b2d35d45522be",
        "expected_revision": 14,
        "expected_producer_major": 10,
    },
    "publisher2003_control": {
        "url": "https://raw.githubusercontent.com/digital-preservation/pronom-research-week/master/MicrosoftPublisher/Sample%20Files/MsPublisher2003-Sample.pub",
        "expected_git_blob_sha1": "78c6c0982312eb7b56845dd5281100fddbc7b8ae",
        "expected_revision": 19,
        "expected_producer_major": 11,
    },
    "publisher2007_control": {
        "url": "https://raw.githubusercontent.com/digital-preservation/pronom-research-week/master/MicrosoftPublisher/Sample%20Files/MSPublisher2007-Sample.pub",
        "expected_git_blob_sha1": "a7417f6398749285202b0f64553fa40d56af599b",
        "expected_revision": 21,
        "expected_producer_major": 12,
    },
}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def git_blob_sha1(data: bytes) -> str:
    h = hashlib.sha1()
    h.update(f"blob {len(data)}\0".encode("ascii"))
    h.update(data)
    return h.hexdigest()


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=60) as resp:
        data = resp.read(8 * 1024 * 1024 + 1)
    if len(data) > 8 * 1024 * 1024:
        raise ValueError("sample exceeds 8 MiB cap")
    return data


def u16(data: bytes, off: int) -> int:
    return int.from_bytes(data[off:off+2], "little")


def u32(data: bytes, off: int) -> int:
    return int.from_bytes(data[off:off+4], "little")


def decode_piddsi(raw: bytes) -> dict:
    out = {"stream_len": len(raw), "stream_sha256": sha256(raw)}
    if len(raw) < 28 or raw[:2] != b"\xfe\xff":
        return {**out, "state": "malformed_header"}
    nsec = u32(raw, 24)
    hits = []
    for i in range(nsec):
        desc = 28 + 20 * i
        if desc + 20 > len(raw):
            return {**out, "state": "truncated_section_table"}
        if raw[desc:desc+16] != FMTID_DOCSUMMARY:
            continue
        sec = u32(raw, desc+16)
        if sec + 8 > len(raw):
            return {**out, "state": "section_oob"}
        size = u32(raw, sec)
        count = u32(raw, sec+4)
        end = sec + size
        if size < 8 or end > len(raw) or sec + 8 + count * 8 > end:
            return {**out, "state": "section_invalid"}
        for j in range(count):
            ent = sec + 8 + j * 8
            pid = u32(raw, ent)
            rel = u32(raw, ent+4)
            if pid != 0x17:
                continue
            voff = sec + rel
            if voff + 8 > end:
                hits.append({"state": "value_oob"})
                continue
            vt = u16(raw, voff)
            if vt != 3:
                hits.append({"state": "wrong_vt", "vt": vt})
                continue
            value = u32(raw, voff+4)
            hits.append({
                "state": "valid_vt_i4",
                "vt": vt,
                "value_u32": value,
                "value_hex": f"0x{value:08X}",
                "producer_major": (value >> 16) & 0xFFFF,
                "producer_low16": value & 0xFFFF,
            })
    if len(hits) != 1:
        return {**out, "state": "missing_or_duplicate", "hits": hits}
    return {**out, **hits[0]}


def safe_value(v):
    if isinstance(v, bytes):
        return {"kind": "bytes", "len": len(v), "sha256": sha256(v)}
    if isinstance(v, datetime):
        return v.isoformat()
    if isinstance(v, (str, int, float, bool)) or v is None:
        return v
    return {"kind": type(v).__name__, "repr": repr(v)[:200]}


def property_set(ole, name: str) -> dict:
    if not ole.exists([name]):
        return {"present": False}
    raw = ole.openstream([name]).read()
    try:
        props = ole.getproperties([name], convert_time=True)
        clean = {str(k): safe_value(v) for k, v in sorted(props.items())}
    except Exception as exc:
        clean = {"_error": f"{type(exc).__name__}:{exc}"}
    return {
        "present": True,
        "len": len(raw),
        "sha256": sha256(raw),
        "properties": clean,
    }


def extract_compobj_strings(raw: bytes) -> dict:
    ascii_s = [m.decode("latin1") for m in re.findall(rb"[ -~]{4,}", raw)]
    wide_s = []
    for m in re.findall(rb"(?:[ -~]\x00){4,}", raw):
        try:
            wide_s.append(m.decode("utf-16le"))
        except Exception:
            pass
    def uniq(xs):
        seen = set(); out = []
        for x in xs:
            x = x.strip("\x00 ")
            if x and x not in seen:
                seen.add(x); out.append(x[:300])
        return out[:40]
    return {"ascii": uniq(ascii_s), "utf16le": uniq(wide_s)}


def escher_probe(raw: bytes) -> dict:
    targets = []
    all_counts = Counter()
    errors = []

    def walk(start: int, end: int, depth: int):
        if depth > 24:
            errors.append({"offset": start, "reason": "depth_limit"})
            return
        pos = start
        while pos + 8 <= end:
            vi, typ, length = struct.unpack_from("<HHI", raw, pos)
            recver = vi & 0xF
            instance = vi >> 4
            recend = pos + 8 + length
            if recend > end or recend > len(raw):
                errors.append({
                    "offset": pos,
                    "reason": "record_oob",
                    "type_hex": f"0x{typ:04X}",
                    "declared_length": length,
                    "range_end": end,
                })
                return
            all_counts[(typ, recver, instance)] += 1
            if typ in TARGET_TYPES:
                targets.append({
                    "offset": pos,
                    "type": TARGET_TYPES[typ],
                    "type_hex": f"0x{typ:04X}",
                    "recVer": recver,
                    "recInstance": instance,
                    "payload_len": length,
                    "depth": depth,
                })
            if recver == 0xF and length:
                walk(pos + 8, recend, depth + 1)
            pos = recend
        if pos != end and any(raw[pos:end]):
            errors.append({"offset": pos, "reason": "nonzero_trailing_bytes", "count": end-pos})

    walk(0, len(raw), 0)

    # Publisher host records can occur behind host-specific payload regions that
    # are not recursively describable as generic OfficeArt containers.  Use a
    # second bounded header scan, but accept only complete in-stream records and
    # require recVer=0xA for the three Publisher host-specific record types.
    scan_targets = []
    for pos in range(0, max(0, len(raw) - 7)):
        vi, typ, length = struct.unpack_from("<HHI", raw, pos)
        if typ not in TARGET_TYPES:
            continue
        recver = vi & 0xF
        instance = vi >> 4
        recend = pos + 8 + length
        if recver != 0xA or recend > len(raw):
            continue
        scan_targets.append({
            "offset": pos,
            "type": TARGET_TYPES[typ],
            "type_hex": f"0x{typ:04X}",
            "recVer": recver,
            "recInstance": instance,
            "payload_len": length,
        })

    target_counts = Counter((r["type"], r["recVer"], r["recInstance"]) for r in targets)
    scan_counts = Counter((r["type"], r["recVer"], r["recInstance"]) for r in scan_targets)
    return {
        "stream_len": len(raw),
        "stream_sha256": sha256(raw),
        "target_records": targets,
        "target_counts": [
            {"type": k[0], "recVer": k[1], "recInstance": k[2], "count": v}
            for k, v in sorted(target_counts.items())
        ],
        "target_header_scan": scan_targets,
        "target_header_scan_counts": [
            {"type": k[0], "recVer": k[1], "recInstance": k[2], "count": v}
            for k, v in sorted(scan_counts.items())
        ],
        "parse_errors": errors,
        "parsed_record_key_count": len(all_counts),
    }


def inspect(label: str, cfg: dict, data: bytes) -> dict:
    result = {
        "label": label,
        "source": cfg.get("source") or cfg["url"],
        "size": len(data),
        "sha256": sha256(data),
        "git_blob_sha1": git_blob_sha1(data),
    }
    if "expected_size" in cfg and len(data) != cfg["expected_size"]:
        raise ValueError(f"{label}: size mismatch")
    if "expected_sha256" in cfg and result["sha256"] != cfg["expected_sha256"]:
        raise ValueError(f"{label}: sha256 mismatch")
    if "expected_git_blob_sha1" in cfg and result["git_blob_sha1"] != cfg["expected_git_blob_sha1"]:
        raise ValueError(f"{label}: git blob sha1 mismatch")

    with olefile.OleFileIO(io.BytesIO(data)) as ole:
        paths = ole.listdir(streams=True, storages=False)
        streams = []
        for path in paths:
            raw = ole.openstream(path).read()
            streams.append({
                "path": "/" + "/".join(path),
                "len": len(raw),
                "sha256": sha256(raw),
                "ctime": safe_value(ole.getctime(path)),
                "mtime": safe_value(ole.getmtime(path)),
            })
        result["streams"] = streams

        contents_path = next((p for p in paths if p and p[-1] == "Contents"), None)
        if contents_path is None:
            raise ValueError(f"{label}: Contents stream missing")
        contents = ole.openstream(contents_path).read()
        if len(contents) < 32:
            raise ValueError(f"{label}: Contents too short")
        result["contents"] = {
            "len": len(contents),
            "sha256": sha256(contents),
            "header32_hex": contents[:32].hex(),
            "family_magic_hex": contents[:4].hex(),
            "serialization_revision_u16": u16(contents, 12),
            "serialization_revision_hex": f"0x{u16(contents,12):04X}",
        }

        dsi = property_set(ole, DSI)
        if not dsi["present"]:
            result["dsi"] = dsi
        else:
            dsi_raw = ole.openstream([DSI]).read()
            result["dsi"] = {**dsi, "raw_piddsi_decode": decode_piddsi(dsi_raw)}

        result["summary_information"] = property_set(ole, SI)

        comp = next((p for p in paths if p and p[-1] == "\x01CompObj" and len(p) == 1), None)
        if comp:
            raw = ole.openstream(comp).read()
            result["compobj"] = {
                "len": len(raw),
                "sha256": sha256(raw),
                "strings": extract_compobj_strings(raw),
            }
        else:
            result["compobj"] = {"present": False}

        escher = next((p for p in paths if p and p[-1] == "EscherStm"), None)
        if escher:
            result["escher"] = escher_probe(ole.openstream(escher).read())
        else:
            result["escher"] = {"present": False}

    decoded = result.get("dsi", {}).get("raw_piddsi_decode", {})
    result["observed_producer_major"] = decoded.get("producer_major")
    result["observed_producer_low16"] = decoded.get("producer_low16")
    result["observed_revision"] = result["contents"]["serialization_revision_u16"]

    if "expected_revision" in cfg and result["observed_revision"] != cfg["expected_revision"]:
        raise ValueError(f"{label}: revision mismatch {result['observed_revision']} != {cfg['expected_revision']}")
    if "expected_producer_major" in cfg and result["observed_producer_major"] != cfg["expected_producer_major"]:
        raise ValueError(f"{label}: producer major mismatch")
    return result


def main():
    outdir = Path("out")
    outdir.mkdir(exist_ok=True)
    results = {}
    raw_paths = []
    try:
        for label, cfg in SAMPLES.items():
            data = fetch(cfg["url"])
            tmp = outdir / f"{label}.pub"
            tmp.write_bytes(data)
            raw_paths.append(tmp)
            results[label] = inspect(label, cfg, data)

        target = results["target_rev20"]
        target_instances = {
            row["recInstance"]
            for row in target.get("escher", {}).get("target_header_scan", [])
            if row["type"] in {"ClientAnchor", "ClientData", "ClientTextbox"}
        }
        controls = {
            k: {
                "revision": v["observed_revision"],
                "producer_major": v["observed_producer_major"],
                "host_recinstances": sorted({
                    r["recInstance"] for r in v.get("escher", {}).get("target_header_scan", [])
                }),
            }
            for k, v in results.items() if k != "target_rev20"
        }
        target_host_consistent = bool(target_instances) and target_instances == {20}
        summary = {
            "schema": "chaptera.corpus-rev20-provenance.v1",
            "source_safe": True,
            "raw_pub_retained": False,
            "target_sha256": target["sha256"],
            "target_size": target["size"],
            "target_revision": target["observed_revision"],
            "target_producer_major": target["observed_producer_major"],
            "target_producer_low16": target["observed_producer_low16"],
            "target_host_recinstances": sorted(target_instances),
            "target_host_revision_consistent": target_host_consistent,
            "controls": controls,
            "classification": (
                "internally_consistent_publisher_host_serialization_rev20"
                if target["observed_revision"] == 20
                and target["observed_producer_major"] == 12
                and target_host_consistent
                and controls["publisher2003_control"]["host_recinstances"] == [19]
                and controls["publisher2007_control"]["host_recinstances"] == [21]
                else "needs_further_classification"
            ),
            "marketing_release_boundary": "unresolved_without_exact_writer_provenance",
        }
        (outdir / "details.json").write_text(json.dumps(results, indent=2, ensure_ascii=False, sort_keys=True)+"\n", encoding="utf-8")
        (outdir / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False, sort_keys=True)+"\n", encoding="utf-8")
        print(json.dumps(summary, indent=2, ensure_ascii=False, sort_keys=True))
    finally:
        for p in raw_paths:
            try:
                p.unlink()
            except FileNotFoundError:
                pass
    if list(outdir.glob("*.pub")):
        raise RuntimeError("raw PUB leaked into retained output")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
