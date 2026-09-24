#!/usr/bin/env python3
import json
import pathlib
import sys

ROOT=pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/"services"/"cloud-export"))
from export_job_v1 import CloudExportJobServiceV1, ExportConflict, ExportRejected

OUT=ROOT/"target"/"cloud-export-job-v1"/"receipt.json"
REV="sha256:"+"a"*64
LAYOUT="sha256:"+"b"*64
auth_state={"allow":True}
def authz(action,job):
    return auth_state["allow"]

svc=CloudExportJobServiceV1()
job=svc.create_export(
    tenant_id="tenant:1",document_id="doc:1",exact_revision_id=REV,
    target_profile="pdf:v1",layout_environment_id=LAYOUT,
    client_request_id="receipt-export-0001",authz=authz
)
retry=svc.create_export(
    tenant_id="tenant:1",document_id="doc:1",exact_revision_id=REV,
    target_profile="pdf:v1",layout_environment_id=LAYOUT,
    client_request_id="receipt-export-0001",authz=authz
)
claim=svc.claim(job_id=job["job_id"],worker_id="worker:a",authz=authz)
svc.expire_lease(job_id=job["job_id"],expected_lease_generation=claim["lease_generation"])
claim2=svc.claim(job_id=job["job_id"],worker_id="worker:b",authz=authz)
published=svc.publish(
    job_id=job["job_id"],worker_id="worker:b",
    lease_generation=claim2["lease_generation"],
    artifact_content_hash="c"*64,loss_report_hash="d"*64,authz=authz
)
download=svc.authorize_download(job_id=job["job_id"],authz=authz)
auth_state["allow"]=False
download_revoked=False
try:
    svc.authorize_download(job_id=job["job_id"],authz=authz)
except ExportRejected as e:
    download_revoked=str(e)=="authz_denied"
auth_state["allow"]=True
expired=svc.expire_artifact(job_id=job["job_id"])
expired_denied=False
try:
    svc.authorize_download(job_id=job["job_id"],authz=authz)
except ExportRejected as e:
    expired_denied=str(e)=="artifact_expired"

cancel_svc=CloudExportJobServiceV1()
cancel=cancel_svc.create_export(
    tenant_id="tenant:1",document_id="doc:1",exact_revision_id=REV,
    target_profile="pdf:v1",layout_environment_id=LAYOUT,
    client_request_id="receipt-cancel-0001",authz=lambda a,j: True
)
cancel_claim=cancel_svc.claim(job_id=cancel["job_id"],worker_id="w",authz=lambda a,j: True)
cancel_svc.request_cancel(job_id=cancel["job_id"])
cancel_before_publish=False
try:
    cancel_svc.publish(
        job_id=cancel["job_id"],worker_id="w",
        lease_generation=cancel_claim["lease_generation"],
        artifact_content_hash="c"*64,loss_report_hash="d"*64,authz=lambda a,j: True
    )
except ExportRejected as e:
    cancel_before_publish=str(e)=="cancelled_before_publish"

receipt={
  "receipt_kind":"chaptera.cloud-export-job-v1.contract",
  "real_exporter":False,
  "product_acceptance":False,
  "invariants":{
    "create_retry_same_job":retry["job_id"]==job["job_id"],
    "exact_revision_preserved":published["exact_revision_id"]==REV==download["exact_revision_id"],
    "lease_generation_advanced_on_reclaim":claim2["lease_generation"]==2,
    "cancel_before_publish_wins":cancel_before_publish,
    "revoked_download_denied":download_revoked,
    "expiry_preserves_succeeded_history":expired["state"]=="succeeded" and expired["artifact_expired"],
    "expired_download_denied":expired_denied,
    "logical_binding_present":published["artifact_binding_id"] is not None,
    "content_hash_not_binding_id":published["artifact_binding_id"]!="c"*64
  },
  "guardrail":"Public orchestration reference only; real exporter/queue/blob/AuthZ latency not measured."
}
assert all(receipt["invariants"].values()),receipt
OUT.parent.mkdir(parents=True,exist_ok=True)
OUT.write_text(json.dumps(receipt,indent=2)+"\n")
print(json.dumps(receipt,indent=2))
