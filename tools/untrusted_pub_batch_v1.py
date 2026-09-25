#!/usr/bin/env python3
"""Canonical Rar batch admission for untrusted PUB inputs.

This is a consumer of tools/migration_pdf_worker_isolation.py, not a second
sandbox. The parent performs deterministic regular-file discovery and size
preflight; each admitted file then runs in the existing per-file process/
rlimit/network boundary. The Rust worker reads the authorized PUB once, seals
path-based filesystem/process access, and parses CFB only from memory.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import pathlib
import shutil
import stat
from dataclasses import asdict, dataclass
from typing import Iterable

from migration_pdf_worker_isolation import (
    WorkerLimits,
    run_isolated_worker,
)

SECURITY_PROFILE = "chaptera-untrusted-pub-v1"
DEFAULT_MAX_FILE_BYTES = 256 * 1024 * 1024
DEFAULT_MAX_CFB_ENTRIES = 8_192
DEFAULT_MAX_DECLARED_STREAM_BYTES = 512 * 1024 * 1024


@dataclass(frozen=True)
class BatchPolicyV1:
    max_file_bytes: int = DEFAULT_MAX_FILE_BYTES
    max_cfb_entries: int = DEFAULT_MAX_CFB_ENTRIES
    max_declared_stream_bytes: int = DEFAULT_MAX_DECLARED_STREAM_BYTES
    wall_timeout_ms: int = 15_000
    address_space_bytes: int = 1024 * 1024 * 1024
    cpu_seconds: int = 10
    open_files: int = 64

    def validate(self) -> None:
        for name in (
            "max_file_bytes",
            "max_cfb_entries",
            "max_declared_stream_bytes",
            "wall_timeout_ms",
            "address_space_bytes",
            "cpu_seconds",
            "open_files",
        ):
            if getattr(self, name) <= 0:
                raise ValueError(f"{name} must be positive")
        if self.address_space_bytes < 64 * 1024 * 1024:
            raise ValueError("address_space_bytes is implausibly small")
        if self.open_files < 16:
            raise ValueError("open_files must be at least 16")


def _relative(root: pathlib.Path, path: pathlib.Path) -> str:
    return path.relative_to(root).as_posix()


def discover_regular_pub_files(root: pathlib.Path) -> list[pathlib.Path]:
    """Return deterministic regular .pub files without following symlinks/FIFOs."""

    root = root.resolve()
    if not root.is_dir():
        raise NotADirectoryError(root)

    found: list[pathlib.Path] = []

    def visit(directory: pathlib.Path) -> None:
        with os.scandir(directory) as entries:
            ordered = sorted(entries, key=lambda item: item.name)
        for entry in ordered:
            path = pathlib.Path(entry.path)
            if entry.is_dir(follow_symlinks=False):
                visit(path)
                continue
            if not entry.name.lower().endswith(".pub"):
                continue
            try:
                mode = entry.stat(follow_symlinks=False).st_mode
            except OSError:
                continue
            if stat.S_ISREG(mode):
                found.append(path)

    visit(root)
    found.sort(key=lambda path: _relative(root, path))
    return found


def _worker_limits(policy: BatchPolicyV1) -> WorkerLimits:
    return WorkerLimits(
        address_space_bytes=policy.address_space_bytes,
        cpu_seconds=policy.cpu_seconds,
        open_files=policy.open_files,
        output_file_bytes=4 * 1024 * 1024,
    )


def _read_worker_receipt(output_dir: pathlib.Path) -> dict:
    path = output_dir / "result.json"
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("protocol_version") != "chaptera.untrusted-pub-scan-result.v1":
        raise ValueError("worker returned the wrong untrusted-PUB protocol")
    if value.get("security_profile") != SECURITY_PROFILE:
        raise ValueError("worker returned the wrong security profile")
    if value.get("filesystem_confinement") is not True:
        raise ValueError("worker did not prove post-read filesystem confinement")
    return value


def run_untrusted_pub_batch(
    root: pathlib.Path,
    *,
    worker: pathlib.Path,
    output_root: pathlib.Path,
    policy: BatchPolicyV1 | None = None,
) -> dict:
    policy = policy or BatchPolicyV1()
    policy.validate()
    root = root.resolve()
    worker = worker.resolve()
    output_root = output_root.resolve()

    if output_root.exists():
        raise FileExistsError(f"output_root already exists: {output_root}")
    output_root.mkdir(parents=True)

    rows: list[dict] = []
    for index, path in enumerate(discover_regular_pub_files(root)):
        relative_path = _relative(root, path)
        try:
            st = path.stat(follow_symlinks=False)
        except OSError as exc:
            rows.append(
                {
                    "relative_path": relative_path,
                    "byte_len": None,
                    "status": "read_failed",
                    "sha256": None,
                    "cfb_entry_count": None,
                    "declared_stream_bytes": None,
                    "filesystem_confinement": False,
                    "security_event": None,
                    "error": f"{type(exc).__name__}: {exc}",
                }
            )
            continue

        byte_len = st.st_size
        if byte_len > policy.max_file_bytes:
            rows.append(
                {
                    "relative_path": relative_path,
                    "byte_len": byte_len,
                    "status": "rejected_by_policy",
                    "sha256": None,
                    "cfb_entry_count": None,
                    "declared_stream_bytes": None,
                    "filesystem_confinement": False,
                    "security_event": (
                        f"input_size_limit: {byte_len} > {policy.max_file_bytes}"
                    ),
                    "error": "PUB input exceeds admitted byte limit",
                }
            )
            continue

        final_output = output_root / f"job-{index:04d}"
        result = run_isolated_worker(
            [
                str(worker),
                "inspect",
                "--max-file-bytes",
                str(policy.max_file_bytes),
                "--max-cfb-entries",
                str(policy.max_cfb_entries),
                "--max-declared-stream-bytes",
                str(policy.max_declared_stream_bytes),
            ],
            final_output_dir=final_output,
            timeout_seconds=policy.wall_timeout_ms / 1000.0,
            limits=_worker_limits(policy),
            input_path=path,
        )

        if result.status == "timeout":
            rows.append(
                {
                    "relative_path": relative_path,
                    "byte_len": byte_len,
                    "status": "timed_out",
                    "sha256": None,
                    "cfb_entry_count": None,
                    "declared_stream_bytes": None,
                    "filesystem_confinement": False,
                    "security_event": f"wall_timeout_ms: {policy.wall_timeout_ms}",
                    "error": "worker exceeded wall-clock limit",
                }
            )
            continue

        if not result.succeeded:
            rows.append(
                {
                    "relative_path": relative_path,
                    "byte_len": byte_len,
                    "status": "worker_failed",
                    "sha256": None,
                    "cfb_entry_count": None,
                    "declared_stream_bytes": None,
                    "filesystem_confinement": False,
                    "security_event": "worker_process_failure",
                    "error": result.stderr_tail,
                }
            )
            continue

        receipt = _read_worker_receipt(final_output)
        rows.append(
            {
                "relative_path": relative_path,
                "byte_len": receipt["byte_len"],
                "status": receipt["status"],
                "sha256": receipt["sha256"],
                "cfb_entry_count": receipt["cfb_entry_count"],
                "declared_stream_bytes": receipt["declared_stream_bytes"],
                "filesystem_confinement": receipt["filesystem_confinement"],
                "security_event": receipt["security_event"],
                "error": receipt["error"],
            }
        )

    return {
        "protocol_version": "chaptera.untrusted-pub-batch.v1",
        "security": {
            "profile": SECURITY_PROFILE,
            "shared_worker_harness": "tools/migration_pdf_worker_isolation.py",
            "worker_process_isolation": True,
            "input_size_limit": True,
            "cfb_structure_limits": True,
            "linux_rlimits": True,
            "network_default_deny": True,
            "post_read_filesystem_confinement": True,
            **asdict(policy),
        },
        "root": str(root),
        "files": rows,
    }


def run_filesystem_probe(
    *,
    worker: pathlib.Path,
    source: pathlib.Path,
    output_dir: pathlib.Path,
) -> dict:
    result = run_isolated_worker(
        [str(worker.resolve()), "probe"],
        final_output_dir=output_dir.resolve(),
        timeout_seconds=10.0,
        limits=WorkerLimits(),
        input_path=source.resolve(),
    )
    if not result.succeeded:
        raise RuntimeError(f"filesystem probe worker failed: {result.stderr_tail}")
    value = json.loads((output_dir / "result.json").read_text(encoding="utf-8"))
    if value.get("protocol_version") != "chaptera.untrusted-pub-filesystem-probe.v1":
        raise ValueError("filesystem probe returned wrong protocol")
    return value


def _policy_from_args(args: argparse.Namespace) -> BatchPolicyV1:
    return BatchPolicyV1(
        max_file_bytes=args.max_file_bytes,
        max_cfb_entries=args.max_cfb_entries,
        max_declared_stream_bytes=args.max_declared_stream_bytes,
        wall_timeout_ms=args.wall_timeout_ms,
        address_space_bytes=args.address_space_mb * 1024 * 1024,
        cpu_seconds=args.cpu_seconds,
        open_files=args.open_files,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="mode", required=True)

    batch = sub.add_parser("batch")
    batch.add_argument("root", type=pathlib.Path)
    batch.add_argument("--worker", type=pathlib.Path, required=True)
    batch.add_argument("--output-root", type=pathlib.Path, required=True)
    batch.add_argument("--max-file-bytes", type=int, default=DEFAULT_MAX_FILE_BYTES)
    batch.add_argument("--max-cfb-entries", type=int, default=DEFAULT_MAX_CFB_ENTRIES)
    batch.add_argument(
        "--max-declared-stream-bytes",
        type=int,
        default=DEFAULT_MAX_DECLARED_STREAM_BYTES,
    )
    batch.add_argument("--wall-timeout-ms", type=int, default=15_000)
    batch.add_argument("--address-space-mb", type=int, default=1024)
    batch.add_argument("--cpu-seconds", type=int, default=10)
    batch.add_argument("--open-files", type=int, default=64)

    probe = sub.add_parser("probe")
    probe.add_argument("--worker", type=pathlib.Path, required=True)
    probe.add_argument("--source", type=pathlib.Path, required=True)
    probe.add_argument("--output-dir", type=pathlib.Path, required=True)

    args = parser.parse_args()
    if args.mode == "batch":
        value = run_untrusted_pub_batch(
            args.root,
            worker=args.worker,
            output_root=args.output_root,
            policy=_policy_from_args(args),
        )
    else:
        value = run_filesystem_probe(
            worker=args.worker,
            source=args.source,
            output_dir=args.output_dir,
        )
    print(json.dumps(value, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
