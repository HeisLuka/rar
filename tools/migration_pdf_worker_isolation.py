#!/usr/bin/env python3
"""Bounded per-file worker isolation for migration/archive conversion.

This module is deliberately converter-neutral. It provides the minimum process
boundary required by MIGRATION-PDF-SEC-01 so a future rescue-pdf adapter can
delegate one file at a time without copying the legacy converter into Rar.

Linux guarantees in this slice:
- one fresh process group per file;
- parent wall timeout with whole-process-group kill;
- RLIMIT_AS / RLIMIT_CPU / RLIMIT_NOFILE / RLIMIT_FSIZE before exec;
- no_new_privs + libseccomp network syscall deny before exec;
- worker output is written to an unpublished staging directory;
- only a successful worker may atomically publish regular files/directories;
- failed/timed-out staging is removed.

Filesystem write confinement is NOT claimed: the worker receives a staging
directory but this slice does not mount/chroot/namespace the filesystem.
"""

from __future__ import annotations

import argparse
import ctypes
import ctypes.util
import errno
import json
import math
import os
import pathlib
import resource
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
from dataclasses import asdict, dataclass
from typing import Any, Iterable, Mapping, Sequence

NETWORK_SYSCALLS = (
    "socket",
    "socketpair",
    "connect",
    "accept",
    "accept4",
    "bind",
    "listen",
    "sendto",
    "recvfrom",
    "sendmsg",
    "recvmsg",
    "sendmmsg",
    "recvmmsg",
    "shutdown",
)

SCMP_ACT_ALLOW = 0x7FFF0000
SCMP_ACT_ERRNO_BASE = 0x00050000
PR_SET_NO_NEW_PRIVS = 38
CONTROL_CAPTURE_LIMIT = 64 * 1024


@dataclass(frozen=True)
class WorkerLimits:
    address_space_bytes: int = 512 * 1024 * 1024
    cpu_seconds: int = 10
    open_files: int = 64
    output_file_bytes: int = 32 * 1024 * 1024

    def validate(self) -> None:
        if self.address_space_bytes < 64 * 1024 * 1024:
            raise ValueError("address_space_bytes is implausibly small")
        if self.cpu_seconds <= 0:
            raise ValueError("cpu_seconds must be positive")
        if self.open_files < 16:
            raise ValueError("open_files must be at least 16")
        if self.output_file_bytes <= 0:
            raise ValueError("output_file_bytes must be positive")


@dataclass(frozen=True)
class WorkerResult:
    status: str
    exit_code: int | None
    timed_out: bool
    outputs: tuple[str, ...]
    staging_cleaned: bool
    network_policy: str
    limits: WorkerLimits
    stderr_tail: str

    @property
    def succeeded(self) -> bool:
        return self.status == "success"


def _scmp_act_errno(error_number: int) -> int:
    return SCMP_ACT_ERRNO_BASE | (error_number & 0xFFFF)


def _require_linux() -> None:
    if sys.platform != "linux":
        raise RuntimeError("MIGRATION-PDF-SEC V1 guarded worker currently requires Linux")


def _install_no_new_privs() -> None:
    libc_name = ctypes.util.find_library("c")
    if not libc_name:
        raise RuntimeError("libc unavailable; cannot set no_new_privs")
    libc = ctypes.CDLL(libc_name, use_errno=True)
    libc.prctl.argtypes = [
        ctypes.c_int,
        ctypes.c_ulong,
        ctypes.c_ulong,
        ctypes.c_ulong,
        ctypes.c_ulong,
    ]
    libc.prctl.restype = ctypes.c_int
    if libc.prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0:
        err = ctypes.get_errno()
        raise OSError(err, "prctl(PR_SET_NO_NEW_PRIVS) failed")


def _install_network_deny_seccomp() -> None:
    library = ctypes.util.find_library("seccomp")
    if not library:
        raise RuntimeError("libseccomp unavailable; refusing unguarded worker execution")

    seccomp = ctypes.CDLL(library, use_errno=True)
    seccomp.seccomp_init.argtypes = [ctypes.c_uint32]
    seccomp.seccomp_init.restype = ctypes.c_void_p
    seccomp.seccomp_syscall_resolve_name.argtypes = [ctypes.c_char_p]
    seccomp.seccomp_syscall_resolve_name.restype = ctypes.c_int
    seccomp.seccomp_load.argtypes = [ctypes.c_void_p]
    seccomp.seccomp_load.restype = ctypes.c_int
    seccomp.seccomp_release.argtypes = [ctypes.c_void_p]
    seccomp.seccomp_release.restype = None
    # seccomp_rule_add is variadic, but with arg_cnt=0 this call has exactly
    # four fixed arguments. Declaring them is essential on 64-bit hosts:
    # without c_void_p for ctx, ctypes may truncate the pointer and crash.
    seccomp.seccomp_rule_add.argtypes = [
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.c_int,
        ctypes.c_uint,
    ]
    seccomp.seccomp_rule_add.restype = ctypes.c_int

    ctx = seccomp.seccomp_init(SCMP_ACT_ALLOW)
    if not ctx:
        raise RuntimeError("seccomp_init failed")

    try:
        denied_action = _scmp_act_errno(errno.EPERM)
        for name in NETWORK_SYSCALLS:
            syscall_number = seccomp.seccomp_syscall_resolve_name(name.encode("ascii"))
            if syscall_number < 0:
                continue
            rc = seccomp.seccomp_rule_add(
                ctx,
                ctypes.c_uint32(denied_action),
                ctypes.c_int(syscall_number),
                ctypes.c_uint(0),
            )
            if rc != 0:
                raise RuntimeError(f"seccomp_rule_add({name}) failed: {rc}")
        rc = seccomp.seccomp_load(ctx)
        if rc != 0:
            raise RuntimeError(f"seccomp_load failed: {rc}")
    finally:
        seccomp.seccomp_release(ctx)


