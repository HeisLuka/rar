#!/usr/bin/env python3
"""Hosted orchestration for the real Chaptera Reader open-phase harness."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import platform
import statistics
import subprocess
import tempfile

from open_first_useful_page_v1 import summarize_runs, validate_receipt

RUN_SCHEMA = "chaptera.open-first-useful-page-run.v1"
RECEIPT_VERSION = "chaptera.open-first-useful-page.v1"


def _sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _load_json(path: pathlib.Path):
    return json.loads(path.read_text(encoding="utf-8"))


def _run_one(binary: pathlib.Path, fixture: pathlib.Path, cache_state: str, output: pathlib.Path):
    subprocess.run(
        [
            str(binary),
            "--open-phase-run-v1",
            str(fixture),
            cache_state,
            str(output),
        ],
        check=True,
    )
    return _load_json(output)


def _run_series(
    binary: pathlib.Path,
    fixture: pathlib.Path,
    cache_state: str,
    count: int,
    output: pathlib.Path,
):
    subprocess.run(
        [
            str(binary),
            "--open-phase-series-v1",
            str(fixture),
            cache_state,
            str(count),
            str(output),
        ],
        check=True,
    )
    rows = _load_json(output)
    if not isinstance(rows, list) or len(rows) != count:
        raise RuntimeError("Reader warm series did not return the requested run count")
    return rows


def _validate_observation(run: dict, *, fixture_hash: str, fixture_bytes: int, cache_state: str):
    if run.get("schema_version") != RUN_SCHEMA:
        raise RuntimeError("Reader open-phase run schema mismatch")
    if run.get("cache_state") != cache_state:
        raise RuntimeError("Reader open-phase cache_state mismatch")
    if run.get("source_sha256") != fixture_hash:
        raise RuntimeError("Reader open-phase fixture hash mismatch")
    if run.get("source_bytes") != fixture_bytes:
        raise RuntimeError("Reader open-phase fixture byte length mismatch")

    definition = run.get("first_useful_page_definition") or {}
    for key in (
        "page_geometry_present",
        "fidelity_diagnostics_present",
        "visible_resources_ready",
        "current_text_layout_present",
        "non_empty_visible_content",
    ):
        if definition.get(key) is not True:
            raise RuntimeError(f"Reader first-useful-page condition is false: {key}")

    equivalence = run.get("final_equivalence") or {}
    for key in (
        "canonical_document_equal",
        "final_scene_equal",
        "final_search_projection_equal",
    ):
        if equivalence.get(key) is not True:
            raise RuntimeError(f"instrumented/plain Reader equivalence failed: {key}")

    first = float(run["first_useful_page_ms"])
    overhead = float(run.get("instrumentation_overhead_ms", 0.0))
    allowed = max(10.0, first * 0.10)
    if overhead > allowed:
        raise RuntimeError(
            f"instrumentation unattributed overhead {overhead:.3f}ms exceeds {allowed:.3f}ms"
        )


def _receipt_run(run: dict) -> dict:
    return {
        "cache_state": run["cache_state"],
        "first_useful_page_ms": run["first_useful_page_ms"],
        "fully_ready_ms": run["fully_ready_ms"],
        "first_useful_page_definition": run["first_useful_page_definition"],
        "phases": run["phases"],
        "document_global_before_first_useful_page": run[
            "document_global_before_first_useful_page"
        ],
        "instrumentation_overhead_ms": run["instrumentation_overhead_ms"],
        "instrumented_output_sha256": run["instrumented_output_sha256"],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--build-sha", required=True)
    parser.add_argument("--expected-sha256")
    parser.add_argument("--expected-byte-len", type=int)
    parser.add_argument("--cold-runs", type=int, default=3)
    parser.add_argument("--warm-runs", type=int, default=3)
    args = parser.parse_args()

    if not args.binary.is_file():
        raise SystemExit(f"Reader binary is missing: {args.binary}")
    if not args.fixture.is_file():
        raise SystemExit(f"fixture is missing: {args.fixture}")
    if not 1 <= args.cold_runs <= 16 or not 1 <= args.warm_runs <= 16:
        raise SystemExit("cold/warm run counts must be 1..=16")

    fixture_hash = _sha256(args.fixture)
    fixture_bytes = args.fixture.stat().st_size
    if args.expected_sha256 and fixture_hash != args.expected_sha256.lower():
        raise SystemExit(
            f"fixture SHA-256 mismatch: expected {args.expected_sha256}, got {fixture_hash}"
        )
    if args.expected_byte_len is not None and fixture_bytes != args.expected_byte_len:
        raise SystemExit(
            f"fixture byte length mismatch: expected {args.expected_byte_len}, got {fixture_bytes}"
        )

    with tempfile.TemporaryDirectory(prefix="chaptera-open-phase-") as temp:
        root = pathlib.Path(temp)
        raw_runs = []
        for index in range(args.cold_runs):
            raw_runs.append(
                _run_one(
                    args.binary,
                    args.fixture,
                    "cold",
                    root / f"cold-{index}.json",
                )
            )
        raw_runs.extend(
            _run_series(
                args.binary,
                args.fixture,
                "warm",
                args.warm_runs,
                root / "warm.json",
            )
        )

    for run in raw_runs:
        _validate_observation(
            run,
            fixture_hash=fixture_hash,
            fixture_bytes=fixture_bytes,
            cache_state=run["cache_state"],
        )

    page_counts = {run["page_count"] for run in raw_runs}
    runtime_identities = {
        json.dumps(run["runtime_identity"], sort_keys=True) for run in raw_runs
    }
    if len(page_counts) != 1 or len(runtime_identities) != 1:
        raise RuntimeError("Reader run identity drifted across samples")

    runs = [_receipt_run(run) for run in raw_runs]
    overheads = [float(run["instrumentation_overhead_ms"]) for run in raw_runs]
    output_hashes = {run["instrumented_output_sha256"] for run in raw_runs}
    if len(output_hashes) != 1:
        raise RuntimeError("instrumented Reader output hash drifted across repeated opens")

    runtime_identity = raw_runs[0]["runtime_identity"]
    receipt = {
        "receipt_version": RECEIPT_VERSION,
        "measurement_class": "hosted_public_fixture",
        "producer": {
            "build_sha": args.build_sha,
            "workload_id": f"hosted-public:{fixture_hash[:16]}",
            "document_class": "hash-pinned-public-pub",
            "source_bytes": fixture_bytes,
            "page_count": next(iter(page_counts)),
            "runtime_identity": runtime_identity,
        },
        "runs": runs,
        "summary": summarize_runs(runs),
        "final_equivalence": {
            "canonical_document_equal": True,
            "final_scene_equal": True,
            "final_search_projection_equal": True,
        },
        "evidence_authority": {
            "real_pub_runtime": True,
            "architecture_decision_allowed": False,
            "blocker": (
                "hosted runner timing is mechanics/regression evidence only; "
                "representative local/native measurement owns architecture decisions"
            ),
        },
        "corpus_identity": {
            "fixture_hash": fixture_hash,
            "raw_path": None,
        },
        "instrumentation": {
            "max_unattributed_overhead_ms": max(overheads),
            "median_unattributed_overhead_ms": statistics.median(overheads),
            "output_sha256": next(iter(output_hashes)),
        },
        "limitations": [
            "Hosted CI timing is not representative Chaptera product latency.",
            "The first_paint phase is a headless paint-readiness boundary; a separate reader-only WGPU acceptance proves an actual rendered first frame.",
            "The current Viewer path is eager; document-global pre-first phases are reported as observed and do not authorize lazy parsing.",
            "Current search scans recovered Story text directly and has no separate search-index build.",
        ],
    }
    validate_receipt(receipt)

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "receipt_version": receipt["receipt_version"],
                "measurement_class": receipt["measurement_class"],
                "fixture_hash": fixture_hash,
                "cold_runs": args.cold_runs,
                "warm_runs": args.warm_runs,
                "architecture_decision_allowed": False,
                "runner_platform": platform.platform(),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
