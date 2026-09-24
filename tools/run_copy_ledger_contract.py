#!/usr/bin/env python3
import json
import os
import pathlib

from copy_ledger_v1 import COPY_LEDGER_SCHEMA, copy_ledger_measurement

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "copy-ledger-v1"
OUT.mkdir(parents=True, exist_ok=True)

receipt = {
    "receipt_version": COPY_LEDGER_SCHEMA,
    "runtime": {
        "runtime": "python-contract-fixture",
        "platform": "public-synthetic",
        "machine": os.environ.get("RUNNER_ARCH", "local"),
        "runner_os": os.environ.get("RUNNER_OS", "local"),
        "runner_arch": os.environ.get("RUNNER_ARCH", "local"),
    },
    "workload": {
        "fixture_binding_id": "copy-ledger-contract-fixture-v1",
        "document_class": "synthetic-medium",
        "pages": 12,
        "stories": 30,
        "resources": 8,
    },
    "evidence_authority": {
        "real_product_source_free": False,
        "synthetic_or_public_fixture": True,
    },
    "scenarios": [
        {
            "name": "small_story_edit",
            "frequency_class": "per_edit",
            "allocations": 40,
            "allocated_bytes": 12000,
            "peak_live_bytes": 8000,
            "serialized_bytes": 256,
            "tracked_unique_payload_bytes": 2048,
            "correctness": {
                "semantic_equivalent": True,
                "output_equivalent": True,
            },
            "copy_sites": [
                {
                    "site_id": "editor.history.before_after",
                    "payload_class": "story_text",
                    "classification": "AVOIDABLE_DUPLICATE",
                    "bytes_total": 4096,
                    "events": 2,
                    "measurement_method": "explicit_copy_counter",
                    "from_stage": "authoring.story",
                    "to_stage": "editor.history",
                },
                {
                    "site_id": "project.serialize",
                    "payload_class": "story_text",
                    "classification": "REQUIRED_SERIALIZATION",
                    "bytes_total": 256,
                    "events": 1,
                    "measurement_method": "serializer_count",
                    "from_stage": "editor.history",
                    "to_stage": "project.bytes",
                },
            ],
        }
    ],
    "limitations": [
        "Contract fixture only; no real Chaptera EditorSession or PUB bytes are measured here."
    ],
}

build = {
    "repository": os.environ.get("GITHUB_REPOSITORY", "HeisLuka/rar"),
    "sha": os.environ.get("GITHUB_SHA", "local-contract"),
}
measurement = copy_ledger_measurement(receipt, build)

if measurement["metrics"]["scenario.small_story_edit.avoidable_duplicate_bytes"]["value"] != 4096:
    raise AssertionError("avoidable byte total was not recomputed from copy sites")
if measurement["evidence_authority"]["technology_decision_allowed"]:
    raise AssertionError("synthetic contract fixture cannot authorize technology choice")
if not all(measurement["correctness"].values()):
    raise AssertionError("synthetic copy-ledger correctness fence failed")

(OUT / "measurement.json").write_text(
    json.dumps(measurement, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
print(json.dumps({
    "schema": measurement["schema"],
    "producer": measurement["producer"],
    "workload_identity_hash": measurement["workload_identity_hash"],
    "top_avoidable_site": measurement["copy_ledger"]["ranked_avoidable_sites"][0]["site_id"],
    "technology_decision_allowed": measurement["evidence_authority"]["technology_decision_allowed"],
}, indent=2, sort_keys=True))
