#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import zipfile
from collections import Counter, defaultdict
from pathlib import Path

OUT = Path("tools/corpus/receipts/logical-stream-duplicate-locators-2026-09-24.json")
A425 = 10825119604
A407 = 10826764921
EXPECTED_SHA = "9f50a03751b1d4e86df414101d9edfec4177d1d4a845eceac441f347e6416cde"

ROOT_IDS = {
    "https://archive.org/download/PUB40CD/PUB_40_CD.ISO": "pub40cd",
    "https://archive.org/download/MSPublisher97ThaiEdition/Publisher97Thai.iso": "thai",
    "https://archive.org/download/MSPublisher97-EN/PUB_40_CD.iso": "en",
    "https://archive.org/download/microsoft-publisher-97-cd-deluxe/microsoft-publisher-97-cd-deluxe.iso": "deluxe",
}
PUB40 = "https://archive.org/download/PUB40CD/PUB_40_CD.ISO"

def fetch_artifact(artifact_id: int, dest: Path) -> Path:
    dest.mkdir(parents=True, exist_ok=True)
    z = dest / "artifact.zip"
    with z.open("wb") as fh:
        subprocess.run(
            ["gh", "api", "-H", "Accept: application/vnd.github+json",
             f"/repos/{os.environ['GITHUB_REPOSITORY']}/actions/artifacts/{artifact_id}/zip"],
            check=True, stdout=fh,
        )
    with zipfile.ZipFile(z) as arc:
        arc.extractall(dest / "unpacked")
    return dest / "unpacked" / "fingerprints.json"

def rec(row: dict) -> dict:
    return {
        "root": ROOT_IDS[row["container_url"]],
        "member": row["archive_member"],
        "sha256": row["sha256"],
        "size": row["byte_len"],
        "filename": row["filenames"][0],
    }

def main() -> int:
    tmp = Path(".tmp-logical-dup-receipt")
    rows = []
    for aid, name in [(A425, "c425"), (A407, "c407")]:
        fp = fetch_artifact(aid, tmp / name)
        payload = json.loads(fp.read_text(encoding="utf-8"))
        rows.extend(payload)

    roots = {}
    for url, rid in ROOT_IDS.items():
        matched = [r for r in rows if r.get("container_url") == url]
        if not matched:
            raise SystemExit(f"missing root rows: {url}")
        shas = {r["root_sha256"] for r in matched}
        if len(shas) != 1:
            raise SystemExit(f"root SHA ambiguity: {url}")
        roots[rid] = {"url": url, "sha256": next(iter(shas))}

    grouped = defaultdict(list)
    for row in rows:
        grouped[row["content_topology_fingerprint_sha256"]].append(row)

    groups = []
    for identity, members in grouped.items():
        if len(members) <= 1:
            continue
        urls = {m["container_url"] for m in members}
        if PUB40 not in urls or not urls.issubset(ROOT_IDS):
            continue
        if len(urls) != len(members):
            raise SystemExit(f"duplicate root inside identity: {identity}")
        anchor = next(m for m in members if m["container_url"] == PUB40)
        copies = sorted(
            [rec(m) for m in members if m is not anchor],
            key=lambda x: (x["root"], x["filename"].casefold(), x["sha256"]),
        )
        groups.append({
            "logical_identity": identity,
            "anchor": rec(anchor),
            "copies": copies,
        })

    groups.sort(key=lambda g: g["logical_identity"])
    out = {
        "schema": "chaptera.logical-stream-duplicate-locators.v1",
        "successful_sha_population": 1471,
        "logical_identity_count": 1311,
        "group_count": len(groups),
        "comparison_count": sum(len(g["copies"]) for g in groups),
        "collapsed_physical_sha": sum(len(g["copies"]) for g in groups),
        "roots": dict(sorted(roots.items())),
        "groups": groups,
    }
    raw = (json.dumps(out, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()
    digest = hashlib.sha256(raw).hexdigest()
    sizes = Counter(1 + len(g["copies"]) for g in groups)
    media = Counter(c["root"] for g in groups for c in g["copies"])
    if len(groups) != 115 or out["comparison_count"] != 160:
        raise SystemExit("census drift")
    if sizes != Counter({2: 70, 3: 45}):
        raise SystemExit(f"group-size drift: {sizes}")
    if media != Counter({"thai": 102, "en": 45, "deluxe": 13}):
        raise SystemExit(f"media drift: {media}")
    if digest != EXPECTED_SHA:
        raise SystemExit(f"receipt SHA drift: {digest}")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(raw)
    print(json.dumps({"groups":115,"comparisons":160,"sha256":digest}, sort_keys=True))
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
