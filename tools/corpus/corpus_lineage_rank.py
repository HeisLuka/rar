#!/usr/bin/env python3
"""Rank cross-cohort Publisher lineage candidates from source-free CFB fingerprints.

This is deliberately a ranking/provenance layer, not a lineage oracle. It preserves
both the canonical pair contract from structural_novelty.py (cluster size <= 30,
normalized filename similarity >= 0.65) and an explicitly separate expanded-only
set from larger structural families.
"""
from __future__ import annotations

import argparse
import json
import re
from collections import Counter, defaultdict
from difflib import SequenceMatcher
from pathlib import Path

SCHEMA = "chaptera.corpus-lineage-rank.v1"
CANONICAL_CLUSTER_MAX = 30
NAME_SIMILARITY_MIN = 0.65


def normalized_stem(name: str) -> str:
    stem = Path(name).stem.casefold()
    stem = re.sub(r"\b(?:19|20)\d{2}\b", "<year>", stem)
    stem = re.sub(r"[_\-]+", " ", stem)
    return re.sub(r"\s+", " ", stem).strip()


def best_name(row: dict) -> str:
    names = row.get("filenames") or []
    return names[0] if names else ""


def carrier_pattern(row: dict) -> str:
    flags = row.get("carrier_flags") or {}
    return "|".join(k for k, v in sorted(flags.items()) if v) or "none"


def media_id(row: dict) -> str:
    url = str(row.get("container_url") or "").strip()
    if url:
        return url
    sources = ",".join(sorted(str(x) for x in row.get("sources") or []))
    return f"source:{sources}" if sources else "source:unknown"


def parse_cohort_arg(value: str) -> tuple[str, Path]:
    if "=" not in value:
        raise argparse.ArgumentTypeError("cohort must be NAME=PATH")
    name, raw = value.split("=", 1)
    name = name.strip()
    if not name or not re.fullmatch(r"[A-Za-z0-9_.-]+", name):
        raise argparse.ArgumentTypeError(f"invalid cohort name: {name!r}")
    return name, Path(raw)


def load_rows(cohorts: list[tuple[str, Path]]) -> list[dict]:
    rows = []
    seen_sha: set[str] = set()
    for cohort, path in cohorts:
        data = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data, list):
            raise ValueError(f"cohort {cohort}: fingerprints must be a JSON list")
        for raw in data:
            if not isinstance(raw, dict) or "sha256" not in raw:
                raise ValueError(f"cohort {cohort}: malformed fingerprint row")
            sha = str(raw["sha256"])
            if sha in seen_sha:
                raise ValueError(f"SHA appears in more than one cohort: {sha}")
            seen_sha.add(sha)
            row = dict(raw)
            row["_cohort"] = cohort
            rows.append(row)
    rows.sort(key=lambda r: r["sha256"])
    return rows


def pair_score(left: dict, right: dict, similarity: float) -> tuple[int, dict, dict]:
    left_name, right_name = best_name(left), best_name(right)
    exact_name = bool(left_name and right_name and left_name.casefold() == right_name.casefold())
    same_normalized_stem = bool(
        normalized_stem(left_name)
        and normalized_stem(left_name) == normalized_stem(right_name)
    )
    left_media, right_media = media_id(left), media_id(right)
    same_media_id = left_media == right_media
    left_carrier, right_carrier = carrier_pattern(left), carrier_pattern(right)
    carrier_equal = left_carrier == right_carrier

    breakdown = {
        "structural_identity": 20,
        "cross_cohort": 15,
        "distinct_sha": 5 if left["sha256"] != right["sha256"] else 0,
        "filename": (
            40 if exact_name
            else 34 if same_normalized_stem
            else 28 if similarity >= 0.95
            else 22 if similarity >= 0.85
            else 16 if similarity >= 0.75
            else 10
        ),
        "source_media_independence": 12 if not same_media_id else 2,
        "carrier_evidence": 8 if carrier_equal else 5,
    }
    flags = {
        "exact_name": exact_name,
        "same_normalized_stem": same_normalized_stem,
        "same_media_id": same_media_id,
        "carrier_equal": carrier_equal,
    }
    return sum(breakdown.values()), breakdown, flags


