#!/usr/bin/env python3
import errno
import json
import os
import pathlib
import sys
import tempfile
import textwrap
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from migration_pdf_worker_isolation import (
    WorkerLimits,
    run_isolated_batch,
    run_isolated_worker,
)


VALID_WORKER = r"""#!/usr/bin/env python3
import json
import os
import pathlib
import resource

out = pathlib.Path(os.environ["CHAPTERA_WORKER_OUTPUT_DIR"])
out.mkdir(parents=True, exist_ok=True)
(out / "document.pdf").write_bytes(b"%PDF-1.4\n% bounded test\n")
(out / "loss.json").write_text(json.dumps({"loss": []}), encoding="utf-8")
(out / "limits.json").write_text(json.dumps({
    "as": list(resource.getrlimit(resource.RLIMIT_AS)),
    "cpu": list(resource.getrlimit(resource.RLIMIT_CPU)),
    "nofile": list(resource.getrlimit(resource.RLIMIT_NOFILE)),
    "fsize": list(resource.getrlimit(resource.RLIMIT_FSIZE)),
    "network_policy": os.environ.get("CHAPTERA_NETWORK_POLICY"),
}), encoding="utf-8")
"""

NETWORK_PROBE = r"""#!/usr/bin/env python3
import errno
import json
import os
import pathlib
import socket

out = pathlib.Path(os.environ["CHAPTERA_WORKER_OUTPUT_DIR"])
out.mkdir(parents=True, exist_ok=True)
blocked = False
error_number = None
try:
    socket.socket(socket.AF_INET, socket.SOCK_STREAM)
except OSError as exc:
    error_number = exc.errno
    blocked = exc.errno == errno.EPERM

(out / "network.json").write_text(json.dumps({
    "blocked": blocked,
    "errno": error_number,
}), encoding="utf-8")
raise SystemExit(0 if blocked else 9)
"""

FAIL_AFTER_PARTIAL = r"""#!/usr/bin/env python3
import os
import pathlib

out = pathlib.Path(os.environ["CHAPTERA_WORKER_OUTPUT_DIR"])
out.mkdir(parents=True, exist_ok=True)
(out / "partial.pdf").write_bytes(b"partial")
raise SystemExit(23)
"""

SLEEP_WORKER = r"""#!/usr/bin/env python3
import os
import pathlib
import time

time.sleep(10)
out = pathlib.Path(os.environ["CHAPTERA_WORKER_OUTPUT_DIR"])
out.mkdir(parents=True, exist_ok=True)
(out / "late.pdf").write_bytes(b"late")
"""