def _apply_limits(limits: WorkerLimits) -> None:
    limits.validate()
    resource.setrlimit(
        resource.RLIMIT_AS,
        (limits.address_space_bytes, limits.address_space_bytes),
    )
    # Give the kernel one second of hard-limit headroom so SIGXCPU can arrive
    # deterministically before the hard kill.
    resource.setrlimit(
        resource.RLIMIT_CPU,
        (limits.cpu_seconds, limits.cpu_seconds + 1),
    )
    resource.setrlimit(
        resource.RLIMIT_NOFILE,
        (limits.open_files, limits.open_files),
    )
    resource.setrlimit(
        resource.RLIMIT_FSIZE,
        (limits.output_file_bytes, limits.output_file_bytes),
    )


def _guard_exec(command: Sequence[str], limits: WorkerLimits) -> "NoReturn":
    _require_linux()
    if not command:
        raise RuntimeError("guard requires a command")
    _apply_limits(limits)
    _install_no_new_privs()
    _install_network_deny_seccomp()
    os.execvpe(command[0], list(command), os.environ.copy())
    raise AssertionError("exec returned unexpectedly")


def _read_tail(path: pathlib.Path, limit: int = CONTROL_CAPTURE_LIMIT) -> str:
    if not path.exists():
        return ""
    with path.open("rb") as fh:
        try:
            fh.seek(-limit, os.SEEK_END)
        except OSError:
            fh.seek(0)
        return fh.read(limit).decode("utf-8", errors="replace")


def _validate_publish_tree(root: pathlib.Path) -> tuple[str, ...]:
    outputs: list[str] = []
    for path in sorted(root.rglob("*")):
        rel = path.relative_to(root)
        st = path.lstat()
        if stat.S_ISLNK(st.st_mode):
            raise RuntimeError(f"worker output symlink is not publishable: {rel}")
        if path.is_dir():
            continue
        if not stat.S_ISREG(st.st_mode):
            raise RuntimeError(f"worker output is not a regular file: {rel}")
        outputs.append(rel.as_posix())
    if not outputs:
        raise RuntimeError("successful worker produced no output files")
    return tuple(outputs)


def _cleanup(path: pathlib.Path) -> bool:
    try:
        if path.exists():
            shutil.rmtree(path)
        return not path.exists()
    except OSError:
        return False


