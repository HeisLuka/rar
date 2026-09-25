#!/usr/bin/env python3
"""Launch Chaptera's recovery worker under the proven Windows Job Object fence.

This is orchestration, not recovery. The configured external executor is passed
as worker launch configuration, never through the untrusted job JSON. The
worker then spawns the executor as its child; Windows associates child
processes with the parent's job by default when breakaway is not enabled.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time
import uuid

from probe_rescue_worker_windows_fence import (
    assign_job,
    close_job,
    create_job,
    terminate_job,
)

FENCE_MODE_ENV = "CHAPTERA_RECOVERY_FENCE_MODE"
FENCE_MODE = "windows_job_object_v1"
JOB_VERSION = "chaptera.rescue-worker-job.v1"
RESULT_VERSION = "chaptera.rescue-worker-result.v1"


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _lower_sha256(value: str) -> bool:
    return len(value) == 64 and all(char in "0123456789abcdef" for char in value)


def _parse_terminal(stdout: bytes) -> tuple[list[dict], dict | None]:
    values: list[dict] = []
    for raw in stdout.decode("utf-8", errors="strict").splitlines():
        if raw.strip():
            values.append(json.loads(raw))
    terminal = next(
        (
            value
            for value in reversed(values)
            if value.get("protocol_version") == RESULT_VERSION
        ),
        None,
    )
    return values, terminal


def run_worker(
    *,
    worker: pathlib.Path,
    executor: pathlib.Path,
    executor_args: list[str],
    source: pathlib.Path,
    source_sha256: str,
    job_directory: pathlib.Path,
    job_id: str,
    wall_time_ms: int,
    cpu_time_ms: int,
    memory_bytes: int,
    output_bytes: int,
    artifact_count: int,
    native_pub_delivery_allowed: bool,
) -> dict:
    if sys.platform != "win32":
        raise RuntimeError("authorized Rescue worker launcher requires Windows")
    if not worker.is_file():
        raise ValueError(f"worker executable not found: {worker}")
    if not executor.is_file():
        raise ValueError(f"executor executable not found: {executor}")
    if not source.is_file():
        raise ValueError(f"source not found: {source}")
    if not _lower_sha256(source_sha256):
        raise ValueError("source_sha256 must be lowercase SHA-256")
    if sha256(source) != source_sha256:
        raise ValueError("source bytes do not match admitted source_sha256")
    if job_directory.exists() and any(job_directory.iterdir()):
        raise ValueError("job_directory must be empty before launch")

    before = sha256(source)
    job = {
        "protocol_version": JOB_VERSION,
        "job_id": job_id,
        "source": {
            "path": str(source.resolve()),
            "sha256": source_sha256,
        },
        "operation": "bounded_recovery",
        "output": {
            "job_directory": str(job_directory.resolve()),
        },
        "limits": {
            "wall_time_ms": wall_time_ms,
            "cpu_time_ms": cpu_time_ms,
            "memory_bytes": memory_bytes,
            "output_bytes": output_bytes,
            "artifact_count": artifact_count,
        },
        "policy": {
            "source_mutation_allowed": False,
            "native_pub_delivery_allowed": native_pub_delivery_allowed,
        },
    }

    command = [str(worker.resolve()), "--executor", str(executor.resolve())]
    for value in executor_args:
        command.extend(["--executor-arg", value])

    environment = os.environ.copy()
    environment[FENCE_MODE_ENV] = FENCE_MODE

    job_handle = create_job(
        cpu_time_ms=cpu_time_ms,
        memory_bytes=memory_bytes,
    )
    process: subprocess.Popen[bytes] | None = None
    timed_out = False
    started = time.monotonic()
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
        )
        assign_job(job_handle, process)

        payload = json.dumps(job, separators=(",", ":")).encode("utf-8")
        assert process.stdin is not None
        process.stdin.write(payload)
        process.stdin.close()
        process.stdin = None

        deadline = started + wall_time_ms / 1000.0
        while process.poll() is None:
            if time.monotonic() >= deadline:
                timed_out = True
                terminate_job(job_handle, 1460)
                break
            time.sleep(0.01)

        stdout, stderr = process.communicate(timeout=5)
        elapsed_ms = int((time.monotonic() - started) * 1000)
        after = sha256(source)
        if after != before:
            raise RuntimeError("source identity changed while fenced worker tree was running")

        values, terminal = _parse_terminal(stdout)
        stderr_text = stderr.decode("utf-8", errors="replace").strip()
        if stderr_text:
            print(stderr_text[-4096:], file=sys.stderr)

        if timed_out:
            return {
                "schema_version": "chaptera.rescue-worker-launch.v1",
                "status": "timed_out",
                "code": "wall_time",
                "worker_exit": process.returncode,
                "elapsed_ms": elapsed_ms,
                "source_sha256": source_sha256,
                "source_unchanged": True,
                "producer_receipt": None,
            }

        if terminal is None:
            return {
                "schema_version": "chaptera.rescue-worker-launch.v1",
                "status": "failed",
                "code": "missing_terminal_result",
                "worker_exit": process.returncode,
                "elapsed_ms": elapsed_ms,
                "source_sha256": source_sha256,
                "source_unchanged": True,
                "producer_receipt": None,
                "protocol_record_count": len(values),
            }

        if terminal.get("source_sha256") != source_sha256:
            raise RuntimeError("worker terminal result changed admitted source identity")
        if terminal.get("source_unchanged") is not True:
            raise RuntimeError("worker terminal result does not prove source_unchanged")

        return {
            "schema_version": "chaptera.rescue-worker-launch.v1",
            "status": terminal.get("status"),
            "code": terminal.get("code"),
            "worker_exit": process.returncode,
            "elapsed_ms": elapsed_ms,
            "source_sha256": source_sha256,
            "source_unchanged": True,
            "executor": terminal.get("executor"),
            "producer_receipt": terminal.get("producer_receipt"),
            "protocol_record_count": len(values),
            "windows_job_object": {
                "worker_assigned": True,
                "child_inheritance_required": True,
                "breakaway_enabled": False,
                "process_cpu_time_limit_ms": cpu_time_ms,
                "process_memory_limit_bytes": memory_bytes,
                "parent_wall_time_limit_ms": wall_time_ms,
            },
        }
    finally:
        if process is not None and process.poll() is None:
            terminate_job(job_handle, 1)
            process.wait(timeout=5)
        close_job(job_handle)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True, type=pathlib.Path)
    parser.add_argument("--executor", required=True, type=pathlib.Path)
    parser.add_argument("--executor-arg", action="append", default=[])
    parser.add_argument("--source", required=True, type=pathlib.Path)
    parser.add_argument("--source-sha256", required=True)
    parser.add_argument("--job-directory", required=True, type=pathlib.Path)
    parser.add_argument("--job-id", default=None)
    parser.add_argument("--wall-time-ms", type=int, default=60_000)
    parser.add_argument("--cpu-time-ms", type=int, default=30_000)
    parser.add_argument("--memory-bytes", type=int, default=268_435_456)
    parser.add_argument("--output-bytes", type=int, default=67_108_864)
    parser.add_argument("--artifact-count", type=int, default=1000)
    parser.add_argument("--native-pub-delivery-allowed", action="store_true")
    parser.add_argument("--output-json", type=pathlib.Path)
    args = parser.parse_args()

    job_id = args.job_id or str(uuid.uuid4())
    result = run_worker(
        worker=args.worker,
        executor=args.executor,
        executor_args=args.executor_arg,
        source=args.source,
        source_sha256=args.source_sha256,
        job_directory=args.job_directory,
        job_id=job_id,
        wall_time_ms=args.wall_time_ms,
        cpu_time_ms=args.cpu_time_ms,
        memory_bytes=args.memory_bytes,
        output_bytes=args.output_bytes,
        artifact_count=args.artifact_count,
        native_pub_delivery_allowed=args.native_pub_delivery_allowed,
    )
    rendered = json.dumps(result, indent=2, sort_keys=True)
    if args.output_json is not None:
        args.output_json.parent.mkdir(parents=True, exist_ok=True)
        args.output_json.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0 if result["status"] == "succeeded" else 2


if __name__ == "__main__":
    raise SystemExit(main())
