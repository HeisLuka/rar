#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import tempfile
from collections import defaultdict
from pathlib import Path

HEX64 = re.compile(r"[0-9a-f]{64}")
SCHEMA = "chaptera.container-resume-reconcile.v2"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def require_sha(value: object, label: str) -> str:
    sha = str(value or "").lower()
    if not HEX64.fullmatch(sha):
        raise ValueError(f"{label} must be lowercase SHA-256")
    return sha


def container_sha_rows(rows: list[dict]) -> dict[str, list[dict]]:
    by_sha: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        if row.get("row_kind") != "container_member":
            continue
        if row.get("classification") != "cfb_publisher_hint":
            continue
        sha = require_sha(row.get("sha256"), "container member sha256")
        by_sha[sha].append(row)
    return dict(by_sha)


def crossarm_shas(payload: dict) -> set[str]:
    rows = payload.get("rows")
    if not isinstance(rows, list):
        raise ValueError("cross-arm rows must be a list")
    out = {
        require_sha(row.get("sha256"), "cross-arm sha256")
        for row in rows
        if isinstance(row, dict)
    }
    declared = int(payload.get("delta_sha_count", -1))
    if declared != len(out):
        raise ValueError(
            f"cross-arm delta count mismatch: declared {declared}, found {len(out)}"
        )
    return out


