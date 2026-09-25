#!/usr/bin/env python3
"""Run the public Chaptera recovery-worker fail-closed protocol probe."""

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
        source = root / "damaged-probe.pub"
        source.write_bytes(b"synthetic protocol probe; not a real Publisher recovery fixture")
        before = sha256(source)
        job_dir = root / "job-output"
        job = {
            "protocol_version": "chaptera.rescue-worker-job.v1",
            "job_id": "11111111-1111-4111-8111-111111111111",
            "source": {"path": str(source), "sha256": before},
            "operation": "bounded_recovery",
            "output": {"job_directory": str(job_dir)},
            "limits": {
                "wall_time_ms": 60000,
                "cpu_time_ms": 30000,
                "memory_bytes": 268435456,
                "output_bytes": 67108864,
                "artifact_count": 1000,
            },
            "policy": {
                "source_mutation_allowed": False,
                "native_pub_delivery_allowed": False,
            },
        }
        completed = subprocess.run(
            [str(args.exe)],
            input=json.dumps(job),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if completed.returncode != 3:
            raise SystemExit(
                f"expected executor_unavailable exit 3, got {completed.returncode}\n"
                f"stdout={completed.stdout}\nstderr={completed.stderr}"
            )
        values = [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]
        if len(values) < 4:
            raise SystemExit(f"worker emitted too few protocol records: {values!r}")
        if values[0].get("event") != "started":
            raise SystemExit("worker did not emit started event")
        if not any(
            item.get("phase") == "source_verification" and item.get("status") == "running"
            for item in values
        ):
            raise SystemExit("worker did not emit source_verification phase")
        terminal = values[-1]
        if terminal.get("protocol_version") != "chaptera.rescue-worker-result.v1":
            raise SystemExit("terminal worker record has wrong protocol version")
        if terminal.get("status") != "executor_unavailable":
            raise SystemExit("public worker shell must end executor_unavailable")
        if terminal.get("source_unchanged") is not True:
            raise SystemExit("terminal result must prove source_unchanged")
        if terminal.get("producer_receipt") is not None:
            raise SystemExit("executor_unavailable must not emit a producer receipt")
        if terminal.get("executor", {}).get("available") is not False:
            raise SystemExit("public worker shell must not claim an executor")
        after = sha256(source)
        if after != before:
            raise SystemExit("worker mutated the source during protocol probe")
        if job_dir.exists() and any(job_dir.iterdir()):
            raise SystemExit("executor_unavailable worker unexpectedly produced output artifacts")

        print(
            json.dumps(
                {
                    "status": "pass",
                    "worker_exit": completed.returncode,
                    "terminal_status": terminal["status"],
                    "source_unchanged": True,
                    "producer_receipt": None,
                },
                indent=2,
                sort_keys=True,
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
