#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import tempfile
import urllib.request
import zipfile
from pathlib import Path

import olefile

from structural_novelty import download_artifact, load_ledger, safe_extract_zip
from container_resume_reconcile import (
    container_sha_rows,
    crossarm_shas,
    sha256_file,
)

REPO = "HeisLuka/rar"
PREDECESSOR_COUNT = 1486
PREDECESSOR_DIGEST = "885eb9dad74f72617f00d7f9c113c8d9f44c295bc14e5e53c525d0e01e91c683"
BASELINE_COUNT = 950
CROSSARM_COUNT = 129
CONTAINER_PUBLISHER_COUNT = 869
EXPECTED_OBSERVATIONS = 33
EXPECTED_NEW_UNIQUE = 29
MAX_PACKAGE_BYTES = 512 * 1024 * 1024
UA = "rar-t720-corpus-register/1.0"


def canonical_digest(shas: set[str] | list[str]) -> str:
    payload = ("\n".join(sorted(shas)) + "\n").encode("ascii")
    return hashlib.sha256(payload).hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fetch_bytes(url: str, max_bytes: int = MAX_PACKAGE_BYTES) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "*/*"})
    with urllib.request.urlopen(req, timeout=120) as resp:
        data = resp.read(max_bytes + 1)
    if len(data) > max_bytes:
        raise ValueError(f"download exceeds {max_bytes} bytes: {url}")
    return data


def validate_pub(data: bytes, expected_size: int, expected_sha: str, expected_revision: int) -> None:
    if len(data) != expected_size:
        raise ValueError(f"size mismatch: expected {expected_size}, got {len(data)}")
    actual = sha256_bytes(data)
    if actual != expected_sha:
        raise ValueError(f"SHA mismatch: expected {expected_sha}, got {actual}")
    if data[:8] != bytes.fromhex("d0cf11e0a1b11ae1"):
        raise ValueError("payload is not CFB")
    with olefile.OleFileIO(io.BytesIO(data)) as ole:
        if not ole.exists("Contents"):
            raise ValueError("CFB has no Contents stream")
        contents = ole.openstream("Contents").read(4)
    if len(contents) < 4:
        raise ValueError("Contents is too short")
    revision = int.from_bytes(contents[2:4], "little")
    if revision != expected_revision:
        raise ValueError(
            f"Contents revision mismatch: expected {expected_revision}, got {revision}"
        )


def reconstruct_predecessor(
    sources: Path,
    container_artifact_id: int,
    container_artifact_sha256: str,
    crossarm_artifact_id: int,
    crossarm_artifact_sha256: str,
    work: Path,
) -> set[str]:
    token = os.environ.get("GITHUB_TOKEN", "")
    baseline_by_sha, _ = load_ledger(sources, work / "baseline")
    baseline = set(baseline_by_sha)
    if len(baseline) != BASELINE_COUNT:
        raise ValueError(f"baseline count mismatch: {len(baseline)}")

    container_zip = work / "container.zip"
    download_artifact(REPO, container_artifact_id, token, container_zip)
    if sha256_file(container_zip) != container_artifact_sha256:
        raise ValueError("container artifact digest mismatch")
    container_dir = work / "container"
    container_dir.mkdir()
    safe_extract_zip(container_zip, container_dir)
    container_rows = json.loads((container_dir / "manifest.json").read_text(encoding="utf-8"))
    container = set(container_sha_rows(container_rows))
    if len(container) != CONTAINER_PUBLISHER_COUNT:
        raise ValueError(f"container Publisher-CFB count mismatch: {len(container)}")

    crossarm_zip = work / "crossarm.zip"
    download_artifact(REPO, crossarm_artifact_id, token, crossarm_zip)
    if sha256_file(crossarm_zip) != crossarm_artifact_sha256:
        raise ValueError("cross-arm artifact digest mismatch")
    crossarm_dir = work / "crossarm"
    crossarm_dir.mkdir()
    safe_extract_zip(crossarm_zip, crossarm_dir)
    crossarm_payload = json.loads(
        (crossarm_dir / "cross-arm-union.json").read_text(encoding="utf-8")
    )
    crossarm = crossarm_shas(crossarm_payload)
    if len(crossarm) != CROSSARM_COUNT:
        raise ValueError(f"cross-arm count mismatch: {len(crossarm)}")

    predecessor = baseline | crossarm | container
    if len(predecessor) != PREDECESSOR_COUNT:
        raise ValueError(f"predecessor count mismatch: {len(predecessor)}")
    digest = canonical_digest(predecessor)
    if digest != PREDECESSOR_DIGEST:
        raise ValueError(
            f"predecessor digest mismatch: expected {PREDECESSOR_DIGEST}, got {digest}"
        )
    return predecessor