@unittest.skipUnless(sys.platform == "linux", "Linux security slice")
class MigrationPdfWorkerIsolationTests(unittest.TestCase):
    def write_worker(self, root, name, content):
        path = pathlib.Path(root) / name
        path.write_text(content, encoding="utf-8")
        return path

    def limits(self):
        return WorkerLimits(
            address_space_bytes=384 * 1024 * 1024,
            cpu_seconds=5,
            open_files=48,
            output_file_bytes=8 * 1024 * 1024,
        )

    def test_valid_worker_publishes_only_after_success_and_inherits_limits(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            worker = self.write_worker(tmp, "valid.py", VALID_WORKER)
            final = tmp / "published"
            result = run_isolated_worker(
                [sys.executable, str(worker)],
                final_output_dir=final,
                timeout_seconds=5,
                limits=self.limits(),
            )

            self.assertTrue(result.succeeded)
            self.assertFalse(result.timed_out)
            self.assertEqual("seccomp_default_deny", result.network_policy)
            self.assertEqual(
                ("document.pdf", "limits.json", "loss.json"),
                result.outputs,
            )
            self.assertTrue((final / "document.pdf").is_file())
            limits = json.loads((final / "limits.json").read_text(encoding="utf-8"))
            self.assertEqual([384 * 1024 * 1024, 384 * 1024 * 1024], limits["as"])
            self.assertEqual([5, 6], limits["cpu"])
            self.assertEqual([48, 48], limits["nofile"])
            self.assertEqual([8 * 1024 * 1024, 8 * 1024 * 1024], limits["fsize"])
            self.assertEqual("seccomp_default_deny", limits["network_policy"])

    def test_network_socket_creation_is_denied_before_worker_executes_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            worker = self.write_worker(tmp, "network_probe.py", NETWORK_PROBE)
            final = tmp / "network-result"
            result = run_isolated_worker(
                [sys.executable, str(worker)],
                final_output_dir=final,
                timeout_seconds=5,
                limits=self.limits(),
            )

            self.assertTrue(result.succeeded, result.stderr_tail)
            receipt = json.loads((final / "network.json").read_text(encoding="utf-8"))
            self.assertTrue(receipt["blocked"])
            self.assertEqual(errno.EPERM, receipt["errno"])

    def test_forced_zero_timeout_never_publishes_or_runs_worker(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            marker = tmp / "should-not-exist"
            worker = self.write_worker(
                tmp,
                "marker.py",
                textwrap.dedent(
                    f"""\
                    import pathlib
                    pathlib.Path({str(marker)!r}).write_text("ran")
                    """
                ),
            )
            final = tmp / "zero-timeout"
            result = run_isolated_worker(
                [sys.executable, str(worker)],
                final_output_dir=final,
                timeout_seconds=0,
                limits=self.limits(),
            )

            self.assertEqual("timeout", result.status)
            self.assertTrue(result.timed_out)
            self.assertTrue(result.staging_cleaned)
            self.assertFalse(final.exists())
            self.assertFalse(marker.exists())

    def test_wall_timeout_kills_worker_group_and_removes_partial_staging(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            worker = self.write_worker(tmp, "sleep.py", SLEEP_WORKER)
            final = tmp / "timeout-result"
            result = run_isolated_worker(
                [sys.executable, str(worker)],
                final_output_dir=final,
                timeout_seconds=0.2,
                limits=self.limits(),
            )

            self.assertEqual("timeout", result.status)
            self.assertTrue(result.timed_out)
            self.assertTrue(result.staging_cleaned)
            self.assertFalse(final.exists())

    def test_failed_file_does_not_publish_partial_output_and_later_job_still_runs(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            bad = self.write_worker(tmp, "bad.py", FAIL_AFTER_PARTIAL)
            good = self.write_worker(tmp, "good.py", VALID_WORKER)
            bad_final = tmp / "bad-output"
            good_final = tmp / "good-output"

            rows = run_isolated_batch(
                [
                    {
                        "name": "malformed",
                        "command": [sys.executable, str(bad)],
                        "final_output_dir": bad_final,
                    },
                    {
                        "name": "valid-after-malformed",
                        "command": [sys.executable, str(good)],
                        "final_output_dir": good_final,
                    },
                ],
                timeout_seconds=5,
                limits=self.limits(),
            )

            self.assertEqual("failed", rows[0]["status"])
            self.assertEqual(23, rows[0]["exit_code"])
            self.assertTrue(rows[0]["staging_cleaned"])
            self.assertFalse(bad_final.exists())
            self.assertEqual("success", rows[1]["status"])
            self.assertTrue((good_final / "document.pdf").is_file())

    def test_output_file_limit_fails_closed_without_publication(self):
        oversized = r"""#!/usr/bin/env python3
import os
import pathlib
out = pathlib.Path(os.environ["CHAPTERA_WORKER_OUTPUT_DIR"])
out.mkdir(parents=True, exist_ok=True)
with (out / "too-large.bin").open("wb") as fh:
    fh.write(b"x" * (2 * 1024 * 1024))
"""
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            worker = self.write_worker(tmp, "oversized.py", oversized)
            final = tmp / "oversized-output"
            limits = WorkerLimits(
                address_space_bytes=384 * 1024 * 1024,
                cpu_seconds=5,
                open_files=48,
                output_file_bytes=1024 * 1024,
            )
            result = run_isolated_worker(
                [sys.executable, str(worker)],
                final_output_dir=final,
                timeout_seconds=5,
                limits=limits,
            )
            self.assertEqual("failed", result.status)
            self.assertTrue(result.staging_cleaned)
            self.assertFalse(final.exists())


if __name__ == "__main__":
    unittest.main()
