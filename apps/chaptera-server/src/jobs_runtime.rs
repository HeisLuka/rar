use std::{
    fmt,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    authz_runtime::{CAP_EXPORT, SqliteAuthzAuthority},
    export_executor::{EXPORT_JOB_PAYLOAD_SCHEMA_V1, ExportJobPayloadV1},
    export_publication::{ExportPublicationRecordV1, SqliteExportPublicationStore},
    job_queue::{EnqueueOutcome, EnqueueRequest, JobKind, JobRecord, JobStatus, SqliteJobQueue},
};

const EXPORT_PAYLOAD_SCHEMA_VERSION: i64 = 1;
const EXPORT_MAX_ATTEMPTS: i64 = 3;
const MAX_CLIENT_REQUEST_ID_BYTES: usize = 192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobsRuntimeError {
    pub code: &'static str,
    pub message: String,
}

impl JobsRuntimeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for JobsRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for JobsRuntimeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateExportJobRequestV1 {
    pub tenant_id: String,
    pub document_id: String,
    pub principal_id: String,
    pub exact_revision_id: String,
    pub canonical_authoring_revision_id: String,
    pub target_profile: String,
    pub layout_environment_id: String,
    pub client_request_id: String,
    pub operation_id: String,
    pub now_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportJobSnapshotV1 {
    pub job_id: String,
    pub tenant_id: String,
    pub document_id: String,
    pub exact_revision_id: String,
    pub canonical_authoring_revision_id: String,
    pub target_profile: String,
    pub layout_environment_id: String,
    pub status: String,
    pub attempt: i64,
    pub max_attempts: i64,
    pub cancel_requested: bool,
    pub terminal_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorizedExportDownloadV1 {
    pub job_id: String,
    pub document_id: String,
    pub exact_revision_id: String,
    pub canonical_authoring_revision_id: String,
    pub target_profile: String,
    pub layout_environment_id: String,
    pub artifact_binding_id: String,
    pub artifact_content_hash: String,
    pub loss_binding_id: String,
    pub loss_report_hash: String,
}

#[derive(Clone)]
pub struct JobsRuntime {
    queue: SqliteJobQueue,
    authz: SqliteAuthzAuthority,
    publications: SqliteExportPublicationStore,
}

impl JobsRuntime {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, JobsRuntimeError> {
        let path = path.as_ref();
        let authz = SqliteAuthzAuthority::open(path, max_connections, busy_timeout)
            .await
            .map_err(authz_error)?;
        Self::open_with_authz(path, max_connections, busy_timeout, authz).await
    }

    pub async fn open_with_authz(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
        authz: SqliteAuthzAuthority,
    ) -> Result<Self, JobsRuntimeError> {
        let path = path.as_ref();
        let queue = SqliteJobQueue::open(path, max_connections, busy_timeout)
            .await
            .map_err(queue_error)?;
        let publications = SqliteExportPublicationStore::open(path, max_connections, busy_timeout)
            .await
            .map_err(publication_error)?;
        Ok(Self {
            queue,
            authz,
            publications,
        })
    }

    pub async fn close(&self) {
        self.queue.close().await;
        self.authz.close().await;
        self.publications.close().await;
    }

    pub async fn create_export(
        &self,
        request: CreateExportJobRequestV1,
    ) -> Result<ExportJobSnapshotV1, JobsRuntimeError> {
        validate_request_identity(&request)?;
        let payload = ExportJobPayloadV1 {
            schema_version: EXPORT_JOB_PAYLOAD_SCHEMA_V1.to_owned(),
            tenant_id: request.tenant_id.clone(),
            document_id: request.document_id.clone(),
            requesting_principal_id: request.principal_id.clone(),
            exact_revision_id: request.exact_revision_id.clone(),
            canonical_authoring_revision_id: request.canonical_authoring_revision_id.clone(),
            target_profile: request.target_profile.clone(),
            layout_environment_id: request.layout_environment_id.clone(),
        };
        let payload_bytes = serde_json::to_vec(&payload).map_err(|error| {
            JobsRuntimeError::new("export_payload_encode_failed", bounded(&error.to_string()))
        })?;
        // Reuse the executor's canonical V1 decoder/validator instead of
        // maintaining a second payload grammar in the serve path.
        ExportJobPayloadV1::decode(&payload_bytes).map_err(export_payload_error)?;

        self.authz
            .authorize(
                &request.tenant_id,
                &request.document_id,
                &request.principal_id,
                CAP_EXPORT,
                &request.operation_id,
                request.now_ms,
            )
            .await
            .map_err(authz_error)?;

        let job_id = stable_job_id(&request.tenant_id, &request.client_request_id);
        let outcome = self
            .queue
            .enqueue(EnqueueRequest {
                job_id,
                tenant_id: request.tenant_id,
                job_kind: JobKind::Export,
                payload_schema_version: EXPORT_PAYLOAD_SCHEMA_VERSION,
                payload: payload_bytes,
                idempotency_key: request.client_request_id,
                max_attempts: EXPORT_MAX_ATTEMPTS,
                now_ms: request.now_ms,
            })
            .await
            .map_err(queue_error)?;
        let job = match outcome {
            EnqueueOutcome::Enqueued(job) | EnqueueOutcome::Existing(job) => job,
        };
        snapshot(&job)
    }

    pub async fn status(
        &self,
        tenant_id: &str,
        principal_id: &str,
        job_id: &str,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<ExportJobSnapshotV1, JobsRuntimeError> {
        let (job, payload) = self
            .authorized_job(tenant_id, principal_id, job_id, operation_id, now_ms)
            .await?;
        snapshot_with_payload(&job, &payload)
    }

    pub async fn request_cancel(
        &self,
        tenant_id: &str,
        principal_id: &str,
        job_id: &str,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<ExportJobSnapshotV1, JobsRuntimeError> {
        let (_job, _payload) = self
            .authorized_job(tenant_id, principal_id, job_id, operation_id, now_ms)
            .await?;
        let job = self
            .queue
            .request_cancel(job_id, now_ms)
            .await
            .map_err(queue_error)?;
        snapshot(&job)
    }

    pub async fn authorize_download(
        &self,
        tenant_id: &str,
        principal_id: &str,
        job_id: &str,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<AuthorizedExportDownloadV1, JobsRuntimeError> {
        let (job, payload) = self
            .authorized_job(tenant_id, principal_id, job_id, operation_id, now_ms)
            .await?;
        if job.status != JobStatus::Succeeded {
            return Err(JobsRuntimeError::new(
                "export_artifact_not_ready",
                "export job has not reached durable success",
            ));
        }
        let publication = self
            .publications
            .get_visible_by_job(tenant_id, job_id)
            .await
            .map_err(publication_error)?
            .ok_or_else(|| {
                JobsRuntimeError::new(
                    "export_artifact_not_visible",
                    "succeeded job has no publication visible through the durable effect barrier",
                )
            })?;
        validate_publication_identity(&payload, &publication)?;
        Ok(AuthorizedExportDownloadV1 {
            job_id: job.job_id,
            document_id: payload.document_id,
            exact_revision_id: payload.exact_revision_id,
            canonical_authoring_revision_id: payload.canonical_authoring_revision_id,
            target_profile: payload.target_profile,
            layout_environment_id: payload.layout_environment_id,
            artifact_binding_id: publication.input.artifact_binding_id,
            artifact_content_hash: publication.input.artifact_content_hash,
            loss_binding_id: publication.input.loss_binding_id,
            loss_report_hash: publication.input.loss_report_hash,
        })
    }

    async fn authorized_job(
        &self,
        tenant_id: &str,
        principal_id: &str,
        job_id: &str,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<(JobRecord, ExportJobPayloadV1), JobsRuntimeError> {
        if now_ms < 0 {
            return Err(JobsRuntimeError::new(
                "invalid_now",
                "timestamp must be non-negative",
            ));
        }
        let job = self
            .queue
            .get(job_id)
            .await
            .map_err(queue_error)?
            .ok_or_else(|| JobsRuntimeError::new("job_not_found", "export job does not exist"))?;
        if job.tenant_id != tenant_id || job.job_kind != JobKind::Export {
            return Err(JobsRuntimeError::new(
                "job_scope_mismatch",
                "job is outside the requested tenant/export scope",
            ));
        }
        let payload = ExportJobPayloadV1::decode(&job.payload).map_err(export_payload_error)?;
        if payload.tenant_id != tenant_id {
            return Err(JobsRuntimeError::new(
                "job_payload_scope_mismatch",
                "durable export payload tenant differs from queue scope",
            ));
        }
        self.authz
            .authorize(
                tenant_id,
                &payload.document_id,
                principal_id,
                CAP_EXPORT,
                operation_id,
                now_ms,
            )
            .await
            .map_err(authz_error)?;
        Ok((job, payload))
    }
}

fn validate_request_identity(request: &CreateExportJobRequestV1) -> Result<(), JobsRuntimeError> {
    if request.now_ms < 0 {
        return Err(JobsRuntimeError::new(
            "invalid_now",
            "timestamp must be non-negative",
        ));
    }
    if request.client_request_id.is_empty()
        || request.client_request_id.len() > MAX_CLIENT_REQUEST_ID_BYTES
        || !request.client_request_id.bytes().all(valid_ident_byte)
    {
        return Err(JobsRuntimeError::new(
            "invalid_client_request_id",
            "client_request_id must be a bounded opaque identifier",
        ));
    }
    if request.operation_id.is_empty() || !request.operation_id.bytes().all(valid_ident_byte) {
        return Err(JobsRuntimeError::new(
            "invalid_operation_id",
            "operation_id must be a bounded opaque identifier",
        ));
    }
    Ok(())
}

fn valid_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'@' | b'/' | b'-')
}

fn stable_job_id(tenant_id: &str, client_request_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"chaptera-export-job-v1\0");
    hasher.update(tenant_id.as_bytes());
    hasher.update([0]);
    hasher.update(client_request_id.as_bytes());
    let digest = hasher.finalize();
    format!("export-job:{:x}", digest)
}

fn snapshot(job: &JobRecord) -> Result<ExportJobSnapshotV1, JobsRuntimeError> {
    let payload = ExportJobPayloadV1::decode(&job.payload).map_err(export_payload_error)?;
    snapshot_with_payload(job, &payload)
}

fn snapshot_with_payload(
    job: &JobRecord,
    payload: &ExportJobPayloadV1,
) -> Result<ExportJobSnapshotV1, JobsRuntimeError> {
    if job.job_kind != JobKind::Export || job.tenant_id != payload.tenant_id {
        return Err(JobsRuntimeError::new(
            "job_payload_scope_mismatch",
            "durable queue row and export payload identities differ",
        ));
    }
    Ok(ExportJobSnapshotV1 {
        job_id: job.job_id.clone(),
        tenant_id: job.tenant_id.clone(),
        document_id: payload.document_id.clone(),
        exact_revision_id: payload.exact_revision_id.clone(),
        canonical_authoring_revision_id: payload.canonical_authoring_revision_id.clone(),
        target_profile: payload.target_profile.clone(),
        layout_environment_id: payload.layout_environment_id.clone(),
        status: status_name(job.status).to_owned(),
        attempt: job.attempt,
        max_attempts: job.max_attempts,
        cancel_requested: job.cancel_requested_at_ms.is_some(),
        terminal_code: job.terminal_code.clone(),
    })
}

fn status_name(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::Succeeded => "succeeded",
        JobStatus::Failed => "failed",
        JobStatus::Cancelled => "cancelled",
    }
}

fn validate_publication_identity(
    payload: &ExportJobPayloadV1,
    publication: &ExportPublicationRecordV1,
) -> Result<(), JobsRuntimeError> {
    let input = &publication.input;
    if input.tenant_id != payload.tenant_id
        || input.document_id != payload.document_id
        || input.exact_revision_id != payload.exact_revision_id
        || input.canonical_revision_id != payload.canonical_authoring_revision_id
        || input.target_profile != payload.target_profile
        || input.layout_environment_id != payload.layout_environment_id
    {
        return Err(JobsRuntimeError::new(
            "export_publication_identity_mismatch",
            "visible publication differs from durable export job identity",
        ));
    }
    Ok(())
}

fn queue_error(error: crate::job_queue::JobQueueError) -> JobsRuntimeError {
    JobsRuntimeError::new(error.code, error.message)
}

fn authz_error(error: crate::authz_runtime::AuthzError) -> JobsRuntimeError {
    JobsRuntimeError::new(error.code, error.message)
}

fn publication_error(error: crate::export_publication::ExportPublicationError) -> JobsRuntimeError {
    JobsRuntimeError::new(error.code, error.message)
}

fn export_payload_error(error: crate::export_executor::ExportExecutorError) -> JobsRuntimeError {
    JobsRuntimeError::new(error.code, error.message)
}

fn bounded(message: &str) -> String {
    message.chars().take(512).collect()
}

pub fn unix_now_ms() -> Result<i64, JobsRuntimeError> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        JobsRuntimeError::new("clock_before_epoch", "system clock is before UNIX epoch")
    })?;
    i64::try_from(elapsed.as_millis()).map_err(|_| {
        JobsRuntimeError::new(
            "clock_overflow",
            "system clock does not fit i64 milliseconds",
        )
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::{
        authz_runtime::DocumentRole,
        export_publication::{ExportPublicationInputV1, ExportPublicationPrepareOutcomeV1},
        job_queue::{JobKind, JobStatus},
        schema_migration::SqliteMigrationRuntime,
    };

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-jobs-runtime-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    async fn runtime(label: &str) -> (JobsRuntime, PathBuf) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let runtime = JobsRuntime::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        runtime
            .authz
            .set_role(
                "tenant:1",
                "doc:1",
                "principal:1",
                DocumentRole::Editor,
                None,
                "grant:initial",
                1,
            )
            .await
            .unwrap();
        (runtime, path)
    }

    fn create_request(client: &str) -> CreateExportJobRequestV1 {
        CreateExportJobRequestV1 {
            tenant_id: "tenant:1".into(),
            document_id: "doc:1".into(),
            principal_id: "principal:1".into(),
            exact_revision_id: format!("sha256:{}", "a".repeat(64)),
            canonical_authoring_revision_id: "b".repeat(64),
            target_profile: crate::export_executor::IDML_BOUNDED_EDITABLE_PROFILE.into(),
            layout_environment_id: format!("sha256:{}", "c".repeat(64)),
            client_request_id: client.into(),
            operation_id: format!("create:{client}"),
            now_ms: 10,
        }
    }

    #[tokio::test]
    async fn create_is_idempotent_and_changed_retry_conflicts() {
        let (runtime, path) = runtime("idempotency").await;
        let request = create_request("client:one");
        let first = runtime.create_export(request.clone()).await.unwrap();
        let retry = runtime.create_export(request.clone()).await.unwrap();
        assert_eq!(first, retry);
        assert_eq!(first.status, "queued");

        let mut changed = request;
        changed.exact_revision_id = format!("sha256:{}", "d".repeat(64));
        changed.operation_id = "create:changed".into();
        let error = runtime.create_export(changed).await.unwrap_err();
        assert_eq!(error.code, "idempotency_conflict");

        runtime.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn restart_reopens_same_durable_job_without_payload_leakage() {
        let (runtime, path) = runtime("restart").await;
        let created = runtime
            .create_export(create_request("client:restart"))
            .await
            .unwrap();
        runtime.close().await;

        let reopened = JobsRuntime::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        let snapshot = reopened
            .status(
                "tenant:1",
                "principal:1",
                &created.job_id,
                "status:restart",
                20,
            )
            .await
            .unwrap();
        assert_eq!(snapshot, created);
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(!json.contains("requesting_principal_id"));
        assert!(!json.contains("payload"));

        reopened.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn status_and_cancel_are_tenant_scoped_and_reauthorize_current_grant() {
        let (runtime, path) = runtime("cancel").await;
        let created = runtime
            .create_export(create_request("client:cancel"))
            .await
            .unwrap();

        let wrong_tenant = runtime
            .status(
                "tenant:other",
                "principal:1",
                &created.job_id,
                "status:wrong-tenant",
                20,
            )
            .await
            .unwrap_err();
        assert_eq!(wrong_tenant.code, "job_scope_mismatch");

        let cancelled = runtime
            .request_cancel("tenant:1", "principal:1", &created.job_id, "cancel:one", 21)
            .await
            .unwrap();
        assert!(cancelled.cancel_requested);
        assert_eq!(cancelled.status, "queued");

        runtime
            .authz
            .revoke("tenant:1", "doc:1", "principal:1", "revoke:one", 22)
            .await
            .unwrap();
        let denied = runtime
            .status(
                "tenant:1",
                "principal:1",
                &created.job_id,
                "status:revoked",
                23,
            )
            .await
            .unwrap_err();
        assert_eq!(denied.code, "grant_missing");

        runtime.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn download_requires_visible_publication_effect_and_current_authz() {
        let (runtime, path) = runtime("download").await;
        let created = runtime
            .create_export(create_request("client:download"))
            .await
            .unwrap();

        let lease = runtime
            .queue
            .claim_one("worker:test", 20, 10_000, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease.job.status, JobStatus::Running);

        let input = ExportPublicationInputV1 {
            tenant_id: "tenant:1".into(),
            job_id: created.job_id.clone(),
            document_id: "doc:1".into(),
            exact_revision_id: format!("sha256:{}", "a".repeat(64)),
            canonical_revision_id: "b".repeat(64),
            target_profile: crate::export_executor::IDML_BOUNDED_EDITABLE_PROFILE.into(),
            layout_environment_id: format!("sha256:{}", "c".repeat(64)),
            fence_id: format!("sha256:{}", "d".repeat(64)),
            artifact_binding_id: "binding:artifact".into(),
            artifact_content_hash: format!("sha256:{}", "e".repeat(64)),
            loss_binding_id: "binding:loss".into(),
            loss_report_hash: format!("sha256:{}", "f".repeat(64)),
        };
        let prepared = runtime.publications.prepare(input, 21).await.unwrap();
        let record = match prepared {
            ExportPublicationPrepareOutcomeV1::Prepared(record)
            | ExportPublicationPrepareOutcomeV1::AlreadyPrepared(record) => record,
        };

        let before_effect = runtime
            .authorize_download(
                "tenant:1",
                "principal:1",
                &created.job_id,
                "download:before-effect",
                22,
            )
            .await
            .unwrap_err();
        assert_eq!(before_effect.code, "export_artifact_not_ready");

        runtime
            .queue
            .publish_success(&lease, 23, &record.effect_key)
            .await
            .unwrap();

        let download = runtime
            .authorize_download(
                "tenant:1",
                "principal:1",
                &created.job_id,
                "download:visible",
                24,
            )
            .await
            .unwrap();
        assert_eq!(download.artifact_binding_id, "binding:artifact");
        assert_eq!(download.loss_binding_id, "binding:loss");

        runtime
            .authz
            .revoke("tenant:1", "doc:1", "principal:1", "revoke:download", 25)
            .await
            .unwrap();
        let revoked = runtime
            .authorize_download(
                "tenant:1",
                "principal:1",
                &created.job_id,
                "download:revoked",
                26,
            )
            .await
            .unwrap_err();
        assert_eq!(revoked.code, "grant_missing");

        runtime.close().await;
        cleanup(&path);
    }
}
