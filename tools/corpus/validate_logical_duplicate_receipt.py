#!/usr/bin/env python3
from __future__ import annotations

import gzip
import hashlib
import json
import re
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
RECEIPT = HERE / "receipts" / "logical-stream-duplicate-locators-2026-09-24.json.gz"
EXPECTED_INNER_SHA256 = "233bdffb79d7389148b1ae7f89318ada6553abc981c0fc488b975d71ca1c92f3"
PUB40_URL = "https://archive.org/download/PUB40CD/PUB_40_CD.ISO"
HEX64 = re.compile(r"^[0-9a-f]{64}$")


def load() -> dict:
    raw = gzip.decompress(RECEIPT.read_bytes())
    digest = hashlib.sha256(raw).hexdigest()
    if digest != EXPECTED_INNER_SHA256:
        raise ValueError(f"inner receipt SHA drift: {digest}")
    return json.loads(raw)


def main() -> int:
    d = load()
    if d.get("schema") != "chaptera.logical-stream-duplicate-locators.v1":
        raise ValueError("schema drift")
    if d.get("group_count") != 115 or len(d.get("groups", [])) != 115:
        raise ValueError("group count drift")
    if d.get("comparison_count") != 160:
        raise ValueError("comparison count drift")
    if d.get("successful_sha_population") != 1471:
        raise ValueError("successful population drift")
    if d.get("logical_identity_count") != 1311:
        raise ValueError("logical identity count drift")
    if d.get("collapsed_physical_sha") != 160:
        raise ValueError("collapsed physical count drift")
    roots = d.get("roots", {})
    if len(roots) != 4:
        raise ValueError("expected exactly four root media")
    pub40_ids = [rid for rid, spec in roots.items() if spec.get("url") == PUB40_URL]
    if len(pub40_ids) != 1:
        raise ValueError("PUB40 anchor root ambiguity")
    pub40_id = pub40_ids[0]
    for rid, spec in roots.items():
        if not HEX64.fullmatch(str(spec.get("sha256", ""))):
            raise ValueError(f"bad root SHA: {rid}")
        if not str(spec.get("url", "")).startswith("https://archive.org/"):
            raise ValueError(f"unexpected root URL: {rid}")

    seen_identities: set[str] = set()
    seen_sha: set[str] = set()
    media = Counter()
    copy_count = 0
    group_sizes = Counter()
    for group in d["groups"]:
        identity = str(group.get("logical_identity", ""))
        if not HEX64.fullmatch(identity) or identity in seen_identities:
            raise ValueError("bad/duplicate logical identity")
        seen_identities.add(identity)
        anchor = group.get("anchor", {})
        if anchor.get("root") != pub40_id:
            raise ValueError("non-PUB40 anchor")
        members = [anchor, *group.get("copies", [])]
        if len(members) not in (2, 3):
            raise ValueError("unexpected group size")
        group_sizes[len(members)] += 1
        anchor_size = anchor.get("size")
        anchor_name = str(anchor.get("filename", "")).casefold()
        for index, member in enumerate(members):
            sha = str(member.get("sha256", ""))
            if not HEX64.fullmatch(sha) or sha in seen_sha:
                raise ValueError("bad/duplicate physical SHA")
            seen_sha.add(sha)
            if member.get("root") not in roots:
                raise ValueError("unknown root")
            if not member.get("member"):
                raise ValueError("missing archive member")
            if member.get("size") != anchor_size or int(anchor_size or 0) <= 0:
                raise ValueError("file-size identity drift")
            if str(member.get("filename", "")).casefold() != anchor_name:
                raise ValueError("filename casefold identity drift")
            if index:
                media[member["root"]] += 1
                copy_count += 1

    if group_sizes != Counter({2: 70, 3: 45}):
        raise ValueError(f"group-size distribution drift: {dict(group_sizes)}")
    if copy_count != 160 or len(seen_sha) != 275:
        raise ValueError("physical comparison population drift")
    if sorted(media.values()) != [13, 45, 102]:
        raise ValueError(f"target-media distribution drift: {dict(media)}")

    print(json.dumps({
        "schema": d["schema"],
        "groups": len(d["groups"]),
        "comparisons": copy_count,
        "physical_sha": len(seen_sha),
        "group_sizes": dict(sorted(group_sizes.items())),
        "target_media_counts": dict(sorted(media.items())),
        "inner_sha256": EXPECTED_INNER_SHA256,
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
