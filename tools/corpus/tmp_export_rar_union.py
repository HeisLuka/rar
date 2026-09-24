#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import structural_novelty as sn  # type: ignore
import pub_container_extract as ce  # type: ignore

SOURCES = HERE / "structural_novelty_sources.json"
CFB_MAGIC = bytes.fromhex("D0CF11E0A1B11AE1")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def clean_name(name: str) -> str:
    name = Path(name or "candidate.pub").name
    name = re.sub(r"[^A-Za-z0-9._ ()+\-]+", "_", name).strip(" ._") or "candidate.pub"
    if not name.lower().endswith(".pub"):
        name += ".pub"
    return name[:140]


def emit(out: Path, expected_sha: str, data: bytes, names: list[str], source: str, detail: dict) -> dict:
    actual = sha256(data)
    if actual != expected_sha:
        raise ValueError(f"SHA mismatch expected={expected_sha} actual={actual}")
    if not data.startswith(CFB_MAGIC):
        raise ValueError(f"not CFB: {expected_sha}")
    preferred = clean_name(names[0] if names else "candidate.pub")
    target = out / "files" / f"{expected_sha}__{preferred}"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    return {
        "sha256": expected_sha,
        "size_bytes": len(data),
        "file": str(target.relative_to(out)),
        "source": source,
        "names": names,
        "detail": detail,
    }


def direct(args: argparse.Namespace) -> int:
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    records = []
    failures = []
    with tempfile.TemporaryDirectory(prefix="rar-export-direct-") as td:
        ledger, meta = sn.load_ledger(SOURCES, Path(td))
        items = [ledger[k] for k in sorted(ledger)]
        nondeferred = [item for item in items if not item["deferred"]]
        if meta["union_count"] != 950 or len(nondeferred) != 525:
            raise RuntimeError(f"authority drift union={meta['union_count']} nondeferred={len(nondeferred)}")
        selected = [item for idx, item in enumerate(nondeferred) if idx % args.shard_count == args.shard]
        for item in selected:
            expected = item["sha256"]
            errors = []
            wrote = False
            for source, row in item["rows"]:
                if source == "container_first_wave":
                    continue
                try:
                    data = sn.rehydrate(source, row, args.timeout, args.max_bytes, args.max_archive_bytes)
                    rec = emit(out, expected, data, item.get("filenames", []), source, {
                        "candidate_filename": row.get("candidate_filename", ""),
                        "source_page": row.get("source_page", ""),
                        "direct_url": row.get("direct_url", ""),
                        "archive_member": row.get("archive_member", ""),
                    })
                    records.append(rec)
                    wrote = True
                    print(f"OK {expected} {source} {rec['size_bytes']}")
                    break
                except Exception as exc:
                    errors.append(f"{source}:{type(exc).__name__}:{exc}")
            if not wrote:
                failures.append({"sha256": expected, "sources": item.get("sources", []), "errors": errors})
                print(f"FAIL {expected} :: {' | '.join(errors)}", file=sys.stderr)

    records.sort(key=lambda r: r["sha256"])
    failures.sort(key=lambda r: r["sha256"])
    (out / "index.json").write_text(json.dumps(records, indent=2, ensure_ascii=False), encoding="utf-8")
    summary = {
        "mode": "direct",
        "shard": args.shard,
        "shard_count": args.shard_count,
        "selected": len(records) + len(failures),
        "ok": len(records),
        "failed": len(failures),
        "failures": failures,
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0 if not failures else 2


def containers(args: argparse.Namespace) -> int:
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    records = []
    failures = []
    token = os.environ.get("GITHUB_TOKEN", "")
    with tempfile.TemporaryDirectory(prefix="rar-export-container-") as td_raw:
        td = Path(td_raw)
        zip_path = td / "container-source.zip"
        sn.download_artifact("HeisLuka/rar", 10789500808, token, zip_path)
        src = td / "src"
        src.mkdir()
        sn.safe_extract_zip(zip_path, src)
        rows = sn.complete_cfb_rows(src)
        rows = [r for r in rows if r.get("row_kind") == "container_member"]
        unique_sha = {str(r.get("sha256", "")).lower() for r in rows}
        if len(unique_sha) != 425:
            raise RuntimeError(f"container authority drift: {len(unique_sha)} unique SHA")
        urls = sorted({str(r.get("container_url", "")).strip() for r in rows if r.get("container_url")})
        if len(urls) != 3:
            raise RuntimeError(f"expected 3 source containers, got {len(urls)}")
        if not (0 <= args.container_index < len(urls)):
            raise RuntimeError("bad container index")
        url = urls[args.container_index]
        selected = [r for r in rows if str(r.get("container_url", "")).strip() == url]
        archive = td / f"container-{args.container_index}.bin"
        meta = ce.fetch(url, archive, args.timeout, args.max_container_bytes)
        expected_root = {str(r.get("root_sha256", "")).lower() for r in selected if r.get("root_sha256")}
        if len(expected_root) != 1 or meta["sha256"].lower() not in expected_root:
            raise RuntimeError(f"container SHA mismatch expected={sorted(expected_root)} actual={meta['sha256']}")
        seen = set()
        for r in selected:
            expected = str(r["sha256"]).lower()
            if expected in seen:
                continue
            seen.add(expected)
            member = str(r.get("archive_member", ""))
            tmp_member = td / f"member-{expected}.pub"
            try:
                ce.extract_member(archive, member, tmp_member, args.command_timeout, args.max_bytes)
                data = tmp_member.read_bytes()
                rec = emit(out, expected, data, [Path(member).name], "container_first_wave", {
                    "container_url": url,
                    "container_sha256": meta["sha256"],
                    "archive_member": member,
                })
                records.append(rec)
                print(f"OK {expected} {member} {rec['size_bytes']}")
            except Exception as exc:
                failures.append({"sha256": expected, "archive_member": member, "error": f"{type(exc).__name__}:{exc}"})
                print(f"FAIL {expected} {member}: {exc}", file=sys.stderr)
            finally:
                tmp_member.unlink(missing_ok=True)

    records.sort(key=lambda r: r["sha256"])
    failures.sort(key=lambda r: r["sha256"])
    (out / "index.json").write_text(json.dumps(records, indent=2, ensure_ascii=False), encoding="utf-8")
    summary = {
        "mode": "container",
        "container_index": args.container_index,
        "container_url": url,
        "selected_unique_sha": len(records) + len(failures),
        "ok": len(records),
        "failed": len(failures),
        "failures": failures,
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0 if not failures else 2


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="mode", required=True)
    d = sub.add_parser("direct")
    d.add_argument("--shard", type=int, required=True)
    d.add_argument("--shard-count", type=int, default=4)
    d.add_argument("--out", type=Path, required=True)
    d.add_argument("--timeout", type=float, default=45)
    d.add_argument("--max-bytes", type=int, default=100 * 1024 * 1024)
    d.add_argument("--max-archive-bytes", type=int, default=160 * 1024 * 1024)
    d.set_defaults(func=direct)

    c = sub.add_parser("containers")
    c.add_argument("--container-index", type=int, required=True)
    c.add_argument("--out", type=Path, required=True)
    c.add_argument("--timeout", type=float, default=180)
    c.add_argument("--command-timeout", type=int, default=120)
    c.add_argument("--max-bytes", type=int, default=100 * 1024 * 1024)
    c.add_argument("--max-container-bytes", type=int, default=400 * 1024 * 1024)
    c.set_defaults(func=containers)

    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
