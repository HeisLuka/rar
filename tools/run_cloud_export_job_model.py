#!/usr/bin/env python3
"""Executable architecture model for Cloud export jobs.

This is not a production service. It stress-tests the contract implied by the
Cloud Editor gap audit: exact revision pinning, durable user job identity,
idempotent create, cooperative cancellation, crash recovery, authorization
re-checks and safe artifact reuse.
"""

from __future__ import annotations

import dataclasses
import hashlib
import json
from pathlib import Path
from typing import Dict, Optional

OUT = Path("target/cloud-export-quota/export-job.json")


def h(value: object) -> str:
    raw = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(raw).hexdigest()


@dataclasses.dataclass
class Grant:
    generation: int = 1
    export_allowed: bool = True


@dataclasses.dataclass
class Job:
    job_id: str
    tenant_id: str
    document_id: str
    revision_id: str
    profile: str
    layout_environment_id: str
    client_request_id: str
    request_hash: str
    authz_generation_at_create: int
    state: str = "queued"
    attempt: int = 0
    lease_owner: Optional[str] = None
    cancel_requested: bool = False
    progress_stage: str = "queued"
    artifact_binding_id: Optional[str] = None
    failure: Optional[str] = None


@dataclasses.dataclass
class PhysicalArtifact:
    physical_id: str
    export_key: str
    content_hash: str
    bytes_len: int
    complete: bool


@dataclasses.dataclass
class ArtifactBinding:
    binding_id: str
    tenant_id: str
    job_id: str
    physical_id: str
    revision_id: str
    profile: str
    loss_report_hash: str
    expires_tick: int


