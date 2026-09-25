#!/usr/bin/env python3
"""Exercise Chaptera Rescue source hashing + receipt/report path on Windows."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", required=True, type=pathlib.Path)
    args = parser.parse_args()

    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        source = root / "source.pub"
        source.write_bytes(b"synthetic Chaptera Rescue desktop path probe")
        source_sha = sha256(source)
        receipt = root / "product-validation.json"
        receipt.write_text(
            json.dumps(
                {
                    "receipt_version": "chaptera.rescue-product-validation.v1",
                    "product": "Chaptera Rescue",
                    "producer_receipt_sha256": "a" * 64,
                    "fixture": {
                        "kind": "healthy_control",
                        "source_sha256": source_sha,
                    },
                    "outcome": "unsupported/no_safe_recovery",
                    "source_immutable": True,
                    "artifact_count": 0,
                    "known_loss": False,
                    "native_pub_delivery_allowed": False,
                    "fail_closed": True,
                    "privacy": {
                        "source_free": True,
                        "private_content_serialized": False,
                    },
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        report = root / "report.json"
        before = sha256(source)
        completed = subprocess.run(
            [str(args.exe), "--acceptance-v1", str(source), str(receipt), str(report)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if completed.returncode != 0:
            raise SystemExit(
                f"Rescue acceptance path failed: {completed.returncode}\n"
                f"stdout={completed.stdout}\nstderr={completed.stderr}"
            )
        after = sha256(source)
        if after != before:
            raise SystemExit("Rescue acceptance path mutated source bytes")
        value = json.loads(report.read_text(encoding="utf-8"))
        if value.get("product_id") != "chaptera.rescue":
            raise SystemExit("report has wrong product identity")
        if value.get("source_sha256") != source_sha:
            raise SystemExit("report source identity mismatch")
        if value.get("outcome") != "unsupported/no_safe_recovery":
            raise SystemExit("probe must remain a fail-closed non-recovery outcome")
        if value.get("source_immutable") is not True:
            raise SystemExit("report must preserve source immutable state")
        if value.get("native_pub_delivery_allowed") is not False:
            raise SystemExit("probe must not enable native PUB delivery")
        print(
            json.dumps(
                {
                    "status": "pass",
                    "source_unchanged": True,
                    "outcome": value["outcome"],
                    "report_written": True,
                },
                indent=2,
                sort_keys=True,
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
