#!/usr/bin/env python3
"""Windows Job Object acceptance probe for Chaptera Rescue worker isolation.

This proves the OS resource-fence mechanics without embedding or simulating
Publisher recovery semantics. The same worker-status vocabulary is used by
chaptera.rescue-worker-result.v1.
"""

from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

STATUS_SUCCESS = "succeeded"
STATUS_FAILED = "failed"
STATUS_CANCELLED = "cancelled"
STATUS_TIMED_OUT = "timed_out"
STATUS_RESOURCE_LIMITED = "resource_limited"

JOB_OBJECT_LIMIT_PROCESS_TIME = 0x00000002
JOB_OBJECT_LIMIT_PROCESS_MEMORY = 0x00000100
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000
JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS = 9
ERROR_SUCCESS = 0

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True) if sys.platform == "win32" else None


class IO_COUNTERS(ctypes.Structure):
    _fields_ = [
        ("ReadOperationCount", ctypes.c_ulonglong),
        ("WriteOperationCount", ctypes.c_ulonglong),
        ("OtherOperationCount", ctypes.c_ulonglong),
        ("ReadTransferCount", ctypes.c_ulonglong),
        ("WriteTransferCount", ctypes.c_ulonglong),
        ("OtherTransferCount", ctypes.c_ulonglong),
    ]


class JOBOBJECT_BASIC_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("PerProcessUserTimeLimit", ctypes.c_longlong),
        ("PerJobUserTimeLimit", ctypes.c_longlong),
        ("LimitFlags", wintypes.DWORD),
        ("MinimumWorkingSetSize", ctypes.c_size_t),
        ("MaximumWorkingSetSize", ctypes.c_size_t),
        ("ActiveProcessLimit", wintypes.DWORD),
        ("Affinity", ctypes.c_size_t),
        ("PriorityClass", wintypes.DWORD),
        ("SchedulingClass", wintypes.DWORD),
    ]


class JOBOBJECT_EXTENDED_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("BasicLimitInformation", JOBOBJECT_BASIC_LIMIT_INFORMATION),
        ("IoInfo", IO_COUNTERS),
        ("ProcessMemoryLimit", ctypes.c_size_t),
        ("JobMemoryLimit", ctypes.c_size_t),
        ("PeakProcessMemoryUsed", ctypes.c_size_t),
        ("PeakJobMemoryUsed", ctypes.c_size_t),
    ]


def _require_windows() -> None:
    if sys.platform != "win32":
        raise RuntimeError("RESCUE-WORKER-WIN-FENCE-01 requires Windows")


def _win_error(label: str) -> OSError:
    code = ctypes.get_last_error()
    return OSError(code, f"{label} failed", None, code)


def _configure_api() -> None:
    assert kernel32 is not None
    kernel32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    kernel32.SetInformationJobObject.argtypes = [
        wintypes.HANDLE,
        ctypes.c_int,
        ctypes.c_void_p,
        wintypes.DWORD,
    ]
    kernel32.SetInformationJobObject.restype = wintypes.BOOL
    kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    kernel32.AssignProcessToJobObject.restype = wintypes.BOOL
    kernel32.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
    kernel32.TerminateJobObject.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def create_job(*, cpu_time_ms: int, memory_bytes: int) -> int:
    _require_windows()
    _configure_api()
    assert kernel32 is not None
    handle = kernel32.CreateJobObjectW(None, None)
    if not handle:
        raise _win_error("CreateJobObjectW")

    info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
    info.BasicLimitInformation.LimitFlags = (
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_PROCESS_TIME
    )
    info.BasicLimitInformation.PerProcessUserTimeLimit = int(cpu_time_ms) * 10_000
    info.ProcessMemoryLimit = int(memory_bytes)

    ok = kernel32.SetInformationJobObject(
        handle,
        JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
        ctypes.byref(info),
        ctypes.sizeof(info),
    )
    if not ok:
        kernel32.CloseHandle(handle)
        raise _win_error("SetInformationJobObject")
    return int(handle)


def close_job(handle: int) -> None:
    if kernel32 is not None and handle:
        kernel32.CloseHandle(wintypes.HANDLE(handle))


def terminate_job(handle: int, exit_code: int) -> None:
    assert kernel32 is not None
    if not kernel32.TerminateJobObject(wintypes.HANDLE(handle), exit_code):
        raise _win_error("TerminateJobObject")


def assign_job(handle: int, process: subprocess.Popen[bytes]) -> None:
    assert kernel32 is not None
    process_handle = int(process._handle)  # CPython Windows Popen native HANDLE.
    if not kernel32.AssignProcessToJobObject(
        wintypes.HANDLE(handle),
        wintypes.HANDLE(process_handle),
    ):
        raise _win_error("AssignProcessToJobObject")


def output_usage(root: pathlib.Path) -> tuple[int, int]:
    total = 0
    count = 0
    if not root.exists():
        return 0, 0
    for path in root.rglob("*"):
        if path.is_file():
            count += 1
            total += path.stat().st_size
    return total, count