class ExportModel:
    def __init__(self) -> None:
        self.jobs: Dict[str, Job] = {}
        self.idempotency: Dict[tuple[str, str], tuple[str, str]] = {}
        self.grants: Dict[tuple[str, str], Grant] = {}
        self.physical: Dict[str, PhysicalArtifact] = {}
        self.bindings: Dict[str, ArtifactBinding] = {}
        self.tick = 0

    def grant(self, tenant_id: str, document_id: str) -> Grant:
        return self.grants.setdefault((tenant_id, document_id), Grant())

    def revoke(self, tenant_id: str, document_id: str) -> None:
        g = self.grant(tenant_id, document_id)
        g.generation += 1
        g.export_allowed = False

    def regrant(self, tenant_id: str, document_id: str) -> None:
        g = self.grant(tenant_id, document_id)
        g.generation += 1
        g.export_allowed = True

    def request_hash(
        self,
        tenant_id: str,
        document_id: str,
        revision_id: str,
        profile: str,
        layout_environment_id: str,
    ) -> str:
        return h(
            {
                "tenant": tenant_id,
                "document": document_id,
                "revision": revision_id,
                "profile": profile,
                "layout_environment": layout_environment_id,
            }
        )

    def export_key(self, job: Job) -> str:
        return h(
            {
                "tenant": job.tenant_id,
                "document": job.document_id,
                "revision": job.revision_id,
                "profile": job.profile,
                "layout_environment": job.layout_environment_id,
                "exporter_version": 1,
            }
        )

    def create(
        self,
        *,
        tenant_id: str,
        document_id: str,
        revision_id: str,
        profile: str,
        layout_environment_id: str,
        client_request_id: str,
    ) -> tuple[str, Optional[Job]]:
        grant = self.grant(tenant_id, document_id)
        if not grant.export_allowed:
            return "authz_denied", None

        rh = self.request_hash(
            tenant_id, document_id, revision_id, profile, layout_environment_id
        )
        idem_key = (tenant_id, client_request_id)
        old = self.idempotency.get(idem_key)
        if old:
            old_hash, old_job_id = old
            if old_hash != rh:
                return "idempotency_conflict", None
            return "same_request", self.jobs[old_job_id]

        job_id = "job:" + h({"tenant": tenant_id, "request": client_request_id})[-24:]
        job = Job(
            job_id=job_id,
            tenant_id=tenant_id,
            document_id=document_id,
            revision_id=revision_id,
            profile=profile,
            layout_environment_id=layout_environment_id,
            client_request_id=client_request_id,
            request_hash=rh,
            authz_generation_at_create=grant.generation,
        )
        self.jobs[job_id] = job
        self.idempotency[idem_key] = (rh, job_id)
        return "created", job

    def claim(self, job_id: str, worker: str) -> str:
        job = self.jobs[job_id]
        grant = self.grant(job.tenant_id, job.document_id)
        if job.state in {"succeeded", "failed", "cancelled"}:
            return "terminal"
        if job.cancel_requested:
            job.state = "cancelled"
            job.progress_stage = "cancelled"
            return "cancelled"
        if not grant.export_allowed:
            job.state = "cancelled"
            job.failure = "authz_revoked_before_claim"
            job.progress_stage = "cancelled"
            return "authz_denied"
        if job.state == "running":
            return "already_running"

        job.state = "running"
        job.attempt += 1
        job.lease_owner = worker
        job.progress_stage = "rendering"
        return "claimed"

    def request_cancel(self, job_id: str) -> str:
        job = self.jobs[job_id]
        if job.state == "succeeded":
            return "too_late_succeeded"
        if job.state in {"failed", "cancelled"}:
            return "terminal"
        if job.state == "queued":
            job.state = "cancelled"
            job.cancel_requested = True
            job.progress_stage = "cancelled"
            return "cancelled"
        job.cancel_requested = True
        return "cancel_requested"

    def crash_worker(self, job_id: str) -> None:
        job = self.jobs[job_id]
        if job.state != "running":
            return
        job.state = "queued"
        job.lease_owner = None
        job.progress_stage = "queued"

    def stage_physical_artifact(self, job_id: str) -> PhysicalArtifact:
        job = self.jobs[job_id]
        if job.state != "running":
            raise RuntimeError("job_not_running")
        key = self.export_key(job)
        physical_id = "physical:" + key[-24:]
        artifact = PhysicalArtifact(
            physical_id=physical_id,
            export_key=key,
            content_hash=h({"export_key": key, "rendered": True}),
            bytes_len=1024 + len(job.profile) * 17,
            complete=True,
        )
        old = self.physical.get(key)
        if old is None:
            self.physical[key] = artifact
            return artifact
        if old != artifact:
            raise RuntimeError("deterministic_artifact_mismatch")
        return old

    def publish(self, job_id: str, *, expires_in: int = 100) -> str:
        job = self.jobs[job_id]
        if job.state != "running":
            return "not_running"

        grant = self.grant(job.tenant_id, job.document_id)
        if job.cancel_requested:
            job.state = "cancelled"
            job.progress_stage = "cancelled"
            job.lease_owner = None
            return "cancelled_before_publish"
        if not grant.export_allowed:
            job.state = "cancelled"
            job.failure = "authz_revoked_before_publish"
            job.progress_stage = "cancelled"
            job.lease_owner = None
            return "authz_denied_before_publish"

        physical = self.stage_physical_artifact(job_id)
        binding_id = "binding:" + h(
            {"tenant": job.tenant_id, "job": job.job_id, "physical": physical.physical_id}
        )[-24:]
        binding = ArtifactBinding(
            binding_id=binding_id,
            tenant_id=job.tenant_id,
            job_id=job.job_id,
            physical_id=physical.physical_id,
            revision_id=job.revision_id,
            profile=job.profile,
            loss_report_hash=h({"job": job.job_id, "loss": []}),
            expires_tick=self.tick + expires_in,
        )
        self.bindings[binding_id] = binding
        job.artifact_binding_id = binding_id
        job.state = "succeeded"
        job.progress_stage = "succeeded"
        job.lease_owner = None
        return "published"

    def adopt_after_crash(self, job_id: str, *, expires_in: int = 100) -> str:
        job = self.jobs[job_id]
        key = self.export_key(job)
        existing = self.physical.get(key)
        if existing is None or not existing.complete:
            return "no_complete_artifact"
        if job.state != "running":
            return "not_running"
        return self.publish(job_id, expires_in=expires_in)

    def download(self, job_id: str) -> str:
        job = self.jobs[job_id]
        if job.state != "succeeded" or not job.artifact_binding_id:
            return "not_ready"
        grant = self.grant(job.tenant_id, job.document_id)
        if not grant.export_allowed:
            return "authz_denied"
        binding = self.bindings[job.artifact_binding_id]
        if self.tick >= binding.expires_tick:
            return "expired"
        return "download_allowed"


