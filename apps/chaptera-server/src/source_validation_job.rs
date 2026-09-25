use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    job_queue::{JobKind, JobRecord},
    job_worker::{CancellationFlag, JobExecutor, JobFailure, JobFuture, JobSuccess},
    source_ingress::{IngressError, UploadRecord, UploadState},
    source_ingress_async::{AsyncSourceSecurityScanner, AsyncSourceValidationRuntime},
    source_ingress_sqlite::SqliteSourceIngressRepository,
    upload_admission::{
        SqliteUploadAdmissionAuthority, UploadAdmissionError, UploadAdmissionRequest,
    },
};

pub const SOURCE_VALIDATION_JOB_SCHEMA_VERSION: i64 = 1;
pub const SOURCE_VALIDATION_JOB_PROTOCOL_V1: &str = "chaptera.source-validation-job.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceValidationJobPayloadV1 {
    pub protocol: String,
    pub tenant_id: String,
    pub upload_id: String,
    pub principal_id: String,
    /// Upload generation immediately after durable completion established
    /// STORED_UNVERIFIED. A retry may observe +1 while VALIDATING or a later
    /// terminal generation, but may never silently bind to a different upload.
    pub completed_upload_generation: u64,
    pub admission_reservation_id: String,
    pub admission_expected_bytes: i64,
    pub admission_request_hash: String,
}

impl SourceValidationJobPayloadV1 {
    pub fn admission_request(&self) -> UploadAdmissionRequest {
        UploadAdmissionRequest {
            reservation_id: self.admission_reservation_id.clone(),
            tenant_id: self.tenant_id.clone(),
            principal_id: self.principal_id.clone(),
            expected_bytes: self.admission_expected_bytes,
            request_hash: self.admission_request_hash.clone(),
        }
    }

