#!/usr/bin/env python3
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile
import urllib.parse
import urllib.request


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run_bounded(argv, timeout_seconds):
    try:
        completed = subprocess.run(
            [str(item) for item in argv],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout_seconds,
            check=False,
        )
        return {
            "timed_out": False,
            "returncode": completed.returncode,
        }
    except subprocess.TimeoutExpired:
        return {
            "timed_out": True,
            "returncode": None,
        }


def classify(product_run, engine_receipt, receipt_run):
    if product_run["timed_out"]:
        return "reader_timeout", True
    if receipt_run["timed_out"]:
        return "structural_receipt_timeout", True
    if product_run["returncode"] not in (0, 1):
        return "reader_execution_failed", True
    if receipt_run["returncode"] != 0 or engine_receipt is None:
        return "structural_receipt_failed", True

    product_opened = product_run["returncode"] == 0
    engine_opened = bool(engine_receipt.get("opened"))
    if product_opened != engine_opened:
        return "product_engine_disagreement", True
    if product_opened:
        return "opened", False
    return "unsupported_or_open_failed", False


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=pathlib.Path)
    parser.add_argument("--reader-exe", required=True, type=pathlib.Path)
    parser.add_argument("--viewer-receipt-exe", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--per-file-dir", required=True, type=pathlib.Path)
    parser.add_argument("--timeout-seconds", type=int, default=25)
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    base_url = manifest["upstream"]["base_url"].rstrip("/")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.per_file_dir.mkdir(parents=True, exist_ok=True)

    rows = []
    hard_violations = []

    with tempfile.TemporaryDirectory(prefix="chaptera-reader-corpus-") as td:
        temp_root = pathlib.Path(td)
        for entry in manifest["fixtures"]:
            name = entry["name"]
            fixture = temp_root / name
            url = base_url + "/" + urllib.parse.quote(name)
            with urllib.request.urlopen(url, timeout=60) as response:
                data = response.read()

            actual_sha = sha256_bytes(data)
            if actual_sha != entry["sha256"]:
                raise RuntimeError(
                    f"{name}: SHA-256 mismatch: {actual_sha} != {entry['sha256']}"
                )
            if len(data) != entry["byte_len"]:
                raise RuntimeError(
                    f"{name}: byte length mismatch: {len(data)} != {entry['byte_len']}"
                )
            fixture.write_bytes(data)

            before_sha = sha256_bytes(fixture.read_bytes())
            product_run = run_bounded(
                [args.reader_exe, "--smoke-check", fixture],
                args.timeout_seconds,
            )
            after_sha = sha256_bytes(fixture.read_bytes())
            source_unchanged = before_sha == after_sha

            safe_stem = hashlib.sha256(name.encode("utf-8")).hexdigest()[:16]
            structural_path = args.per_file_dir / f"{safe_stem}.structural.json"
            receipt_run = run_bounded(
                [args.viewer_receipt_exe, fixture, structural_path],
                args.timeout_seconds,
            )
            engine_receipt = None
            if receipt_run["returncode"] == 0 and structural_path.is_file():
                engine_receipt = json.loads(structural_path.read_text(encoding="utf-8"))

            outcome, hard = classify(product_run, engine_receipt, receipt_run)
            if not source_unchanged:
                outcome = "source_mutated"
                hard = True

            row = {
                "fixture": name,
                "family": entry["family"],
                "semantic": bool(entry["semantic"]),
                "sha256": actual_sha,
                "byte_len": len(data),
                "product_reader_exit_code": product_run["returncode"],
                "product_reader_timed_out": product_run["timed_out"],
                "source_unchanged": source_unchanged,
                "outcome": outcome,
                "structural": engine_receipt,
                "visual_fidelity_proven": False,
            }
            rows.append(row)

            per_file_path = args.per_file_dir / f"{safe_stem}.matrix-row.json"
            per_file_path.write_text(
                json.dumps(row, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            if hard:
                hard_violations.append({"fixture": name, "outcome": outcome})

    semantic_rows = [row for row in rows if row["semantic"]]
    opened_rows = [row for row in rows if row["outcome"] == "opened"]
    semantic_opened = [row for row in semantic_rows if row["outcome"] == "opened"]
    unsupported = [row for row in rows if row["outcome"] == "unsupported_or_open_failed"]
    partial = [
        row for row in opened_rows
        if row.get("structural", {}).get("fidelity_status") == "partial"
    ]

    matrix = {
        "schema": "chaptera.reader-corpus-matrix.v1",
        "manifest_schema": manifest["schema_version"],
        "upstream": manifest["upstream"],
        "claims": {
            "actual_reader_product_path_exercised": True,
            "source_immutability_checked": True,
            "source_free_structural_receipts_emitted": True,
            "visual_fidelity_proven": False,
            "publisher_equivalence_claimed": False,
            "interpretation": "opened means the current Reader product path produced non-empty Viewer pages and scene nodes; it is not a visual-fidelity percentage",
        },
        "summary": {
            "fixture_count": len(rows),
            "semantic_fixture_count": len(semantic_rows),
            "opened_count": len(opened_rows),
            "semantic_opened_count": len(semantic_opened),
            "unsupported_or_open_failed_count": len(unsupported),
            "opened_with_partial_fidelity_count": len(partial),
            "timeout_count": sum(
                1 for row in rows
                if row["product_reader_timed_out"]
                or row["outcome"] == "structural_receipt_timeout"
            ),
            "source_mutation_count": sum(1 for row in rows if not row["source_unchanged"]),
            "product_engine_disagreement_count": sum(
                1 for row in rows if row["outcome"] == "product_engine_disagreement"
            ),
            "hard_violation_count": len(hard_violations),
        },
        "rows": rows,
        "hard_violations": hard_violations,
    }
    args.output.write_text(
        json.dumps(matrix, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(matrix["summary"], indent=2, sort_keys=True))
    if hard_violations:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