def run_fenced(
    mode: str,
    *,
    root: pathlib.Path,
    wall_time_ms: int,
    cpu_time_ms: int,
    memory_bytes: int,
    output_bytes: int,
    artifact_count: int,
    cancel_after_ms: int | None = None,
) -> dict[str, object]:
    output = root / mode
    output.mkdir(parents=True, exist_ok=True)
    command = [
        sys.executable,
        str(pathlib.Path(__file__).resolve()),
        "--child",
        mode,
        "--output",
        str(output),
    ]

    job = create_job(cpu_time_ms=cpu_time_ms, memory_bytes=memory_bytes)
    started = time.monotonic()
    process: subprocess.Popen[bytes] | None = None
    cancelled = False
    timed_out = False
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        assign_job(job, process)

        deadline = started + wall_time_ms / 1000.0
        cancel_deadline = (
            started + cancel_after_ms / 1000.0
            if cancel_after_ms is not None
            else None
        )

        while process.poll() is None:
            now = time.monotonic()
            if cancel_deadline is not None and now >= cancel_deadline:
                cancelled = True
                terminate_job(job, 1223)
                break
            if now >= deadline:
                timed_out = True
                terminate_job(job, 1460)
                break
            time.sleep(0.01)

        stdout, stderr = process.communicate(timeout=5)
        elapsed_ms = int((time.monotonic() - started) * 1000)
        used_bytes, used_artifacts = output_usage(output)

        if cancelled:
            status = STATUS_CANCELLED
            reason = "explicit_cancel"
        elif timed_out:
            status = STATUS_TIMED_OUT
            reason = "wall_time"
        elif used_bytes > output_bytes or used_artifacts > artifact_count:
            status = STATUS_RESOURCE_LIMITED
            reason = "output_limit"
        elif mode in {"memory", "cpu"} and process.returncode != 0:
            status = STATUS_RESOURCE_LIMITED
            reason = f"{mode}_limit"
        elif process.returncode == 0:
            status = STATUS_SUCCESS
            reason = "completed"
        else:
            status = STATUS_FAILED
            reason = "child_failure"

        return {
            "mode": mode,
            "status": status,
            "reason": reason,
            "exit_code": process.returncode,
            "elapsed_ms": elapsed_ms,
            "output_bytes": used_bytes,
            "artifact_count": used_artifacts,
            "stdout": stdout.decode("utf-8", errors="replace")[-4096:],
            "stderr": stderr.decode("utf-8", errors="replace")[-4096:],
        }
    finally:
        if process is not None and process.poll() is None:
            terminate_job(job, 1)
            process.wait(timeout=5)
        close_job(job)


def child(mode: str, output: pathlib.Path) -> int:
    output.mkdir(parents=True, exist_ok=True)
    if mode == "success":
        (output / "ok.bin").write_bytes(b"chaptera-rescue-fence-ok")
        return 0
    if mode in {"sleep", "cancel"}:
        time.sleep(30)
        return 0
    if mode == "crash":
        os._exit(17)
    if mode == "output":
        (output / "too-large.bin").write_bytes(b"x" * (2 * 1024 * 1024))
        return 0
    if mode == "memory":
        blocks: list[bytearray] = []
        try:
            while True:
                blocks.append(bytearray(4 * 1024 * 1024))
        except MemoryError:
            return 88
    if mode == "cpu":
        value = 0
        while True:
            value = (value + 1) & 0xFFFFFFFF
    raise RuntimeError(f"unknown child mode: {mode}")


def acceptance() -> dict[str, object]:
    _require_windows()
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        source = root / "source.pub"
        source.write_bytes(b"synthetic immutable source for Windows fence acceptance")
        source_before = sha256(source)

        common = dict(
            root=root / "jobs",
            wall_time_ms=5000,
            cpu_time_ms=1000,
            memory_bytes=64 * 1024 * 1024,
            output_bytes=64 * 1024,
            artifact_count=8,
        )
        results = [
            run_fenced("success", **common),
            run_fenced("sleep", **{**common, "wall_time_ms": 200}),
            run_fenced("cancel", **common, cancel_after_ms=200),
            run_fenced("memory", **common),
            run_fenced("cpu", **common),
            run_fenced("output", **common),
            run_fenced("crash", **common),
        ]

        expected = {
            "success": STATUS_SUCCESS,
            "sleep": STATUS_TIMED_OUT,
            "cancel": STATUS_CANCELLED,
            "memory": STATUS_RESOURCE_LIMITED,
            "cpu": STATUS_RESOURCE_LIMITED,
            "output": STATUS_RESOURCE_LIMITED,
            "crash": STATUS_FAILED,
        }
        actual = {str(row["mode"]): row["status"] for row in results}
        if actual != expected:
            raise AssertionError(f"typed Windows fence outcomes mismatch: {actual!r}")

        source_after = sha256(source)
        if source_before != source_after:
            raise AssertionError("Windows fence acceptance mutated source bytes")

        return {
            "schema_version": "chaptera.rescue-worker-win-fence-acceptance.v1",
            "source_unchanged": True,
            "job_object": {
                "kill_on_close": True,
                "process_memory_limit": True,
                "process_cpu_time_limit": True,
                "parent_wall_timeout": True,
                "explicit_cancel": True,
                "post_run_output_limits": True,
            },
            "typed_outcomes": expected,
            "results": results,
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--child", choices=["success", "sleep", "cancel", "memory", "cpu", "output", "crash"])
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--output-json", type=pathlib.Path)
    args = parser.parse_args()

    if args.child:
        if args.output is None:
            parser.error("--output is required with --child")
        return child(args.child, args.output)

    result = acceptance()
    rendered = json.dumps(result, indent=2, sort_keys=True)
    if args.output_json is not None:
        args.output_json.parent.mkdir(parents=True, exist_ok=True)
        args.output_json.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