    fn validate(&self) -> Result<(), JobFailure> {
        if self.protocol != SOURCE_VALIDATION_JOB_PROTOCOL_V1
            || !bounded_ident(&self.tenant_id)
            || !bounded_ident(&self.upload_id)
            || !bounded_ident(&self.principal_id)
            || !bounded_ident(&self.admission_reservation_id)
            || self.admission_expected_bytes <= 0
            || !lower_hex_64(&self.admission_request_hash)
        {
            return Err(permanent("source_validation_payload_invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct PortError {
    retryable: bool,
}

impl PortError {
    const fn retryable() -> Self {
        Self { retryable: true }
    }

    const fn permanent() -> Self {
        Self { retryable: false }
    }
}

#[async_trait]
trait UploadLookup: Send + Sync {
    async fn get_upload(&self, upload_id: &str) -> Result<Option<UploadRecord>, PortError>;
}

#[async_trait]
impl UploadLookup for SqliteSourceIngressRepository {
    async fn get_upload(&self, upload_id: &str) -> Result<Option<UploadRecord>, PortError> {
        self.get(upload_id)
            .await
            .map_err(|error| map_ingress_port_error(&error))
    }
}

#[async_trait]
trait UploadAdmissionLifecycle: Send + Sync {
    async fn reacquire_exact(
        &self,
        request: UploadAdmissionRequest,
        now_ms: i64,
    ) -> Result<(), PortError>;

    async fn release_exact(
        &self,
        request: UploadAdmissionRequest,
        now_ms: i64,
    ) -> Result<(), PortError>;
}

#[async_trait]
impl UploadAdmissionLifecycle for SqliteUploadAdmissionAuthority {
    async fn reacquire_exact(
        &self,
        request: UploadAdmissionRequest,
        now_ms: i64,
    ) -> Result<(), PortError> {
        SqliteUploadAdmissionAuthority::reacquire_exact(self, request, now_ms)
            .await
            .map(|_| ())
            .map_err(|error| map_admission_port_error(&error))
    }

    async fn release_exact(
        &self,
        request: UploadAdmissionRequest,
        now_ms: i64,
    ) -> Result<(), PortError> {
        SqliteUploadAdmissionAuthority::release_exact(self, request, now_ms)
            .await
            .map(|_| ())
            .or_else(|error| {
                if error.code == "upload_admission_not_found" {
                    // A retained terminal upload may outlive an already-cleaned
                    // admission row. Capacity is already absent in that case.
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|error| map_admission_port_error(&error))
    }
}

#[async_trait]
trait ValidationOperation: Send + Sync {
    async fn validate(
        &self,
        tenant_id: &str,
        upload_id: &str,
        now_ms: u64,
    ) -> Result<UploadRecord, PortError>;
}

struct ProductionValidationOperation {
    runtime: AsyncSourceValidationRuntime,
    scanner: Arc<dyn AsyncSourceSecurityScanner>,
}

#[async_trait]
impl ValidationOperation for ProductionValidationOperation {
    async fn validate(
        &self,
        tenant_id: &str,
        upload_id: &str,
        now_ms: u64,
    ) -> Result<UploadRecord, PortError> {
        self.runtime
            .validate_and_promote(tenant_id, upload_id, now_ms, self.scanner.as_ref())
            .await
            .map_err(|error| map_ingress_port_error(&error))
    }
}

pub struct SourceValidationJobExecutor {
    uploads: Arc<dyn UploadLookup>,
    admission: Arc<dyn UploadAdmissionLifecycle>,
    validation: Arc<dyn ValidationOperation>,
}

impl SourceValidationJobExecutor {
    pub fn new(
        repo: SqliteSourceIngressRepository,
        admission: SqliteUploadAdmissionAuthority,
        runtime: AsyncSourceValidationRuntime,
        scanner: Arc<dyn AsyncSourceSecurityScanner>,
    ) -> Self {
        Self {
            uploads: Arc::new(repo),
            admission: Arc::new(admission),
            validation: Arc::new(ProductionValidationOperation { runtime, scanner }),
        }
    }

    #[cfg(test)]
    fn with_ports(
        uploads: Arc<dyn UploadLookup>,
        admission: Arc<dyn UploadAdmissionLifecycle>,
        validation: Arc<dyn ValidationOperation>,
    ) -> Self {
        Self {
            uploads,
            admission,
            validation,
        }
    }

    async fn execute_inner(
        &self,
        job: &JobRecord,
        cancellation: CancellationFlag,
    ) -> Result<JobSuccess, JobFailure> {
        if job.job_kind != JobKind::Parse {
            return Err(permanent("source_validation_wrong_job_kind"));
        }
        if job.payload_schema_version != SOURCE_VALIDATION_JOB_SCHEMA_VERSION {
            return Err(permanent("source_validation_payload_version"));
        }
        let payload: SourceValidationJobPayloadV1 = serde_json::from_slice(&job.payload)
            .map_err(|_| permanent("source_validation_payload_invalid"))?;
        payload.validate()?;
        if job.tenant_id != payload.tenant_id {
            return Err(permanent("source_validation_tenant_mismatch"));
        }
        if cancellation.is_cancelled() {
            return Err(permanent("source_validation_cancelled"));
        }

        let upload = self
            .uploads
            .get_upload(&payload.upload_id)
            .await
            .map_err(map_port_failure)?
            .ok_or_else(|| permanent("source_validation_upload_missing"))?;
        validate_upload_fence(&upload, &payload)?;

        let admission_request = payload.admission_request();
        if validation_terminal(upload.state) {
            self.admission
                .release_exact(admission_request, now_ms_i64()?)
                .await
                .map_err(|_| retryable("source_validation_release_retry"))?;
            return Ok(JobSuccess {
                effect_key: effect_key(&upload),
            });
        }

        self.admission
            .reacquire_exact(admission_request.clone(), now_ms_i64()?)
            .await
            .map_err(map_port_failure)?;

        if cancellation.is_cancelled() {
            // Do not release a non-terminal upload reservation on cancellation:
            // the upload itself remains resumable. Its lease can expire and a
            // future Parse attempt must pass exact reacquire again.
            return Err(permanent("source_validation_cancelled"));
        }

        let result = self
            .validation
            .validate(&payload.tenant_id, &payload.upload_id, now_ms_u64()?)
            .await
            .map_err(map_port_failure)?;

        if !validation_terminal(result.state) {
            return Err(retryable("source_validation_nonterminal_result"));
        }

        self.admission
            .release_exact(admission_request, now_ms_i64()?)
            .await
            .map_err(|_| retryable("source_validation_release_retry"))?;

        Ok(JobSuccess {
            effect_key: effect_key(&result),
        })
    }
}

impl JobExecutor for SourceValidationJobExecutor {
    fn execute<'a>(&'a self, job: &'a JobRecord, cancellation: CancellationFlag) -> JobFuture<'a> {
        Box::pin(async move { self.execute_inner(job, cancellation).await })
    }
}

fn validate_upload_fence(
    upload: &UploadRecord,
    payload: &SourceValidationJobPayloadV1,
) -> Result<(), JobFailure> {
    let expected_bytes = u64::try_from(payload.admission_expected_bytes)
        .map_err(|_| permanent("source_validation_payload_invalid"))?;
    if upload.tenant_id != payload.tenant_id
        || upload.principal_id != payload.principal_id
        || upload.expected_byte_len != expected_bytes
    {
        return Err(permanent("source_validation_upload_identity_mismatch"));
    }

    match upload.state {
        UploadState::Issued => Err(permanent("source_validation_upload_not_complete")),
        UploadState::StoredUnverified
            if upload.upload_generation != payload.completed_upload_generation =>
        {
            Err(permanent("source_validation_upload_fence"))
        }
        UploadState::Validating
            if upload.upload_generation
                != payload
                    .completed_upload_generation
                    .checked_add(1)
                    .ok_or_else(|| permanent("source_validation_upload_fence"))? =>
        {
            Err(permanent("source_validation_upload_fence"))
        }
        UploadState::ValidatedDurable
        | UploadState::Consumed
        | UploadState::Rejected
        | UploadState::Expired
            if upload.upload_generation < payload.completed_upload_generation =>
        {
            Err(permanent("source_validation_upload_fence"))
        }
        _ => Ok(()),
    }
}

fn validation_terminal(state: UploadState) -> bool {
    matches!(
        state,
        UploadState::ValidatedDurable
            | UploadState::Consumed
            | UploadState::Rejected
            | UploadState::Expired
    )
}

fn effect_key(upload: &UploadRecord) -> String {
    format!(
        "source-validation:{}:{}:{}",
        upload.upload_id,
        state_name(upload.state),
        upload.upload_generation
    )
}

fn state_name(state: UploadState) -> &'static str {
    match state {
        UploadState::Issued => "issued",
        UploadState::StoredUnverified => "stored_unverified",
        UploadState::Validating => "validating",
        UploadState::ValidatedDurable => "validated_durable",
        UploadState::Consumed => "consumed",
        UploadState::Rejected => "rejected",
        UploadState::Expired => "expired",
    }
}

fn map_port_failure(error: PortError) -> JobFailure {
    if error.retryable {
        retryable("source_validation_retry")
    } else {
        permanent("source_validation_failed")
    }
}

fn map_admission_port_error(error: &UploadAdmissionError) -> PortError {
    if matches!(
        error.code,
        "sqlite_upload_admission_error"
            | "upload_principal_capacity"
            | "upload_tenant_capacity"
    ) {
        PortError::retryable()
    } else {
        PortError::permanent()
    }
}

fn map_ingress_port_error(error: &IngressError) -> PortError {
    if matches!(
        error.code,
        "sqlite_source_ingress_error"
            | "sqlite_blob_metadata_error"
            | "blob_provider_unavailable"
            | "blob_provider_unknown"
            | "source_malware_scanner_unavailable"
            | "source_malware_scanner_timeout"
            | "source_malware_scanner_failed"
            | "source_structural_scan_timeout"
            | "source_structural_scan_failed"
            | "source_scanner_read_failed"
            | "source_scanner_temp_failed"
    ) {
        PortError::retryable()
    } else {
        PortError::permanent()
    }
}

fn retryable(code: &'static str) -> JobFailure {
    JobFailure {
        retryable: true,
        terminal_code: code,
    }
}

fn permanent(code: &'static str) -> JobFailure {
    JobFailure {
        retryable: false,
        terminal_code: code,
    }
}

fn bounded_ident(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
        })
}

fn lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn now_ms_i64() -> Result<i64, JobFailure> {
    i64::try_from(now_ms_u64()?).map_err(|_| permanent("source_validation_clock"))
}

fn now_ms_u64() -> Result<u64, JobFailure> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| permanent("source_validation_clock"))?
        .as_millis();
    u64::try_from(millis).map_err(|_| permanent("source_validation_clock"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn upload(state: UploadState, generation: u64) -> UploadRecord {
        UploadRecord {
            upload_id: "upload-a".into(),
            tenant_id: "tenant-a".into(),
            principal_id: "principal-a".into(),
            purpose: crate::source_ingress::UploadPurpose::PubSource,
            expected_byte_len: 400,
            declared_content_type: None,
            physical_upload_ref: "redacted".into(),
            state,
            upload_generation: generation,
            object_version: Some("generation-a".into()),
            object_etag: Some("etag-a".into()),
            observed_byte_len: Some(400),
            canonical_sha256: None,
            durable_binding_id: None,
            created_at_ms: 1,
            expires_at_ms: u64::MAX,
            completed_at_ms: Some(2),
            terminal_code: None,
            idempotency_key: "issue-a".into(),
            request_hash: "a".repeat(64),
        }
    }

    fn payload() -> SourceValidationJobPayloadV1 {
        SourceValidationJobPayloadV1 {
            protocol: SOURCE_VALIDATION_JOB_PROTOCOL_V1.into(),
            tenant_id: "tenant-a".into(),
            upload_id: "upload-a".into(),
            principal_id: "principal-a".into(),
            completed_upload_generation: 1,
            admission_reservation_id: "upload-admission:a".into(),
            admission_expected_bytes: 400,
            admission_request_hash: "b".repeat(64),
        }
    }

    fn job(payload: &SourceValidationJobPayloadV1) -> JobRecord {
        JobRecord {
            job_id: "parse-a".into(),
            tenant_id: payload.tenant_id.clone(),
            job_kind: JobKind::Parse,
            payload_schema_version: SOURCE_VALIDATION_JOB_SCHEMA_VERSION,
            payload: serde_json::to_vec(payload).unwrap(),
            request_hash: "c".repeat(64),
            status: crate::job_queue::JobStatus::Running,
            available_at_ms: 0,
            attempt: 1,
            max_attempts: 3,
            lease_owner: Some("worker-a".into()),
            lease_generation: 1,
            lease_expires_at_ms: Some(i64::MAX),
            cancel_requested_at_ms: None,
            idempotency_key: "parse-idem-a".into(),
            created_at_ms: 0,
            started_at_ms: Some(0),
            finished_at_ms: None,
            terminal_code: None,
        }
    }

    struct FakeUploads(Mutex<UploadRecord>);

    #[async_trait]
    impl UploadLookup for FakeUploads {
        async fn get_upload(&self, _upload_id: &str) -> Result<Option<UploadRecord>, PortError> {
            Ok(Some(self.0.lock().unwrap().clone()))
        }
    }

    #[derive(Default)]
    struct FakeAdmission {
        reacquire: Mutex<u64>,
        release: Mutex<u64>,
    }

    #[async_trait]
    impl UploadAdmissionLifecycle for FakeAdmission {
        async fn reacquire_exact(
            &self,
            _request: UploadAdmissionRequest,
            _now_ms: i64,
        ) -> Result<(), PortError> {
            *self.reacquire.lock().unwrap() += 1;
            Ok(())
        }

        async fn release_exact(
            &self,
            _request: UploadAdmissionRequest,
            _now_ms: i64,
        ) -> Result<(), PortError> {
            *self.release.lock().unwrap() += 1;
            Ok(())
        }
    }

    struct FakeValidation {
        result: Mutex<UploadRecord>,
    }

    #[async_trait]
    impl ValidationOperation for FakeValidation {
        async fn validate(
            &self,
            _tenant_id: &str,
            _upload_id: &str,
            _now_ms: u64,
        ) -> Result<UploadRecord, PortError> {
            Ok(self.result.lock().unwrap().clone())
        }
    }

    #[tokio::test]
    async fn stored_upload_reacquires_validates_and_releases() {
        let uploads = Arc::new(FakeUploads(Mutex::new(upload(
            UploadState::StoredUnverified,
            1,
        ))));
        let admission = Arc::new(FakeAdmission::default());
        let validation = Arc::new(FakeValidation {
            result: Mutex::new(upload(UploadState::ValidatedDurable, 3)),
        });
        let executor = SourceValidationJobExecutor::with_ports(
            uploads,
            admission.clone(),
            validation,
        );

        let result = executor
            .execute_inner(&job(&payload()), CancellationFlag::default())
            .await
            .unwrap();
        assert!(result.effect_key.contains("validated_durable"));
        assert_eq!(*admission.reacquire.lock().unwrap(), 1);
        assert_eq!(*admission.release.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn terminal_replay_skips_validation_and_only_releases() {
        let uploads = Arc::new(FakeUploads(Mutex::new(upload(
            UploadState::ValidatedDurable,
            3,
        ))));
        let admission = Arc::new(FakeAdmission::default());
        let validation = Arc::new(FakeValidation {
            result: Mutex::new(upload(UploadState::ValidatedDurable, 3)),
        });
        let executor = SourceValidationJobExecutor::with_ports(
            uploads,
            admission.clone(),
            validation,
        );

        let result = executor
            .execute_inner(&job(&payload()), CancellationFlag::default())
            .await
            .unwrap();
        assert!(result.effect_key.contains("validated_durable"));
        assert_eq!(*admission.reacquire.lock().unwrap(), 0);
        assert_eq!(*admission.release.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn validating_retry_accepts_one_generation_advance() {
        let record = upload(UploadState::Validating, 2);
        validate_upload_fence(&record, &payload()).unwrap();
    }

    #[tokio::test]
    async fn stale_nonterminal_generation_fails_closed() {
        let record = upload(UploadState::StoredUnverified, 2);
        assert_eq!(
            validate_upload_fence(&record, &payload())
                .unwrap_err()
                .terminal_code,
            "source_validation_upload_fence"
        );
    }
}
