use std::{
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    job_queue::{EnqueueOutcome, EnqueueRequest, JobKind, JobRecord, SqliteJobQueue},
    job_worker::{CancellationFlag, JobExecutor, JobFailure, JobFuture, JobSuccess},
    source_ingress::{IngressError, UploadRecord, UploadState},
    source_ingress_async::{AsyncSourceSecurityScanner, AsyncSourceValidationRuntime},
    source_ingress_sqlite::SqliteSourceIngressRepository,
    upload_admission::{
        SqliteUploadAdmissionAuthority, UploadAdmissionError, UploadAdmissionRequest,
    },
};

pub const SOURCE_VALIDATION_JOB_PAYLOAD_SCHEMA_V1: &str = "chaptera.source-validation-job.v1";
pub const SOURCE_VALIDATION_JOB_SCHEMA_VERSION: i64 = 1;
const SOURCE_VALIDATION_MAX_ATTEMPTS: i64 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceValidationJobPayloadV1 {
    pub schema_version: String,
    pub tenant_id: String,
    pub upload_id: String,
    pub principal_id: String,
    pub expected_upload_generation: u64,
    pub admission_reservation_id: String,
    pub admission_expected_bytes: i64,
    pub admission_request_hash: String,
}

impl SourceValidationJobPayloadV1 {
    pub fn validate(&self) -> Result<(), SourceValidationJobError> {
        if self.schema_version != SOURCE_VALIDATION_JOB_PAYLOAD_SCHEMA_V1 {
            return Err(SourceValidationJobError::new(
                "source_validation_payload_schema_mismatch",
                "source validation payload schema is unsupported",
            ));
        }
        for (label, value) in [
            ("tenant_id", self.tenant_id.as_str()),
            ("upload_id", self.upload_id.as_str()),
            ("principal_id", self.principal_id.as_str()),
            (
                "admission_reservation_id",
                self.admission_reservation_id.as_str(),
            ),
        ] {
            require_ident(value, label)?;
        }
        if self.admission_expected_bytes <= 0 {
            return Err(SourceValidationJobError::new(
                "source_validation_payload_invalid",
                "admission_expected_bytes must be positive",
            ));
        }
        require_hash(&self.admission_request_hash, "admission_request_hash")
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, SourceValidationJobError> {
        if bytes.is_empty() || bytes.len() > 16 * 1024 {
            return Err(SourceValidationJobError::new(
                "source_validation_payload_invalid",
                "source validation payload is empty or exceeds 16 KiB",
            ));
        }
        let payload: Self = serde_json::from_slice(bytes).map_err(|_| {
            SourceValidationJobError::new(
                "source_validation_payload_invalid",
                "source validation payload is malformed",
            )
        })?;
        payload.validate()?;
        Ok(payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceValidationJobError {
    pub code: &'static str,
    pub message: String,
}

impl SourceValidationJobError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for SourceValidationJobError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SourceValidationJobError {}

#[derive(Clone)]
pub struct SourceValidationJobQueue {
    queue: SqliteJobQueue,
}

impl SourceValidationJobQueue {
    pub fn new(queue: SqliteJobQueue) -> Self {
        Self { queue }
    }

    pub async fn enqueue(
        &self,
        payload: SourceValidationJobPayloadV1,
        now_ms: i64,
    ) -> Result<JobRecord, SourceValidationJobError> {
        payload.validate()?;
        if now_ms < 0 {
            return Err(SourceValidationJobError::new(
                "source_validation_time_invalid",
                "enqueue time must be non-negative",
            ));
        }
        let payload_bytes = serde_json::to_vec(&payload).map_err(|_| {
            SourceValidationJobError::new(
                "source_validation_payload_encode_failed",
                "source validation payload could not be encoded",
            )
        })?;
        SourceValidationJobPayloadV1::decode(&payload_bytes)?;

        let stable = stable_job_identity(
            &payload.tenant_id,
            &payload.upload_id,
            payload.expected_upload_generation,
        );
        let outcome = self
            .queue
            .enqueue(EnqueueRequest {
                job_id: format!("job:parse:{stable}"),
                tenant_id: payload.tenant_id,
                job_kind: JobKind::Parse,
                payload_schema_version: SOURCE_VALIDATION_JOB_SCHEMA_VERSION,
                payload: payload_bytes,
                idempotency_key: format!("source-validate:{stable}"),
                max_attempts: SOURCE_VALIDATION_MAX_ATTEMPTS,
                now_ms,
            })
            .await
            .map_err(|error| SourceValidationJobError::new(error.code, error.message))?;

        Ok(match outcome {
            EnqueueOutcome::Enqueued(job) | EnqueueOutcome::Existing(job) => job,
        })
    }
}

#[async_trait]
pub trait SourceValidationPort: Send + Sync {
    async fn validate_and_promote(
        &self,
        tenant_id: &str,
        upload_id: &str,
        now_ms: u64,
        scanner: &dyn AsyncSourceSecurityScanner,
    ) -> Result<UploadRecord, IngressError>;

    async fn reject_terminal(
        &self,
        tenant_id: &str,
        upload_id: &str,
        code: &'static str,
        now_ms: u64,
    ) -> Result<UploadRecord, IngressError>;
}

#[async_trait]
impl SourceValidationPort for AsyncSourceValidationRuntime {
    async fn validate_and_promote(
        &self,
        tenant_id: &str,
        upload_id: &str,
        now_ms: u64,
        scanner: &dyn AsyncSourceSecurityScanner,
    ) -> Result<UploadRecord, IngressError> {
        AsyncSourceValidationRuntime::validate_and_promote(
            self, tenant_id, upload_id, now_ms, scanner,
        )
        .await
    }

    async fn reject_terminal(
        &self,
        tenant_id: &str,
        upload_id: &str,
        code: &'static str,
        now_ms: u64,
    ) -> Result<UploadRecord, IngressError> {
        AsyncSourceValidationRuntime::reject_terminal(self, tenant_id, upload_id, code, now_ms)
            .await
    }
}

pub struct SourceValidationJobExecutor {
    repo: SqliteSourceIngressRepository,
    validation: Arc<dyn SourceValidationPort>,
    scanner: Arc<dyn AsyncSourceSecurityScanner>,
    upload_admission: SqliteUploadAdmissionAuthority,
}

impl SourceValidationJobExecutor {
    pub fn new(
        repo: SqliteSourceIngressRepository,
        validation: Arc<dyn SourceValidationPort>,
        scanner: Arc<dyn AsyncSourceSecurityScanner>,
        upload_admission: SqliteUploadAdmissionAuthority,
    ) -> Self {
        Self {
            repo,
            validation,
            scanner,
            upload_admission,
        }
    }

    async fn execute_validation(&self, job: &JobRecord) -> Result<JobSuccess, JobFailure> {
        if job.job_kind != JobKind::Parse
            || job.payload_schema_version != SOURCE_VALIDATION_JOB_SCHEMA_VERSION
        {
            return Err(nonretryable("source_validation_job_profile_mismatch"));
        }

        let payload = SourceValidationJobPayloadV1::decode(&job.payload)
            .map_err(|_| nonretryable("source_validation_payload_invalid"))?;
        if payload.tenant_id != job.tenant_id {
            return Err(nonretryable("source_validation_tenant_mismatch"));
        }

        let now_ms = unix_now_ms().map_err(|_| nonretryable("source_validation_clock_invalid"))?;
        let upload = self
            .repo
            .get(&payload.upload_id)
            .await
            .map_err(|_| retryable("source_validation_repository_unavailable"))?
            .ok_or_else(|| nonretryable("source_validation_upload_missing"))?;
        validate_upload_fence(&payload, &upload)?;

        if admission_releasable(upload.state) {
            self.release_terminal_admission(&payload, now_ms).await?;
            return Ok(success_effect(&payload, &upload));
        }

        self.reacquire_active_admission(&payload, now_ms).await?;

        let validation_now =
            u64::try_from(now_ms).map_err(|_| nonretryable("source_validation_clock_invalid"))?;
        match self
            .validation
            .validate_and_promote(
                &payload.tenant_id,
                &payload.upload_id,
                validation_now,
                self.scanner.as_ref(),
            )
            .await
        {
            Ok(result) => {
                validate_upload_identity(&payload, &result)?;
                if !admission_releasable(result.state) {
                    return Err(nonretryable("source_validation_incomplete_result"));
                }
                self.release_terminal_admission(&payload, unix_now_ms().unwrap_or(now_ms))
                    .await?;
                Ok(success_effect(&payload, &result))
            }
            Err(error)
                if validation_error_retryable(error.code) && job.attempt < job.max_attempts =>
            {
                self.reacquire_active_admission(&payload, unix_now_ms().unwrap_or(now_ms))
                    .await?;
                Err(retryable("source_validation_transient_failure"))
            }
            Err(error) => {
                let terminal_code = if validation_error_retryable(error.code) {
                    "source_validation_exhausted"
                } else {
                    "source_validation_failed"
                };
                let terminal_now = unix_now_ms().unwrap_or(now_ms);
                let terminal = self
                    .validation
                    .reject_terminal(
                        &payload.tenant_id,
                        &payload.upload_id,
                        terminal_code,
                        u64::try_from(terminal_now)
                            .map_err(|_| nonretryable("source_validation_clock_invalid"))?,
                    )
                    .await
                    .map_err(|_| nonretryable("source_validation_terminalize_failed"))?;
                validate_upload_identity(&payload, &terminal)?;
                self.release_terminal_admission(&payload, terminal_now)
                    .await?;
                Err(nonretryable(terminal_code))
            }
        }
    }

    async fn reacquire_active_admission(
        &self,
        payload: &SourceValidationJobPayloadV1,
        now_ms: i64,
    ) -> Result<(), JobFailure> {
        self.upload_admission
            .reacquire_exact(admission_request(payload), now_ms)
            .await
            .map(|_| ())
            .map_err(map_reacquire_error)
    }

    async fn release_terminal_admission(
        &self,
        payload: &SourceValidationJobPayloadV1,
        now_ms: i64,
    ) -> Result<(), JobFailure> {
        match self
            .upload_admission
            .release_exact(admission_request(payload), now_ms)
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if error.code == "upload_admission_not_found" => {
                // Terminal upload state is durable authority. A reservation may
                // already have aged out after the bounded retention window.
                Ok(())
            }
            Err(error) if admission_error_retryable(error.code) => {
                Err(retryable("source_validation_admission_unavailable"))
            }
            Err(_) => Err(nonretryable(
                "source_validation_admission_identity_mismatch",
            )),
        }
    }
}

impl JobExecutor for SourceValidationJobExecutor {
    fn execute<'a>(&'a self, job: &'a JobRecord, _cancellation: CancellationFlag) -> JobFuture<'a> {
        Box::pin(async move { self.execute_validation(job).await })
    }
}

fn validate_upload_fence(
    payload: &SourceValidationJobPayloadV1,
    upload: &UploadRecord,
) -> Result<(), JobFailure> {
    validate_upload_identity(payload, upload)?;
    match upload.state {
        UploadState::Issued => Err(nonretryable("source_validation_upload_not_complete")),
        UploadState::StoredUnverified
            if upload.upload_generation != payload.expected_upload_generation =>
        {
            Err(nonretryable("source_validation_generation_mismatch"))
        }
        UploadState::Validating
            if upload.upload_generation
                != payload
                    .expected_upload_generation
                    .checked_add(1)
                    .ok_or_else(|| nonretryable("source_validation_generation_mismatch"))? =>
        {
            Err(nonretryable("source_validation_generation_mismatch"))
        }
        UploadState::ValidatedDurable
        | UploadState::Consumed
        | UploadState::Rejected
        | UploadState::Expired
            if upload.upload_generation < payload.expected_upload_generation =>
        {
            Err(nonretryable("source_validation_generation_mismatch"))
        }
        _ => Ok(()),
    }
}

fn validate_upload_identity(
    payload: &SourceValidationJobPayloadV1,
    upload: &UploadRecord,
) -> Result<(), JobFailure> {
    let expected_bytes = u64::try_from(payload.admission_expected_bytes)
        .map_err(|_| nonretryable("source_validation_payload_invalid"))?;
    if upload.upload_id != payload.upload_id
        || upload.tenant_id != payload.tenant_id
        || upload.principal_id != payload.principal_id
        || upload.expected_byte_len != expected_bytes
    {
        return Err(nonretryable("source_validation_upload_identity_mismatch"));
    }
    Ok(())
}

fn admission_request(payload: &SourceValidationJobPayloadV1) -> UploadAdmissionRequest {
    UploadAdmissionRequest {
        reservation_id: payload.admission_reservation_id.clone(),
        tenant_id: payload.tenant_id.clone(),
        principal_id: payload.principal_id.clone(),
        expected_bytes: payload.admission_expected_bytes,
        request_hash: payload.admission_request_hash.clone(),
    }
}

fn admission_error_retryable(code: &str) -> bool {
    matches!(
        code,
        "sqlite_upload_admission_error" | "upload_principal_capacity" | "upload_tenant_capacity"
    )
}

fn map_reacquire_error(error: UploadAdmissionError) -> JobFailure {
    if admission_error_retryable(error.code) {
        retryable("source_validation_admission_retry")
    } else {
        nonretryable("source_validation_admission_lost")
    }
}

fn admission_releasable(state: UploadState) -> bool {
    matches!(
        state,
        UploadState::ValidatedDurable
            | UploadState::Consumed
            | UploadState::Rejected
            | UploadState::Expired
    )
}

fn success_effect(payload: &SourceValidationJobPayloadV1, upload: &UploadRecord) -> JobSuccess {
    let terminal_class = match upload.state {
        UploadState::ValidatedDurable | UploadState::Consumed => "validated_durable",
        UploadState::Rejected => "rejected",
        UploadState::Expired => "expired",
        UploadState::Issued | UploadState::StoredUnverified | UploadState::Validating => {
            "nonterminal"
        }
    };

    let mut hasher = Sha256::new();
    hasher.update(b"chaptera.source-validation-effect.v1\0");
    hasher.update(payload.tenant_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(payload.upload_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(terminal_class.as_bytes());
    if let Some(hash) = upload.canonical_sha256.as_deref() {
        hasher.update(b"\0hash\0");
        hasher.update(hash.as_bytes());
    }
    if let Some(binding) = upload.durable_binding_id.as_deref() {
        hasher.update(b"\0binding\0");
        hasher.update(binding.as_bytes());
    }
    if let Some(code) = upload.terminal_code.as_deref() {
        hasher.update(b"\0terminal\0");
        hasher.update(code.as_bytes());
    }
    JobSuccess {
        effect_key: format!("source-validation:{:x}", hasher.finalize()),
    }
}

fn stable_job_identity(tenant_id: &str, upload_id: &str, generation: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"chaptera.source-validation-job-identity.v1\0");
    hasher.update(tenant_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(upload_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(generation.to_be_bytes());
    format!("{:x}", hasher.finalize())
}

fn validation_error_retryable(code: &str) -> bool {
    matches!(
        code,
        "source_malware_scanner_unavailable"
            | "source_malware_scanner_timeout"
            | "source_malware_scanner_failed"
            | "source_scanner_temp_failed"
            | "source_scanner_read_failed"
            | "source_structural_scan_timeout"
            | "source_structural_scan_failed"
            | "provider_unknown_unreconciled"
            | "provider_unknown_outcome"
            | "provider_failure"
            | "sqlite_blob_metadata_error"
            | "sqlite_source_ingress_error"
    )
}

fn retryable(code: &'static str) -> JobFailure {
    JobFailure {
        retryable: true,
        terminal_code: code,
    }
}

fn nonretryable(code: &'static str) -> JobFailure {
    JobFailure {
        retryable: false,
        terminal_code: code,
    }
}

fn require_ident(value: &str, label: &'static str) -> Result<(), SourceValidationJobError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(SourceValidationJobError::new(
            "source_validation_payload_invalid",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_hash(value: &str, label: &'static str) -> Result<(), SourceValidationJobError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(SourceValidationJobError::new(
            "source_validation_payload_invalid",
            format!("{label} must be 64 lowercase SHA-256 hex characters"),
        ));
    }
    Ok(())
}

fn unix_now_ms() -> Result<i64, SourceValidationJobError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            SourceValidationJobError::new(
                "source_validation_clock_invalid",
                "system clock is before UNIX epoch",
            )
        })?
        .as_millis();
    i64::try_from(millis).map_err(|_| {
        SourceValidationJobError::new(
            "source_validation_clock_invalid",
            "system clock does not fit i64 milliseconds",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> SourceValidationJobPayloadV1 {
        SourceValidationJobPayloadV1 {
            schema_version: SOURCE_VALIDATION_JOB_PAYLOAD_SCHEMA_V1.into(),
            tenant_id: "tenant-a".into(),
            upload_id: "upload-a".into(),
            principal_id: "principal-a".into(),
            expected_upload_generation: 2,
            admission_reservation_id: "upload-admission:a".into(),
            admission_expected_bytes: 1024,
            admission_request_hash: "a".repeat(64),
        }
    }

    #[test]
    fn payload_roundtrip_is_bounded_and_strict() {
        let bytes = serde_json::to_vec(&payload()).unwrap();
        assert_eq!(
            SourceValidationJobPayloadV1::decode(&bytes).unwrap(),
            payload()
        );

        let mut unknown: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        unknown["raw_pub"] = serde_json::Value::String("forbidden".into());
        let encoded = serde_json::to_vec(&unknown).unwrap();
        assert_eq!(
            SourceValidationJobPayloadV1::decode(&encoded)
                .unwrap_err()
                .code,
            "source_validation_payload_invalid"
        );
    }

    #[test]
    fn stable_parse_job_identity_binds_generation() {
        let first = stable_job_identity("tenant-a", "upload-a", 2);
        let replay = stable_job_identity("tenant-a", "upload-a", 2);
        let next = stable_job_identity("tenant-a", "upload-a", 3);
        assert_eq!(first, replay);
        assert_ne!(first, next);
        assert_eq!(first.len(), 64);
    }

    #[tokio::test]
    async fn enqueue_replay_is_existing_and_changed_fingerprint_conflicts() {
        let path = std::env::temp_dir().join(format!(
            "chaptera-source-validation-job-{}-{}.sqlite",
            std::process::id(),
            unix_now_ms().unwrap()
        ));
        crate::schema_migration::SqliteMigrationRuntime::new(
            &path,
            std::time::Duration::from_secs(2),
        )
        .unwrap()
        .migrate_up()
        .await
        .unwrap();

        let queue = SqliteJobQueue::open(&path, 4, std::time::Duration::from_secs(2))
            .await
            .unwrap();
        let source_queue = SourceValidationJobQueue::new(queue.clone());
        let first = source_queue
            .enqueue(payload(), unix_now_ms().unwrap())
            .await
            .unwrap();
        let replay = source_queue
            .enqueue(payload(), unix_now_ms().unwrap())
            .await
            .unwrap();
        assert_eq!(first.job_id, replay.job_id);
        assert_eq!(first.request_hash, replay.request_hash);

        let mut changed = payload();
        changed.admission_request_hash = "b".repeat(64);
        let error = source_queue
            .enqueue(changed, unix_now_ms().unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.code, "idempotency_conflict");

        queue.close().await;
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[test]
    fn admission_retry_policy_is_explicit() {
        assert!(admission_error_retryable("upload_principal_capacity"));
        assert!(admission_error_retryable("upload_tenant_capacity"));
        assert!(admission_error_retryable("sqlite_upload_admission_error"));
        assert!(!admission_error_retryable(
            "upload_admission_already_released"
        ));
        assert!(!admission_error_retryable(
            "upload_admission_idempotency_conflict"
        ));
        assert!(!admission_error_retryable(
            "upload_admission_reacquire_conflict"
        ));
        assert!(!admission_error_retryable(
            "upload_admission_release_conflict"
        ));
    }

    #[test]
    fn durable_payload_reconstructs_exact_admission_fingerprint() {
        let payload = payload();
        let request = admission_request(&payload);
        assert_eq!(request.reservation_id, payload.admission_reservation_id);
        assert_eq!(request.tenant_id, payload.tenant_id);
        assert_eq!(request.principal_id, payload.principal_id);
        assert_eq!(request.expected_bytes, payload.admission_expected_bytes);
        assert_eq!(request.request_hash, payload.admission_request_hash);
    }

    #[test]
    fn validation_retry_policy_is_explicit() {
        assert!(validation_error_retryable("source_malware_scanner_timeout"));
        assert!(validation_error_retryable("provider_failure"));
        assert!(!validation_error_retryable("durable_binding_hash_mismatch"));
        assert!(!validation_error_retryable("tenant_mismatch"));
    }

    #[test]
    fn validated_durable_releases_upload_admission() {
        assert!(admission_releasable(UploadState::ValidatedDurable));
        assert!(admission_releasable(UploadState::Consumed));
        assert!(admission_releasable(UploadState::Rejected));
        assert!(admission_releasable(UploadState::Expired));
        assert!(!admission_releasable(UploadState::StoredUnverified));
        assert!(!admission_releasable(UploadState::Validating));
    }

    #[test]
    fn consumed_replay_preserves_validation_effect_key() {
        let base = UploadRecord {
            upload_id: "upload-a".into(),
            tenant_id: "tenant-a".into(),
            principal_id: "principal-a".into(),
            purpose: crate::source_ingress::UploadPurpose::PubSource,
            expected_byte_len: 1024,
            declared_content_type: None,
            physical_upload_ref: "redacted".into(),
            state: UploadState::ValidatedDurable,
            upload_generation: 4,
            object_version: Some("generation-a".into()),
            object_etag: Some("etag-a".into()),
            observed_byte_len: Some(1024),
            canonical_sha256: Some("c".repeat(64)),
            durable_binding_id: Some("binding-a".into()),
            created_at_ms: 1,
            expires_at_ms: 10,
            completed_at_ms: Some(2),
            terminal_code: None,
            idempotency_key: "issue-a".into(),
            request_hash: "a".repeat(64),
        };
        let mut consumed = base.clone();
        consumed.state = UploadState::Consumed;
        consumed.upload_generation = 5;

        assert_eq!(
            success_effect(&payload(), &base).effect_key,
            success_effect(&payload(), &consumed).effect_key
        );
    }
}