def build_candidates(rows: list[dict], cluster_max: int | None) -> list[dict]:
    clusters: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        if row.get("status") != "ok":
            continue
        fingerprint = row.get("size_bucket_fingerprint_sha256")
        if fingerprint:
            clusters[str(fingerprint)].append(row)

    pairs = []
    for fingerprint, members in clusters.items():
        if len(members) < 2 or (cluster_max is not None and len(members) > cluster_max):
            continue
        members = sorted(members, key=lambda r: r["sha256"])
        for index, left in enumerate(members):
            for right in members[index + 1 :]:
                if left["_cohort"] == right["_cohort"]:
                    continue
                a, b = normalized_stem(best_name(left)), normalized_stem(best_name(right))
                similarity = SequenceMatcher(None, a, b).ratio() if a and b else 0.0
                if similarity < NAME_SIMILARITY_MIN:
                    continue
                score, breakdown, flags = pair_score(left, right, similarity)
                pairs.append(
                    {
                        "rank_score": score,
                        "score_breakdown": breakdown,
                        "left_sha256": left["sha256"],
                        "right_sha256": right["sha256"],
                        "left_cohort": left["_cohort"],
                        "right_cohort": right["_cohort"],
                        "left_name": best_name(left),
                        "right_name": best_name(right),
                        "name_similarity": round(similarity, 4),
                        "structural_fingerprint": fingerprint,
                        "left_media_id": media_id(left),
                        "right_media_id": media_id(right),
                        "left_carrier_pattern": carrier_pattern(left),
                        "right_carrier_pattern": carrier_pattern(right),
                        "left_family_hint": left.get("family_hint", ""),
                        "right_family_hint": right.get("family_hint", ""),
                        **flags,
                        "inference": "lineage_candidate_only",
                        "next_discriminator": (
                            "version/header + source-media generation; then byte/content delta"
                            if flags["exact_name"]
                            else "source provenance + byte/content delta before semantic/native testing"
                        ),
                    }
                )

    unique = {(p["left_sha256"], p["right_sha256"]): p for p in pairs}
    out = list(unique.values())
    out.sort(
        key=lambda p: (
            -p["rank_score"],
            -p["name_similarity"],
            p["left_name"].casefold(),
            p["right_name"].casefold(),
            p["left_sha256"],
            p["right_sha256"],
        )
    )
    for rank, row in enumerate(out, 1):
        row["rank"] = rank
    return out


def connected_families(pairs: list[dict], by_sha: dict[str, dict]) -> list[dict]:
    adjacency: dict[str, set[str]] = defaultdict(set)
    for pair in pairs:
        left, right = pair["left_sha256"], pair["right_sha256"]
        adjacency[left].add(right)
        adjacency[right].add(left)

    seen: set[str] = set()
    families = []
    for start in sorted(adjacency):
        if start in seen:
            continue
        stack = [start]
        seen.add(start)
        component = []
        while stack:
            current = stack.pop()
            component.append(current)
            for other in sorted(adjacency[current]):
                if other not in seen:
                    seen.add(other)
                    stack.append(other)
        component_set = set(component)
        component_pairs = [
            p for p in pairs
            if p["left_sha256"] in component_set and p["right_sha256"] in component_set
        ]
        names = sorted(
            {best_name(by_sha[s]) for s in component if best_name(by_sha[s])},
            key=str.casefold,
        )
        families.append(
            {
                "component_size": len(component),
                "pair_count": len(component_pairs),
                "cohorts": sorted({by_sha[s]["_cohort"] for s in component}),
                "names": names,
                "sha256": sorted(component),
                "max_pair_score": max(p["rank_score"] for p in component_pairs),
                "exact_name_pair_count": sum(p["exact_name"] for p in component_pairs),
            }
        )
    families.sort(
        key=lambda f: (
            -f["max_pair_score"],
            -f["exact_name_pair_count"],
            -f["component_size"],
            f["names"][0].casefold() if f["names"] else "",
        )
    )
    for rank, family in enumerate(families, 1):
        family["family_rank"] = rank
    return families


