#!/usr/bin/env python3
"""Smoke the real Chaptera ResizeNode producer bridge without emitting a receipt.

This is hosted CI validation of the executable command surface only. It must not
be used as or relabeled into the local_private producer receipt owned by
EDITOR-RESIZE-01.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
from typing import Any

PROBES = (
    "identical_bounds",
    "pure_move",
    "non_positive_size",
    "overflow",
    "unsupported_target",
)


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def invoke(producer: pathlib.Path, fixture: pathlib.Path, payload: dict[str, Any]) -> dict[str, Any]:
    completed = subprocess.run(
        [str(producer), "--resize-producer-v1", str(fixture)],
        input=json.dumps(payload, separators=(",", ":")),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "ResizeNode producer bridge action failed"
            + (f": {completed.stderr.strip()}" if completed.stderr.strip() else "")
        )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("ResizeNode producer bridge returned invalid JSON") from error
    if not isinstance(value, dict):
        raise RuntimeError("ResizeNode producer bridge response must be an object")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--producer", required=True, type=pathlib.Path)
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    args = parser.parse_args()

    source_hash = sha256_file(args.fixture)
    baseline = invoke(
        args.producer,
        args.fixture,
        {
            "action": "baseline",
            "source_hash": source_hash,
            "fixture_kind": "real_pub_sanitized",
        },
    )
    if baseline.get("source_hash") != source_hash:
        raise RuntimeError("baseline source identity mismatch")
    if baseline.get("signed_origin_probe_passed") is not True:
        raise RuntimeError("signed-origin probe did not pass")
    candidate = baseline.get("resize_candidate")
    if not isinstance(candidate, dict):
        raise RuntimeError("baseline did not return resize_candidate")
    after = candidate.get("after")
    if not isinstance(after, dict):
        raise RuntimeError("resize candidate did not return after bounds")

    commit = invoke(
        args.producer,
        args.fixture,
        {
            "action": "commit",
            "source_hash": source_hash,
            "base_project": baseline["baseline_project"],
            "command": {
                "kind": "resize_node_to",
                "node_id": candidate["node_id"],
                "x_emu": after["x"],
                "y_emu": after["y"],
                "width_emu": after["width"],
                "height_emu": after["height"],
            },
        },
    )
    accepted = commit["resulting_project"]
    if commit.get("source_hash_after") != source_hash:
        raise RuntimeError("commit changed source identity")

    undo = invoke(
        args.producer,
        args.fixture,
        {
            "action": "history",
            "source_hash": source_hash,
            "kind": "undo",
            "base_project": accepted,
            "baseline_project": baseline["baseline_project"],
            "accepted_project": accepted,
        },
    )
    if undo.get("resulting_project") != baseline["baseline_project"]:
        raise RuntimeError("undo did not restore exact baseline")

    redo = invoke(
        args.producer,
        args.fixture,
        {
            "action": "history",
            "source_hash": source_hash,
            "kind": "redo",
            "base_project": baseline["baseline_project"],
            "baseline_project": baseline["baseline_project"],
            "accepted_project": accepted,
        },
    )
    if redo.get("resulting_project") != accepted:
        raise RuntimeError("redo did not restore exact accepted project")

    replay = invoke(
        args.producer,
        args.fixture,
        {"action": "replay", "source_hash": source_hash, "project": accepted},
    )
    if replay.get("replayed_project") != accepted:
        raise RuntimeError("fresh replay differs from accepted project")
    if replay.get("legacy_v0_4_rejected") is not True:
        raise RuntimeError("legacy project fence did not reject ResizeNode")
    if replay.get("stale_before_rejected_transactionally") is not True:
        raise RuntimeError("stale-before replay did not fail transactionally")

    export = invoke(
        args.producer,
        args.fixture,
        {"action": "export", "source_hash": source_hash, "project": accepted},
    )
    if export.get("idml_reflects_resized_bounds") is not True:
        raise RuntimeError("IDML export did not reflect resized bounds")
    if export.get("odg_reflects_resized_bounds") is not True:
        raise RuntimeError("ODG export did not reflect resized bounds")

    for probe in PROBES:
        result = invoke(
            args.producer,
            args.fixture,
            {
                "action": "probe",
                "probe": probe,
                "source_hash": source_hash,
                "baseline_project": baseline["baseline_project"],
                "resize_candidate": candidate,
            },
        )
        if result.get("rejected_no_mutation") is not True:
            raise RuntimeError(f"negative probe failed: {probe}")

    if sha256_file(args.fixture) != source_hash:
        raise RuntimeError("ResizeNode producer bridge mutated source fixture")

    print(
        json.dumps(
            {
                "schema_version": "chaptera.resize-producer-bridge-smoke.v1",
                "result": "pass",
                "actions": ["baseline", "commit", "undo", "redo", "replay", "export", *PROBES],
                "receipt_emitted": False,
            },
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
