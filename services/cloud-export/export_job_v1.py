"""Public reference contract for Chaptera durable Cloud export jobs.

This module deliberately models orchestration/authorization only. It does not run
a real exporter or choose a provider queue/blob store.
"""

from __future__ import annotations

import copy
import hashlib
import json
from dataclasses import dataclass
from typing import Callable, Dict, Optional, Tuple


class ExportConflict(ValueError):
    pass


class ExportRejected(ValueError):
    pass


def _hash_request(value: dict) -> str:
    body = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(body.encode()).hexdigest()


def _stable_id(prefix: str, *parts: str) -> str:
    return f"{prefix}:" + hashlib.sha256("|".join(parts).encode()).hexdigest()[:24]


@dataclass
class ExportJob:
    job_id: str
    tenant_id: str
    document_id: str
    exact_revision_id: str
    target_profile: str
    layout_environment_id: str
    request_hash: str
    state: str = "queued"
    lease_owner: Optional[str] = None
    lease_generation: int = 0
    cancel_requested: bool = False
    artifact_binding_id: Optional[str] = None
    artifact_content_hash: Optional[str] = None
    loss_report_hash: Optional[str] = None
    artifact_expired: bool = False
    failure_code: Optional[str] = None


Authz = Callable[[str, ExportJob], bool]


class CloudExportJobServiceV1:
    def __init__(self) -> None:
        self.jobs: Dict[str, ExportJob] = {}
        self._requests: Dict[Tuple[str, str], Tuple[str, str]] = {}

    def create_export(
        self,
        *,
        tenant_id: str,
        document_id: str,
        exact_revision_id: str,
        target_profile: str,
        layout_environment_id: str,
        client_request_id: str,
        authz: Authz,
    ) -> dict:
        request = {
            "tenant_id": tenant_id,
            "document_id": document_id,
            "exact_revision_id": exact_revision_id,
            "target_profile": target_profile,
            "layout_environment_id": layout_environment_id,
        }
        request_hash = _hash_request(request)
        key = (tenant_id, client_request_id)
        prior = self._requests.get(key)
        if prior is not None:
            prior_hash, job_id = prior
            if prior_hash != request_hash:
                raise ExportConflict("idempotency_conflict")
            return self.snapshot(job_id)

        job_id = _stable_id("export-job", tenant_id, client_request_id)
        job = ExportJob(
            job_id=job_id,
            tenant_id=tenant_id,
            document_id=document_id,
            exact_revision_id=exact_revision_id,
            target_profile=target_profile,
            layout_environment_id=layout_environment_id,
            request_hash=request_hash,
        )
        if not authz("create", job):
            raise ExportRejected("authz_denied")
        self.jobs[job_id] = job
        self._requests[key] = (request_hash, job_id)
        return self.snapshot(job_id)

    def snapshot(self, job_id: str) -> dict:
        return copy.deepcopy(self.jobs[job_id].__dict__)

    def claim(
        self,
        *,
        job_id: str,
        worker_id: str,
        authz: Authz,
    ) -> dict:
        job = self.jobs[job_id]
        if job.state not in {"queued", "running"}:
            raise ExportRejected("not_claimable")
        if job.cancel_requested:
            job.state = "cancelled"
            job.lease_owner = None
            raise ExportRejected("cancelled")
        if not authz("claim", job):
            raise ExportRejected("authz_denied")
        if job.state == "running" and job.lease_owner == worker_id:
            return self.snapshot(job_id)
        if job.state == "running" and job.lease_owner is not None:
            raise ExportConflict("lease_held")
        job.state = "running"
        job.lease_owner = worker_id
        job.lease_generation += 1
        return self.snapshot(job_id)

    def expire_lease(self, *, job_id: str, expected_lease_generation: int) -> dict:
        job = self.jobs[job_id]
        if job.state != "running":
            raise ExportRejected("not_running")
        if job.lease_generation != expected_lease_generation:
            raise ExportConflict("stale_lease_generation")
        job.lease_owner = None
        job.state = "queued"
        return self.snapshot(job_id)

    def request_cancel(self, *, job_id: str) -> dict:
        job = self.jobs[job_id]
        if job.state == "queued":
            job.state = "cancelled"
            job.cancel_requested = True
            return self.snapshot(job_id)
        if job.state == "running":
            job.cancel_requested = True
            return self.snapshot(job_id)
        if job.state == "succeeded":
            raise ExportRejected("too_late")
        return self.snapshot(job_id)

    def publish(
        self,
        *,
        job_id: str,
        worker_id: str,
        lease_generation: int,
        artifact_content_hash: str,
        loss_report_hash: str,
        authz: Authz,
    ) -> dict:
        job = self.jobs[job_id]
        if job.state != "running":
            raise ExportRejected("not_running")
        if job.lease_owner != worker_id or job.lease_generation != lease_generation:
            raise ExportConflict("stale_worker_lease")
        if job.cancel_requested:
            job.state = "cancelled"
            job.lease_owner = None
            raise ExportRejected("cancelled_before_publish")
        if not authz("publish", job):
            job.state = "failed"
            job.failure_code = "authz_denied_before_publish"
            job.lease_owner = None
            raise ExportRejected("authz_denied")
        binding = _stable_id(
            "artifact-binding",
            job.tenant_id,
            job.job_id,
            job.exact_revision_id,
            artifact_content_hash,
        )
        job.artifact_binding_id = binding
        job.artifact_content_hash = artifact_content_hash
        job.loss_report_hash = loss_report_hash
        job.state = "succeeded"
        job.lease_owner = None
        return self.snapshot(job_id)

    def fail(
        self,
        *,
        job_id: str,
        worker_id: str,
        lease_generation: int,
        code: str,
    ) -> dict:
        job = self.jobs[job_id]
        if job.state != "running":
            raise ExportRejected("not_running")
        if job.lease_owner != worker_id or job.lease_generation != lease_generation:
            raise ExportConflict("stale_worker_lease")
        job.state = "failed"
        job.failure_code = code
        job.lease_owner = None
        return self.snapshot(job_id)

    def expire_artifact(self, *, job_id: str) -> dict:
        job = self.jobs[job_id]
        if job.state != "succeeded":
            raise ExportRejected("artifact_not_ready")
        job.artifact_expired = True
        return self.snapshot(job_id)

    def authorize_download(self, *, job_id: str, authz: Authz) -> dict:
        job = self.jobs[job_id]
        if job.state != "succeeded" or job.artifact_binding_id is None:
            raise ExportRejected("artifact_not_ready")
        if job.artifact_expired:
            raise ExportRejected("artifact_expired")
        if not authz("download", job):
            raise ExportRejected("authz_denied")
        return {
            "job_id": job.job_id,
            "artifact_binding_id": job.artifact_binding_id,
            "artifact_content_hash": job.artifact_content_hash,
            "exact_revision_id": job.exact_revision_id,
            "target_profile": job.target_profile,
            "layout_environment_id": job.layout_environment_id,
            "loss_report_hash": job.loss_report_hash,
        }