def summarize(rows: list[dict], canonical: list[dict], expanded_only: list[dict], families: list[dict]) -> dict:
    cohort_pairs = Counter(
        tuple(sorted((p["left_cohort"], p["right_cohort"]))) for p in canonical
    )
    return {
        "schema": SCHEMA,
        "input_sha_rows": len(rows),
        "ok_sha_rows": sum(r.get("status") == "ok" for r in rows),
        "canonical_pair_candidates": len(canonical),
        "canonical_family_components": len(families),
        "canonical_exact_name_pairs": sum(p["exact_name"] for p in canonical),
        "canonical_same_normalized_stem_pairs": sum(p["same_normalized_stem"] for p in canonical),
        "canonical_carrier_equal_pairs": sum(p["carrier_equal"] for p in canonical),
        "canonical_cohort_pair_counts": {
            "__".join(key): value for key, value in sorted(cohort_pairs.items())
        },
        "expanded_only_pairs": len(expanded_only),
        "expanded_only_exact_name_pairs": sum(p["exact_name"] for p in expanded_only),
        "expanded_only_structural_fingerprints": len(
            {p["structural_fingerprint"] for p in expanded_only}
        ),
        "canonical_cluster_max": CANONICAL_CLUSTER_MAX,
        "name_similarity_min": NAME_SIMILARITY_MIN,
        "top_score": canonical[0]["rank_score"] if canonical else 0,
        "top_shortlist_count": min(40, len(canonical)),
    }


def rank(cohorts: list[tuple[str, Path]], out: Path) -> dict:
    rows = load_rows(cohorts)
    by_sha = {r["sha256"]: r for r in rows}
    canonical = build_candidates(rows, CANONICAL_CLUSTER_MAX)
    expanded = build_candidates(rows, None)
    canonical_keys = {(p["left_sha256"], p["right_sha256"]) for p in canonical}
    expanded_only = [
        p for p in expanded
        if (p["left_sha256"], p["right_sha256"]) not in canonical_keys
    ]
    for rank_no, row in enumerate(expanded_only, 1):
        row["expanded_rank"] = rank_no
    families = connected_families(canonical, by_sha)
    summary = summarize(rows, canonical, expanded_only, families)

    out.mkdir(parents=True, exist_ok=True)
    payloads = {
        "canonical_ranked_pairs.json": canonical,
        "canonical_families.json": families,
        "canonical_top_shortlist.json": canonical[:40],
        "expanded_only_pairs.json": expanded_only,
        "summary.json": summary,
    }
    for name, payload in payloads.items():
        (out / name).write_text(
            json.dumps(payload, indent=2, ensure_ascii=False), encoding="utf-8"
        )
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    return summary


def self_test() -> int:
    def row(sha: str, cohort: str, name: str, fp: str, carrier=True) -> dict:
        return {
            "sha256": sha,
            "_cohort": cohort,
            "status": "ok",
            "filenames": [name],
            "sources": [cohort],
            "size_bucket_fingerprint_sha256": fp,
            "carrier_flags": {"contents": carrier},
            "family_hint": "cfb_other",
        }

    rows = [
        row("a" * 64, "a", "CARD.PUB", "f" * 64),
        row("b" * 64, "b", "CARD.PUB", "f" * 64),
        row("c" * 64, "b", "UNRELATED.PUB", "f" * 64),
    ]
    pairs = build_candidates(rows, CANONICAL_CLUSTER_MAX)
    assert len(pairs) == 1, pairs
    assert pairs[0]["exact_name"] is True
    assert pairs[0]["rank_score"] == 100

    large = [
        row(f"{i:064x}", "a" if i % 2 == 0 else "b", "SAME.PUB", "e" * 64)
        for i in range(31)
    ]
    assert build_candidates(large, CANONICAL_CLUSTER_MAX) == []
    assert len(build_candidates(large, None)) > 0
    print("corpus lineage rank self-test ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("rank")
    r.add_argument("--cohort", action="append", type=parse_cohort_arg, required=True)
    r.add_argument("--out", type=Path, required=True)
    sub.add_parser("self-test")
    args = ap.parse_args()
    if args.cmd == "self-test":
        return self_test()
    rank(args.cohort, args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
