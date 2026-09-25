#!/usr/bin/env python3
"""Produce the public-safe CLOUD-SERVER-RUNTIME-01 CI receipt."""

from __future__ import annotations

import argparse
import json
import os
import pathlib


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--live-code", type=int, required=True)
    parser.add_argument("--ready-code", type=int, required=True)
    parser.add_argument("--doctor-rc", type=int, required=True)
    parser.add_argument("--worker-rc", type=int, required=True)
    parser.add_argument("--migrate-status-rc", type=int, required=True)
    parser.add_argument("--migrate-up-rc", type=int, required=True)
    parser.add_argument("--server-rc", type=int, required=True)
    args = parser.parse_args()

    if args.live_code != 200:
        raise SystemExit(f"/live expected 200, got {args.live_code}")
    if args.ready_code != 503:
        raise SystemExit(f"/ready expected 503 in the no-config shell smoke, got {args.ready_code}")

    fail_closed = {
        "doctor": args.doctor_rc,
        "worker": args.worker_rc,
        "migrate_status": args.migrate_status_rc,
        "migrate_up": args.migrate_up_rc,
    }
    for command, rc in fail_closed.items():
        if rc == 0:
            raise SystemExit(f"{command} unexpectedly succeeded without producer adapter")

    if args.server_rc != 0:
        raise SystemExit(f"server did not exit cleanly after SIGTERM: rc={args.server_rc}")

    github_sha = os.environ.get("GITHUB_SHA", "unknown")
    if github_sha != "unknown" and github_sha[:12] not in args.version:
        raise SystemExit(
            "binary version output does not contain the current commit identity: "
            f"version={args.version!r} sha={github_sha!r}"
        )

    receipt = {
        "schema": "chaptera.cloud-server-runtime-01.receipt.v1",
        "task": "CLOUD-SERVER-RUNTIME-01",
        "public_safe": True,
        "git_sha": github_sha,
        "binary_version": args.version,
        "runtime_shell_implemented": True,
        "production_ready": False,
        "http": {
            "live": args.live_code,
            "ready_without_producers": args.ready_code,
        },
        "fail_closed_commands": fail_closed,
        "graceful_shutdown_rc": args.server_rc,
        "dependency_seams": [
            "authn",
            "authz",
            "revision_stream",
            "jobs",
            "blob_store",
            "observability",
        ],
        "limitations": [
            "This receipt covers the no-config runtime shell, whose RuntimePorts remain intentionally unconfigured.",
            "Configured production producer composition is exercised by the typed production-config acceptance.",
            "No production AuthN/AuthZ adapter is connected.",
            "No durable async worker adapter is connected.",
            "No production BlobStore adapter is connected.",
            "A real deployment must remain not-ready until all required producers are wired.",
        ],
    }

    target = pathlib.Path("target/cloud-server-runtime-01")
    target.mkdir(parents=True, exist_ok=True)
    path = target / "receipt.json"
    path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