def reconcile(
    baseline_by_sha: dict[str, dict],
    container_by_sha: dict[str, list[dict]],
    crossarm: set[str],
) -> tuple[dict, list[dict]]:
    baseline = set(baseline_by_sha)
    container = set(container_by_sha)
    first_wave = {
        sha
        for sha, item in baseline_by_sha.items()
        if "container_first_wave" in set(item.get("sources") or [])
    }
    if len(baseline) != 950:
        raise ValueError(f"expected baseline 950 SHA, got {len(baseline)}")
    if len(first_wave) != 425:
        raise ValueError(f"expected container first wave 425 SHA, got {len(first_wave)}")
    if len(crossarm) != 129:
        raise ValueError(f"expected cross-arm delta 129 SHA, got {len(crossarm)}")
    if len(container) != 869:
        raise ValueError(
            f"expected 869 unique Publisher-CFB container SHA, got {len(container)}"
        )
    if not first_wave <= container:
        missing = sorted(first_wave - container)
        raise ValueError(f"full container union lost {len(missing)} first-wave SHA")
    if baseline & crossarm:
        raise ValueError("cross-arm delta overlaps exact baseline950")

    overlap_baseline = container & baseline
    overlap_crossarm = container & crossarm
    additional_beyond_first_wave = container - first_wave
    additional_overlap_other_baseline = additional_beyond_first_wave & (baseline - first_wave)
    net_new = container - baseline - crossarm

    delta_rows = []
    for sha in sorted(net_new):
        observations = container_by_sha[sha]
        provenance = []
        for row in observations:
            container_url = (
                row.get("container_url")
                or row.get("source_url")
                or row.get("parent_url")
                or row.get("url")
            )
            root_sha256 = (
                row.get("root_sha256")
                or row.get("parent_sha256")
                or row.get("container_sha256")
            )
            provenance.append(
                {
                    # Compatibility aliases retained for v1 consumers.
                    "source_url": container_url,
                    "parent_sha256": root_sha256,
                    # Canonical v2 container locator chain.
                    "source_page": row.get("source_page"),
                    "container_url": container_url,
                    "container_final_url": row.get("container_final_url"),
                    "container_filename": row.get("container_filename"),
                    "root_sha256": root_sha256,
                    "root_size_bytes": row.get("root_size_bytes"),
                    "container_depth": row.get("container_depth"),
                    "parent_member": row.get("parent_member"),
                    "parent_member_sha256": row.get("parent_member_sha256"),
                    "archive_member": row.get("archive_member")
                    or row.get("member_path"),
                    "size_bytes": row.get("size_bytes"),
                }
            )
        delta_rows.append(
            {
                "sha256": sha,
                "observation_count": len(observations),
                "provenance": provenance,
            }
        )

    summary = {
        "schema": SCHEMA,
        "baseline_sha_count": len(baseline),
        "crossarm_delta_sha_count": len(crossarm),
        "current_rar_union_sha_count": len(baseline | crossarm),
        "container_publisher_unique_sha_count": len(container),
        "container_first_wave_sha_count": len(first_wave),
        "container_overlap_baseline_sha_count": len(overlap_baseline),
        "container_overlap_crossarm_sha_count": len(overlap_crossarm),
        "container_additional_beyond_first_wave_sha_count": len(additional_beyond_first_wave),
        "container_additional_overlap_other_baseline_sha_count": len(
            additional_overlap_other_baseline
        ),
        "container_net_new_sha_count": len(net_new),
        "rar_union_after_container_sha_count": len(baseline | crossarm | container),
    }
    return summary, delta_rows


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sources", type=Path, required=True)
    ap.add_argument("--container-artifact-id", type=int, required=True)
    ap.add_argument("--container-artifact-sha256", required=True)
    ap.add_argument("--crossarm-artifact-id", type=int, required=True)
    ap.add_argument("--crossarm-artifact-sha256", required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    from structural_novelty import download_artifact, load_ledger, safe_extract_zip

    args.out.mkdir(parents=True, exist_ok=True)
    token = os.environ.get("GITHUB_TOKEN", "")
    with tempfile.TemporaryDirectory(prefix="rar-container-reconcile-") as td:
        work = Path(td)
        baseline_by_sha, baseline_summary = load_ledger(
            args.sources, work / "baseline"
        )

        container_zip = work / "container.zip"
        download_artifact(
            "HeisLuka/rar", args.container_artifact_id, token, container_zip
        )
        if sha256_file(container_zip) != args.container_artifact_sha256:
            raise ValueError("container artifact digest mismatch")
        container_dir = work / "container"
        container_dir.mkdir()
        safe_extract_zip(container_zip, container_dir)
        rows = load_json(container_dir / "manifest.json")
        if not isinstance(rows, list):
            raise ValueError("container manifest must be a list")
        all_member_shas = {
            require_sha(row.get("sha256"), "container member sha256")
            for row in rows
            if row.get("row_kind") == "container_member" and row.get("sha256")
        }
        if len(all_member_shas) != 960:
            raise ValueError(
                f"expected 960 unique all-class container member SHA, got {len(all_member_shas)}"
            )
        classification_unique: dict[str, set[str]] = defaultdict(set)
        for row in rows:
            if row.get("row_kind") != "container_member" or not row.get("sha256"):
                continue
            classification_unique[str(row.get("classification") or "unclassified")].add(
                require_sha(row.get("sha256"), "container member sha256")
            )
        container_by_sha = container_sha_rows(rows)

        crossarm_zip = work / "crossarm.zip"
        download_artifact(
            "HeisLuka/rar", args.crossarm_artifact_id, token, crossarm_zip
        )
        if sha256_file(crossarm_zip) != args.crossarm_artifact_sha256:
            raise ValueError("cross-arm artifact digest mismatch")
        crossarm_dir = work / "crossarm"
        crossarm_dir.mkdir()
        safe_extract_zip(crossarm_zip, crossarm_dir)
        crossarm_payload = load_json(crossarm_dir / "cross-arm-union.json")
        crossarm = crossarm_shas(crossarm_payload)

        summary, delta = reconcile(baseline_by_sha, container_by_sha, crossarm)
        summary["container_all_member_unique_sha_count"] = len(all_member_shas)
        summary["container_unique_sha_by_classification"] = {
            key: len(value) for key, value in sorted(classification_unique.items())
        }
        summary["baseline_source_counts"] = baseline_summary["source_counts"]
        summary["source_artifacts"] = {
            "container_union": {
                "artifact_id": args.container_artifact_id,
                "sha256": args.container_artifact_sha256,
            },
            "crossarm_union": {
                "artifact_id": args.crossarm_artifact_id,
                "sha256": args.crossarm_artifact_sha256,
            },
        }
        (args.out / "summary.json").write_text(
            json.dumps(summary, indent=2, sort_keys=True), encoding="utf-8"
        )
        (args.out / "container-net-new.json").write_text(
            json.dumps(delta, indent=2, sort_keys=True), encoding="utf-8"
        )
        print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
