#!/usr/bin/env python3
"""Exercise the operator migration surface and emit a public-safe receipt."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import sqlite3
import subprocess
import tempfile


def run(binary: pathlib.Path, db: pathlib.Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["CHAPTERA_SQLITE_PATH"] = str(db)
    return subprocess.run(
        [str(binary), *args],
        check=check,
        capture_output=True,
        text=True,
        env=env,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()

    binary = args.binary.resolve()
    out = args.out
    out.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="chaptera-migration-") as temp:
        db = pathlib.Path(temp) / "chaptera.sqlite"

        before_proc = run(binary, db, "migrate", "status")
        before = json.loads(before_proc.stdout)
        if db.exists():
            raise SystemExit("migrate status created an absent database")

        up_proc = run(binary, db, "migrate", "up")
        up = json.loads(up_proc.stdout)
        current_proc = run(binary, db, "migrate", "status")
        current = json.loads(current_proc.stdout)

        assert before["state"] == "pending"
        assert before["current_version"] == 0
        expected_versions = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
        assert before["target_version"] == 14
        assert before["pending_versions"] == expected_versions

        assert up["state"] == "current"
        assert up["target_version"] == 14
        assert up["current_version"] == 14
        assert up["applied_versions"] == expected_versions
        assert current == up

        with sqlite3.connect(db) as connection:
            tables = {
                row[0]
                for row in connection.execute(
                    "SELECT name FROM sqlite_master WHERE type='table'"
                )
            }
            for required in {
                "principals",
                "principal_identities",
                "sessions",
                "derived_artifacts",
                "quota_reservations",
                "revision_identity_bindings",
                "export_publications",
                "authz_documents",
                "authz_principal_grants",
                "authz_audit_events",
            }:
                if required not in tables:
                    raise SystemExit(f"migration chain did not materialize {required}")

            physical_blob_columns = {
                row[1]
                for row in connection.execute("PRAGMA table_info(physical_blobs)")
            }
            for required_column in {"gc_delete_fence", "gc_fenced_at_ms"}:
                if required_column not in physical_blob_columns:
                    raise SystemExit(
                        f"migration chain did not materialize physical_blobs.{required_column}"
                    )

            connection.execute(
                "UPDATE chaptera_schema_migrations "
                "SET checksum_sha256='tampered' WHERE version=2"
            )
            connection.commit()

        tampered = run(binary, db, "migrate", "status", check=False)
        if tampered.returncode == 0:
            raise SystemExit("checksum-tampered migration history was accepted")

    receipt = {
        "schema": "chaptera.cloud-migration-01.receipt.v1",
        "task": "CLOUD-MIGRATION-01",
        "git_sha": os.environ.get("GITHUB_SHA", "unknown"),
        "public_safe": True,
        "operator_surface": [
            "chaptera migrate status",
            "chaptera migrate up",
        ],
        "before": before,
        "after": current,
        "checksum_tamper_rejected": True,
        "automatic_startup_migration": False,
        "single_owner_lock": "BEGIN IMMEDIATE",
        "rollback_policy": (
            "restore compatible DB unless a cross-schema binary window "
            "is explicitly declared"
        ),
    }

    (out / "status-before.json").write_text(
        json.dumps(before, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (out / "status-current.json").write_text(
        json.dumps(current, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (out / "receipt.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