def scenario_idempotency() -> dict:
    m = ExportModel()
    args = dict(
        tenant_id="tenant:a",
        document_id="doc:1",
        revision_id="rev:100",
        profile="pdf-high",
        layout_environment_id="env:1",
        client_request_id="req:idem-0001",
    )
    first, j1 = m.create(**args)
    second, j2 = m.create(**args)
    conflict_args = dict(args)
    conflict_args["profile"] = "pdf-small"
    conflict, _ = m.create(**conflict_args)
    assert first == "created"
    assert second == "same_request" and j1 is j2
    assert conflict == "idempotency_conflict"
    return {
        "first": first,
        "exact_retry": second,
        "same_job": j1.job_id == j2.job_id,
        "same_id_different_request": conflict,
    }


def scenario_revision_pin_and_head_move() -> dict:
    m = ExportModel()
    _, job = m.create(
        tenant_id="tenant:a",
        document_id="doc:1",
        revision_id="rev:100",
        profile="pdf-high",
        layout_environment_id="env:1",
        client_request_id="req:pin-0001",
    )
    current_head_after_create = "rev:999"
    assert m.claim(job.job_id, "worker:1") == "claimed"
    assert m.publish(job.job_id) == "published"
    binding = m.bindings[job.artifact_binding_id]
    assert binding.revision_id == "rev:100"
    assert binding.revision_id != current_head_after_create
    return {
        "requested_revision": job.revision_id,
        "current_head_after_create": current_head_after_create,
        "artifact_revision": binding.revision_id,
        "latest_was_not_substituted": True,
    }


def scenario_crash_and_adopt() -> dict:
    m = ExportModel()
    _, job = m.create(
        tenant_id="tenant:a",
        document_id="doc:1",
        revision_id="rev:100",
        profile="pdf-high",
        layout_environment_id="env:1",
        client_request_id="req:crash-0001",
    )
    assert m.claim(job.job_id, "worker:1") == "claimed"
    staged = m.stage_physical_artifact(job.job_id)
    # Worker crashes after deterministic bytes exist, before the durable job result/binding.
    m.crash_worker(job.job_id)
    assert job.state == "queued"
    assert m.claim(job.job_id, "worker:2") == "claimed"
    published = m.adopt_after_crash(job.job_id)
    assert published == "published"
    assert len(m.physical) == 1
    binding = m.bindings[job.artifact_binding_id]
    assert binding.physical_id == staged.physical_id
    return {
        "attempts": job.attempt,
        "physical_artifact_count": len(m.physical),
        "reused_deterministic_physical_artifact": True,
        "final_state": job.state,
    }


def scenario_cancel_races() -> dict:
    # queued cancel
    a = ExportModel()
    _, jq = a.create(
        tenant_id="tenant:a", document_id="doc:1", revision_id="rev:1",
        profile="pdf", layout_environment_id="env:1", client_request_id="req:cancel-q"
    )
    queued_cancel = a.request_cancel(jq.job_id)
    assert queued_cancel == "cancelled" and not a.bindings

    # running cancel before publication
    b = ExportModel()
    _, jr = b.create(
        tenant_id="tenant:a", document_id="doc:1", revision_id="rev:1",
        profile="pdf", layout_environment_id="env:1", client_request_id="req:cancel-r"
    )
    assert b.claim(jr.job_id, "worker") == "claimed"
    running_cancel = b.request_cancel(jr.job_id)
    publish_after_cancel = b.publish(jr.job_id)
    assert running_cancel == "cancel_requested"
    assert publish_after_cancel == "cancelled_before_publish"
    assert not b.bindings

    # publication wins before cancel request
    c = ExportModel()
    _, js = c.create(
        tenant_id="tenant:a", document_id="doc:1", revision_id="rev:1",
        profile="pdf", layout_environment_id="env:1", client_request_id="req:cancel-s"
    )
    assert c.claim(js.job_id, "worker") == "claimed"
    assert c.publish(js.job_id) == "published"
    late_cancel = c.request_cancel(js.job_id)
    assert late_cancel == "too_late_succeeded"
    return {
        "queued_cancel": queued_cancel,
        "running_cancel_request": running_cancel,
        "publish_after_running_cancel": publish_after_cancel,
        "late_cancel_after_publish": late_cancel,
    }