def run_isolated_worker(
    command: Sequence[str],
    *,
    final_output_dir: pathlib.Path,
    timeout_seconds: float,
    limits: WorkerLimits | None = None,
    input_path: pathlib.Path | None = None,
    extra_env: Mapping[str, str] | None = None,
) -> WorkerResult:
    """Run one worker and publish its staging directory only on success."""

    _require_linux()
    limits = limits or WorkerLimits()
    limits.validate()
    final_output_dir = final_output_dir.resolve()
    parent = final_output_dir.parent
    parent.mkdir(parents=True, exist_ok=True)

    if final_output_dir.exists():
        raise FileExistsError(f"final output already exists: {final_output_dir}")

    staging = pathlib.Path(
        tempfile.mkdtemp(prefix=f".{final_output_dir.name}.stage-", dir=parent)
    )
    control = pathlib.Path(
        tempfile.mkdtemp(prefix=f".{final_output_dir.name}.control-", dir=parent)
    )
    stdout_path = control / "stdout.log"
    stderr_path = control / "stderr.log"

    if timeout_seconds <= 0:
        cleaned = _cleanup(staging)
        _cleanup(control)
        return WorkerResult(
            status="timeout",
            exit_code=None,
            timed_out=True,
            outputs=(),
            staging_cleaned=cleaned,
            network_policy="seccomp_default_deny",
            limits=limits,
            stderr_tail="",
        )

    env = os.environ.copy()
    env["CHAPTERA_WORKER_OUTPUT_DIR"] = str(staging)
    env["CHAPTERA_NETWORK_POLICY"] = "seccomp_default_deny"
    if input_path is not None:
        env["CHAPTERA_WORKER_INPUT"] = str(input_path.resolve())
    if extra_env:
        env.update({str(k): str(v) for k, v in extra_env.items()})

    guard_command = [
        sys.executable,
        str(pathlib.Path(__file__).resolve()),
        "_guard",
        "--limits-json",
        json.dumps(asdict(limits), separators=(",", ":")),
        "--",
        *command,
    ]

    process: subprocess.Popen[bytes] | None = None
    timed_out = False
    exit_code: int | None = None
    try:
        with stdout_path.open("wb") as stdout_file, stderr_path.open("wb") as stderr_file:
            process = subprocess.Popen(
                guard_command,
                stdin=subprocess.DEVNULL,
                stdout=stdout_file,
                stderr=stderr_file,
                env=env,
                close_fds=True,
                start_new_session=True,
            )
            try:
                exit_code = process.wait(timeout=timeout_seconds)
            except subprocess.TimeoutExpired:
                timed_out = True
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                exit_code = process.wait()

        stderr_tail = _read_tail(stderr_path)

        if timed_out:
            cleaned = _cleanup(staging)
            return WorkerResult(
                status="timeout",
                exit_code=exit_code,
                timed_out=True,
                outputs=(),
                staging_cleaned=cleaned,
                network_policy="seccomp_default_deny",
                limits=limits,
                stderr_tail=stderr_tail,
            )

        if exit_code != 0:
            cleaned = _cleanup(staging)
            return WorkerResult(
                status="failed",
                exit_code=exit_code,
                timed_out=False,
                outputs=(),
                staging_cleaned=cleaned,
                network_policy="seccomp_default_deny",
                limits=limits,
                stderr_tail=stderr_tail,
            )

        try:
            outputs = _validate_publish_tree(staging)
        except Exception:
            _cleanup(staging)
            raise

        os.replace(staging, final_output_dir)
        return WorkerResult(
            status="success",
            exit_code=0,
            timed_out=False,
            outputs=outputs,
            staging_cleaned=True,
            network_policy="seccomp_default_deny",
            limits=limits,
            stderr_tail=stderr_tail,
        )
    finally:
        _cleanup(control)
        if process is not None and process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        if staging.exists() and not final_output_dir.exists():
            _cleanup(staging)


def run_isolated_batch(
    jobs: Iterable[Mapping[str, Any]],
    *,
    timeout_seconds: float,
    limits: WorkerLimits | None = None,
) -> list[dict[str, Any]]:
    """Run all jobs sequentially; one failure never aborts later jobs."""

    results: list[dict[str, Any]] = []
    for job in jobs:
        name = str(job["name"])
        try:
            result = run_isolated_worker(
                list(job["command"]),
                final_output_dir=pathlib.Path(job["final_output_dir"]),
                timeout_seconds=timeout_seconds,
                limits=limits,
                input_path=(
                    pathlib.Path(job["input_path"])
                    if job.get("input_path") is not None
                    else None
                ),
                extra_env=job.get("env"),
            )
            row = {"name": name, **asdict(result)}
            row["limits"] = asdict(result.limits)
            results.append(row)
        except Exception as exc:
            results.append(
                {
                    "name": name,
                    "status": "failed",
                    "exit_code": None,
                    "timed_out": False,
                    "outputs": (),
                    "staging_cleaned": True,
                    "network_policy": "seccomp_default_deny",
                    "limits": asdict(limits or WorkerLimits()),
                    "stderr_tail": f"harness_error:{type(exc).__name__}:{exc}",
                }
            )
    return results


def _parse_limits(raw: str) -> WorkerLimits:
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise ValueError("limits JSON must be an object")
    limits = WorkerLimits(**value)
    limits.validate()
    return limits


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="mode", required=True)

    guard = sub.add_parser("_guard")
    guard.add_argument("--limits-json", required=True)
    guard.add_argument("command", nargs=argparse.REMAINDER)

    run = sub.add_parser("run")
    run.add_argument("--output-dir", required=True, type=pathlib.Path)
    run.add_argument("--input", type=pathlib.Path)
    run.add_argument("--timeout", required=True, type=float)
    run.add_argument("--address-space-mb", type=int, default=512)
    run.add_argument("--cpu-seconds", type=int, default=10)
    run.add_argument("--open-files", type=int, default=64)
    run.add_argument("--output-file-mb", type=int, default=32)
    run.add_argument("command", nargs=argparse.REMAINDER)

    args = parser.parse_args()

    if args.mode == "_guard":
        command = list(args.command)
        if command and command[0] == "--":
            command = command[1:]
        _guard_exec(command, _parse_limits(args.limits_json))

    command = list(args.command)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("worker command is required after --")

    result = run_isolated_worker(
        command,
        final_output_dir=args.output_dir,
        timeout_seconds=args.timeout,
        limits=WorkerLimits(
            address_space_bytes=args.address_space_mb * 1024 * 1024,
            cpu_seconds=args.cpu_seconds,
            open_files=args.open_files,
            output_file_bytes=args.output_file_mb * 1024 * 1024,
        ),
        input_path=args.input,
    )
    print(json.dumps({**asdict(result), "limits": asdict(result.limits)}, indent=2))
    return 0 if result.succeeded else 1


if __name__ == "__main__":
    raise SystemExit(main())
