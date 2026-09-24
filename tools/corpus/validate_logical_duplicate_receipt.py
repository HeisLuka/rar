#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import re
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
RECEIPT = HERE / "receipts" / "logical-stream-duplicate-locators-2026-09-24.json"
EXPECTED_SHA256 = "9f50a03751b1d4e86df414101d9edfec4177d1d4a845eceac441f347e6416cde"
PUB40_URL = "https://archive.org/download/PUB40CD/PUB_40_CD.ISO"
HEX64 = re.compile(r"^[0-9a-f]{64}$")

def load() -> dict:
    raw = RECEIPT.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    if digest != EXPECTED_SHA256:
        raise ValueError(f"receipt SHA drift: {digest}")
    return json.loads(raw)

def main() -> int:
    d = load()
    if d.get("schema") != "chaptera.logical-stream-duplicate-locators.v1":
        raise ValueError("schema drift")
    if d.get("group_count") != 115 or len(d.get("groups", [])) != 115:
        raise ValueError("group count drift")
    if d.get("comparison_count") != 160:
        raise ValueError("comparison count drift")
    if d.get("successful_sha_population") != 1471 or d.get("logical_identity_count") != 1311:
        raise ValueError("population drift")
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
    seen_identities=set(); seen_sha=set(); media=Counter(); group_sizes=Counter(); copies=0
    for group in d["groups"]:
        ident=str(group.get("logical_identity",""))
        if not HEX64.fullmatch(ident) or ident in seen_identities:
            raise ValueError("bad/duplicate logical identity")
        seen_identities.add(ident)
        anchor=group["anchor"]
        if anchor.get("root") != pub40_id:
            raise ValueError("non-PUB40 anchor")
        members=[anchor,*group.get("copies",[])]
        if len(members) not in (2,3):
            raise ValueError("unexpected group size")
        group_sizes[len(members)] += 1
        anchor_size=anchor.get("size")
        anchor_name=str(anchor.get("filename","")).casefold()
        for i,m in enumerate(members):
            sha=str(m.get("sha256",""))
            if not HEX64.fullmatch(sha) or sha in seen_sha:
                raise ValueError("bad/duplicate physical SHA")
            seen_sha.add(sha)
            if m.get("root") not in roots or not m.get("member"):
                raise ValueError("bad locator")
            if m.get("size") != anchor_size or int(anchor_size or 0) <= 0:
                raise ValueError("file-size identity drift")
            if str(m.get("filename","")).casefold() != anchor_name:
                raise ValueError("filename casefold identity drift")
            if i:
                media[m["root"]] += 1; copies += 1
    if group_sizes != Counter({2:70,3:45}):
        raise ValueError(f"group-size drift: {dict(group_sizes)}")
    if copies != 160 or len(seen_sha) != 275:
        raise ValueError("physical comparison population drift")
    if media != Counter({"thai":102,"en":45,"deluxe":13}):
        raise ValueError(f"target-media distribution drift: {dict(media)}")
    print(json.dumps({"groups":115,"comparisons":160,"physical_sha":275,"sha256":EXPECTED_SHA256}, sort_keys=True))
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