def scenario_revoke_races() -> dict:
    # revoke before claim
    a = ExportModel()
    _, j1 = a.create(
        tenant_id="tenant:a", document_id="doc:1", revision_id="rev:1",
        profile="pdf", layout_environment_id="env:1", client_request_id="req:rev-before"
    )
    a.revoke("tenant:a", "doc:1")
    before_claim = a.claim(j1.job_id, "worker")
    assert before_claim == "authz_denied"
    assert not a.bindings

    # revoke during running, before publish
    b = ExportModel()
    _, j2 = b.create(
        tenant_id="tenant:a", document_id="doc:1", revision_id="rev:1",
        profile="pdf", layout_environment_id="env:1", client_request_id="req:rev-running"
    )
    assert b.claim(j2.job_id, "worker") == "claimed"
    b.revoke("tenant:a", "doc:1")
    before_publish = b.publish(j2.job_id)
    assert before_publish == "authz_denied_before_publish"
    assert not b.bindings

    # revoke after success denies later download
    c = ExportModel()
    _, j3 = c.create(
        tenant_id="tenant:a", document_id="doc:1", revision_id="rev:1",
        profile="pdf", layout_environment_id="env:1", client_request_id="req:rev-download"
    )
    assert c.claim(j3.job_id, "worker") == "claimed"
    assert c.publish(j3.job_id) == "published"
    assert c.download(j3.job_id) == "download_allowed"
    c.revoke("tenant:a", "doc:1")
    after_success = c.download(j3.job_id)
    assert after_success == "authz_denied"

    return {
        "revoke_before_claim": before_claim,
        "revoke_before_publish": before_publish,
        "revoke_after_success_download": after_success,
        "artifact_publication_after_revoke": False,
    }


def scenario_expiry() -> dict:
    m = ExportModel()
    _, job = m.create(
        tenant_id="tenant:a", document_id="doc:1", revision_id="rev:1",
        profile="pdf", layout_environment_id="env:1", client_request_id="req:expiry"
    )
    assert m.claim(job.job_id, "worker") == "claimed"
    assert m.publish(job.job_id, expires_in=5) == "published"
    before = m.download(job.job_id)
    m.tick = 5
    after = m.download(job.job_id)
    assert before == "download_allowed"
    assert after == "expired"
    return {"before_expiry": before, "at_expiry": after}


def main() -> None:
    OUT.parent.mkdir(parents=True, exist_ok=True)
    scenarios = {
        "idempotency": scenario_idempotency(),
        "exact_revision_pin": scenario_revision_pin_and_head_move(),
        "crash_after_artifact_before_job_result": scenario_crash_and_adopt(),
        "cancellation_races": scenario_cancel_races(),
        "authorization_revocation_races": scenario_revoke_races(),
        "artifact_expiry": scenario_expiry(),
    }
    receipt = {
        "receipt_kind": "chaptera.cloud-export-job-reference-model.v1",
        "deployed_service": False,
        "canonical_private_core": False,
        "scenarios": scenarios,
        "bounded_findings": {
            "durable_user_job_record_required": True,
            "job_pins_exact_revision": True,
            "same_request_id_same_hash_is_idempotent": True,
            "same_request_id_different_hash_fails_closed": True,
            "authorization_rechecked_at_claim": True,
            "authorization_rechecked_before_artifact_publication": True,
            "authorization_rechecked_at_download": True,
            "running_cancel_is_cooperative_barrier_before_publish": True,
            "late_cancel_after_success_does_not_revoke_history": True,
            "deterministic_physical_artifact_may_be_reused_after_crash": True,
            "physical_content_identity_is_not_download_authorization": True,
        },
        "architecture_direction": {
            "states": [
                "queued",
                "running",
                "succeeded",
                "failed",
                "cancelled",
            ],
            "cancel_semantics": (
                "queued may cancel immediately; running sets cancel_requested; "
                "publication is a final barrier. If success wins first, cancel is too late."
            ),
            "authz_semantics": (
                "check at create, claim, publication and download; a completed physical "
                "artifact may remain internal after revoke but must not be published/downloaded."
            ),
            "crash_semantics": (
                "lease/retry may adopt the same deterministic physical artifact; durable "
                "job completion/binding remains idempotent and tenant-authorized."
            ),
            "progress": (
                "coarse stage is user-facing advisory state, not canonical percent-complete truth."
            ),
        },
        "guardrail": (
            "Reference-model evidence only. It does not prove exporter determinism, production "
            "queue semantics, deployed AuthZ propagation latency, storage retention or worker cancellation."
        ),
    }
    OUT.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
