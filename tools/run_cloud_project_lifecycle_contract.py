#!/usr/bin/env python3
import json
import pathlib
import sys

ROOT=pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/"services"/"cloud-project"))
from lifecycle_v1 import CloudProjectLifecycleV1, LifecycleConflict, LifecycleRejected

OUT=ROOT/"target"/"cloud-project-lifecycle-v1"/"receipt.json"
svc=CloudProjectLifecycleV1()
created=svc.create_from_upload(
    tenant_id="tenant:1",workspace_id="workspace:a",
    source_hash="a"*64,initial_revision_id="sha256:"+"b"*64,
    name="Newsletter",request_id="create-receipt-0001",
    owner_grant="owner:alice",asset_bindings=("asset:1","asset:2")
)
renamed=svc.rename(
    project_id=created["project_id"],expected_lifecycle_generation=0,
    expected_metadata_version=0,name="Newsletter 2026",request_id="rename-receipt-0001"
)
stale_metadata=False
try:
    svc.rename(
        project_id=created["project_id"],expected_lifecycle_generation=0,
        expected_metadata_version=0,name="stale",request_id="rename-receipt-0002"
    )
except LifecycleConflict as e:
    stale_metadata=str(e)=="stale_metadata_version"
trashed=svc.trash(
    project_id=created["project_id"],expected_lifecycle_generation=0,
    request_id="trash-receipt-0001"
)
stale_lifecycle=False
try:
    svc.restore(
        project_id=created["project_id"],expected_lifecycle_generation=0,
        request_id="restore-stale-0001"
    )
except LifecycleConflict as e:
    stale_lifecycle=str(e)=="stale_lifecycle_generation"
restored=svc.restore(
    project_id=created["project_id"],expected_lifecycle_generation=1,
    request_id="restore-receipt-0001"
)
fork=svc.fork_from_revision(
    source_project_id=created["project_id"],selected_revision_id=created["current_revision_id"],
    target_workspace_id="workspace:b",request_id="fork-receipt-0001"
)
trashed2=svc.trash(
    project_id=created["project_id"],expected_lifecycle_generation=2,
    request_id="trash-receipt-0002"
)
deleted=svc.hard_delete(
    project_id=created["project_id"],expected_lifecycle_generation=3,
    request_id="delete-receipt-0001"
)
terminal=False
try:
    svc.rename(
        project_id=created["project_id"],
        expected_lifecycle_generation=deleted["lifecycle_generation"],
        expected_metadata_version=deleted["metadata_version"],
        name="resurrect",request_id="rename-deleted-0001"
    )
except LifecycleRejected as e:
    terminal=str(e)=="deleted"

receipt={
    "receipt_kind":"chaptera.cloud-project-lifecycle-v1.contract",
    "real_service":False,
    "product_acceptance":False,
    "invariants":{
        "rename_preserves_document_id":renamed["document_id"]==created["document_id"],
        "rename_preserves_revision":renamed["current_revision_id"]==created["current_revision_id"],
        "rename_preserves_lifecycle_generation":renamed["lifecycle_generation"]==0,
        "rename_advances_metadata_version":renamed["metadata_version"]==1,
        "stale_metadata_fails_closed":stale_metadata,
        "trash_restore_preserve_identity":restored["document_id"]==created["document_id"],
        "stale_lifecycle_fails_closed":stale_lifecycle,
        "fork_new_project_id":fork["project_id"]!=created["project_id"],
        "fork_new_document_id":fork["document_id"]!=created["document_id"],
        "fork_new_genesis_revision":fork["current_revision_id"]!=created["current_revision_id"],
        "fork_does_not_inherit_grants":fork["grants"]==(),
        "fork_does_not_inherit_comments":fork["comments_inherited"] is False,
        "hard_delete_terminal":terminal
    },
    "guardrail":"Public reference contract only; no deployed storage/AuthZ/region claim."
}
assert all(receipt["invariants"].values()),receipt
OUT.parent.mkdir(parents=True,exist_ok=True)
OUT.write_text(json.dumps(receipt,indent=2)+"\n")
print(json.dumps(receipt,indent=2))
