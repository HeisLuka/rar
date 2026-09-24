#!/usr/bin/env python3
import json
from pathlib import Path

p = Path(__file__).resolve().parent / "receipts" / "corpus-authority-lower-bound-2026-09-24.json"
r = json.loads(p.read_text(encoding="utf-8"))

assert r["historical_complete_authority_count"] == 339
assert r["historical_exact_identities_rematerialized"] == 316
missing = r["historical_unrematerialized_complete_identities"]
assert missing == 23
assert sum(r["historical_unrematerialized_scope"]["Geo_Core_v3"].values()) == missing

rar = r["rar_exact_union_sha_list"]["count"]
overlap_known = r["rar_overlap_with_rematerialized_baseline"]
lower = r["historical_complete_authority_count"] + rar - overlap_known - missing
upper = r["historical_complete_authority_count"] + rar - overlap_known

assert lower == r["current_complete_cfb_union"]["lower_bound"] == 1216
assert upper == r["current_complete_cfb_union"]["upper_bound"] == 1239
assert lower >= r["milestone"]["target_unique_complete_publisher_cfb"]
assert r["milestone"]["status"] == "proven_by_lower_bound"

source_counts = {k: v["count"] for k, v in r["source_sets"].items()}
assert source_counts == {
    "common_crawl": 370,
    "wayback": 70,
    "internet_archive": 39,
    "container_first_wave": 425,
    "govdocs1": 1,
    "forum_support": 2,
    "github_history": 29,
    "positive_domain_probe": 21,
}
print("authority lower-bound receipt: OK")
