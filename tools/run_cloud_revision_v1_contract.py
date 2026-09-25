#!/usr/bin/env python3
import copy
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "services" / "editor-api"))
sys.path.insert(0, str(ROOT / "services" / "cloud-export"))

from cloud_revision_v1 import (
    AUTHORING_REVISION_SCHEMA_V1,
    CloudRevisionKernelV1,
    DerivedArtifactFenceV1,
    DerivedArtifactStoreV1,
)
from export_job_v1 import CloudExportJobServiceV1

OUT = ROOT / "target" / "cloud-revision-v1" / "receipt.json"
DOCUMENT_ID = "10000000-0000-4000-8000-000000000001"
NODE_ID = "30000000-0000-4000-8000-000000000001"
SOURCE_HASH = "a" * 64
CANONICAL_BASE = "1" * 64
CANONICAL_CHILD = "2" * 64
ENV = "sha256:" + "3" * 64
INPUT = "sha256:" + "4" * 64

kernel = CloudRevisionKernelV1()
baseline_project = {
    "schema_version": "pub-editor-v0.4",
    "source_hash": SOURCE_HASH,
    "operations": [],
    "nodes": {
        NODE_ID: {
            "bounds": {"x": 10, "y": 20, "width": 300, "height": 200}
        }
    },
}
baseline = kernel.register_canonical_baseline(
    document_id=DOCUMENT_ID,
    source_hash=SOURCE_HASH,
    project=baseline_project,
    canonical_revision_id=CANONICAL_BASE,
)
executor_calls = 0

def executor(base_project, diff):
    global executor_calls
    executor_calls += 1
    op = diff["operations"][0]
    project = copy.deepcopy(base_project)
    assert project["nodes"][op["node_id"]]["bounds"] == op["before"]
    project["nodes"][op["node_id"]]["bounds"] = copy.deepcopy(op["after"])
    project["operations"] = list(project["operations"]) + copy.deepcopy(diff["operations"])
    return (
        {
            "schema_version": AUTHORING_REVISION_SCHEMA_V1,
            "revision_id": CANONICAL_CHILD,
            "parent_revision_id": CANONICAL_BASE,
        },
        project,
        [{"key": "canonical.diff", "state": "supported", "note": None}],
    )

request = {
    "protocol_version": "chaptera.semantic-diff-commit.v1",
    "document_id": DOCUMENT_ID,
    "source_hash": SOURCE_HASH,
    "base_revision_id": baseline.revision_id,
    "client_operation_id": "90000000-0000-4000-8000-000000000001",
    "command": {
        "kind": "apply_semantic_diff_v1",
        "semantic_diff": {
            "schema_version": AUTHORING_REVISION_SCHEMA_V1,
            "base_revision_id": CANONICAL_BASE,
            "operations": [{
                "kind": "update_node_bounds",
                "node_id": NODE_ID,
                "before": {"x": 10, "y": 20, "width": 300, "height": 200},
                "after": {"x": 30, "y": 40, "width": 300, "height": 200},
            }],
        },
    },
}

accepted = kernel.commit_semantic_diff(copy.deepcopy(request), executor)
retry = kernel.commit_semantic_diff(copy.deepcopy(request), executor)

artifacts = DerivedArtifactStoreV1()
scene_fence = DerivedArtifactFenceV1(
    document_id=DOCUMENT_ID,
    service_revision_id=accepted["revision_id"],
    canonical_revision_id=accepted["canonical_revision_id"],
    stage="scene",
    stage_version="scene-v1",
    environment_fingerprint=ENV,
    input_fingerprint=INPUT,
)
preview_fence = DerivedArtifactFenceV1(
    document_id=DOCUMENT_ID,
    service_revision_id=accepted["revision_id"],
    canonical_revision_id=accepted["canonical_revision_id"],
    stage="preview",
    stage_version="preview-v1",
    environment_fingerprint=ENV,
    input_fingerprint=scene_fence.fence_id(),
)
export_fence = DerivedArtifactFenceV1(
    document_id=DOCUMENT_ID,
    service_revision_id=accepted["revision_id"],
    canonical_revision_id=accepted["canonical_revision_id"],
    stage="export",
    stage_version="pdf-v1",
    environment_fingerprint=ENV,
    input_fingerprint=preview_fence.fence_id(),
)
scene_key = artifacts.publish(scene_fence, "sha256:" + "5" * 64)
preview_key = artifacts.publish(preview_fence, "sha256:" + "6" * 64)
export_key = artifacts.publish(export_fence, "sha256:" + "7" * 64)

exports = CloudExportJobServiceV1()
allow = lambda action, job: True
job = exports.create_export(
    tenant_id="tenant:1",
    document_id=DOCUMENT_ID,
    exact_revision_id=accepted["canonical_revision_id"],
    target_profile="pdf:v1",
    layout_environment_id=ENV,
    client_request_id="cloud-revision-receipt-export",
    authz=allow,
)
claim = exports.claim(job_id=job["job_id"], worker_id="worker:1", authz=allow)
published = exports.publish(
    job_id=job["job_id"],
    worker_id="worker:1",
    lease_generation=claim["lease_generation"],
    artifact_content_hash="7" * 64,
    loss_report_hash="8" * 64,
    authz=allow,
)

changed_environment = DerivedArtifactFenceV1(
    document_id=DOCUMENT_ID,
    service_revision_id=accepted["revision_id"],
    canonical_revision_id=accepted["canonical_revision_id"],
    stage="scene",
    stage_version="scene-v1",
    environment_fingerprint="sha256:" + "9" * 64,
    input_fingerprint=INPUT,
)

receipt = {
    "receipt_kind": "chaptera.cloud-revision-v1.contract",
    "canonical_revision_model_authority": "HeisLuka/pub-rs REVISION-MODEL-01",
    "real_pub": False,
    "deployed_service": False,
    "product_acceptance": False,
    "service_revision_id": accepted["revision_id"],
    "canonical_revision_id": accepted["canonical_revision_id"],
    "canonical_parent_revision_id": accepted["canonical_parent_revision_id"],
    "artifact_fences": {
        "scene": scene_key,
        "preview": preview_key,
        "export": export_key,
    },
    "export_job_id": published["job_id"],
    "invariants": {
        "service_and_canonical_identity_are_distinct": accepted["revision_id"] != accepted["canonical_revision_id"],
        "canonical_child_parent_is_exact": accepted["canonical_parent_revision_id"] == CANONICAL_BASE,
        "exact_retry_is_idempotent": accepted == retry and executor_calls == 1,
        "scene_exact_fence_resolves": artifacts.resolve(scene_fence) == "sha256:" + "5" * 64,
        "environment_change_requires_rebuild": artifacts.rebuild_required(changed_environment),
        "export_uses_canonical_exact_revision": published["exact_revision_id"] == CANONICAL_CHILD,
        "export_uses_same_environment": published["layout_environment_id"] == ENV,
    },
    "guardrail": (
        "Executable Rar service-contract integration only; production Chaptera "
        "durable Scene/preview persistence and deployed worker-route proof remain."
    ),
}
assert all(receipt["invariants"].values()), receipt
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
print(json.dumps(receipt, indent=2, sort_keys=True))