def reproduce_packages(spec: dict) -> tuple[list[dict], set[str], dict]:
    observations: list[dict] = []
    package_summaries: dict[str, dict] = {}
    package_sets: list[set[str]] = []

    packages = spec.get("packages")
    if not isinstance(packages, list) or len(packages) != 4:
        raise ValueError("input must contain exactly four packages")

    for package in packages:
        key = str(package["key"])
        expected_rows = package.get("files")
        if not isinstance(expected_rows, list):
            raise ValueError(f"{key}: files must be a list")
        package_blob = fetch_bytes(str(package["source_download"]))
        package_sha = sha256_bytes(package_blob)

        with zipfile.ZipFile(io.BytesIO(package_blob)) as zf:
            pub_members = sorted(
                info.filename for info in zf.infolist()
                if not info.is_dir() and info.filename.lower().endswith(".pub")
            )
            expected_members = sorted(str(row["source_archive_path"]) for row in expected_rows)
            if pub_members != expected_members:
                missing = sorted(set(expected_members) - set(pub_members))
                extra = sorted(set(pub_members) - set(expected_members))
                raise ValueError(
                    f"{key}: PUB member set drift: missing={missing} extra={extra}"
                )

            package_shas: set[str] = set()
            for row in expected_rows:
                path = str(row["source_archive_path"])
                data = zf.read(path)
                validate_pub(
                    data,
                    int(row["size"]),
                    str(row["sha256"]),
                    int(row["contents_revision_u16_at_2"]),
                )
                sha = str(row["sha256"])
                package_shas.add(sha)
                observations.append(
                    {
                        "package_key": key,
                        "source_page": package["source_page"],
                        "source_download": package["source_download"],
                        "source_title": package["source_title"],
                        "source_published_date": package["source_published_date"],
                        "source_version_wording": package["source_version_wording"],
                        "provenance_guardrail": spec["guardrail"],
                        "package_zip_sha256": package_sha,
                        "source_archive_path": path,
                        "filename": row["filename"],
                        "size": int(row["size"]),
                        "sha256": sha,
                        "contents_revision_u16_at_2": int(row["contents_revision_u16_at_2"]),
                    }
                )

        if len(expected_rows) != int(package["expected_observation_count"]):
            raise ValueError(f"{key}: declared observation count mismatch")
        if len(package_shas) != int(package["expected_unique_sha_count"]):
            raise ValueError(f"{key}: declared unique SHA count mismatch")
        package_sets.append(package_shas)
        package_summaries[key] = {
            "observation_count": len(expected_rows),
            "unique_sha_count": len(package_shas),
            "package_zip_sha256": package_sha,
        }

    for i, left in enumerate(package_sets):
        for right in package_sets[i + 1:]:
            if left & right:
                raise ValueError("cross-package SHA overlap detected")

    unique = {row["sha256"] for row in observations}
    if len(observations) != EXPECTED_OBSERVATIONS:
        raise ValueError(f"expected {EXPECTED_OBSERVATIONS} observations, got {len(observations)}")
    if len(unique) != EXPECTED_NEW_UNIQUE:
        raise ValueError(f"expected {EXPECTED_NEW_UNIQUE} unique SHA, got {len(unique)}")
    return observations, unique, package_summaries


def write_lines(path: Path, shas: set[str]) -> None:
    path.write_text("\n".join(sorted(shas)) + "\n", encoding="ascii")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sources", type=Path, required=True)
    ap.add_argument("--input", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--container-artifact-id", type=int, required=True)
    ap.add_argument("--container-artifact-sha256", required=True)
    ap.add_argument("--crossarm-artifact-id", type=int, required=True)
    ap.add_argument("--crossarm-artifact-sha256", required=True)
    args = ap.parse_args()

    spec = json.loads(args.input.read_text(encoding="utf-8"))
    args.out.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="rar-t720-register-") as td:
        work = Path(td)
        predecessor = reconstruct_predecessor(
            args.sources,
            args.container_artifact_id,
            args.container_artifact_sha256,
            args.crossarm_artifact_id,
            args.crossarm_artifact_sha256,
            work,
        )
        observations, new_shas, package_summaries = reproduce_packages(spec)

    overlap = predecessor & new_shas
    if overlap:
        raise ValueError(f"new tranche overlaps predecessor in {len(overlap)} SHA")

    successor = predecessor | new_shas
    if len(successor) != 1515:
        raise ValueError(f"successor count mismatch: {len(successor)}")

    summary = {
        "schema": "chaptera.corpus-version-labelled-training-register-proof.v1",
        "source_safe": True,
        "raw_pub_retained": False,
        "predecessor_count": len(predecessor),
        "predecessor_digest": "sha256:" + canonical_digest(predecessor),
        "observation_count": len(observations),
        "new_unique_sha_count": len(new_shas),
        "new_tranche_digest": "sha256:" + canonical_digest(new_shas),
        "overlap_with_predecessor": len(overlap),
        "successor_count": len(successor),
        "successor_digest": "sha256:" + canonical_digest(successor),
        "packages": package_summaries,
    }

    (args.out / "proof-summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    (args.out / "reproduced-observations.json").write_text(
        json.dumps(observations, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    write_lines(args.out / "predecessor-1486.sha256.txt", predecessor)
    write_lines(args.out / "new-tranche-29.sha256.txt", new_shas)
    write_lines(args.out / "successor-1515.sha256.txt", successor)

    print(json.dumps(summary, indent=2, sort_keys=True))
    print("T720_PREDECESSOR_SHA_BEGIN")
    for sha in sorted(predecessor):
        print(sha)
    print("T720_PREDECESSOR_SHA_END")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
