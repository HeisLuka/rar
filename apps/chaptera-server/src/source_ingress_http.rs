use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CONTENT_LENGTH, RETRY_AFTER},
    },
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use axum_extra::extract::CookieJar;
use futures_util::StreamExt;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::{
    auth_http::{AuthHttpError, AuthHttpState},
    blob_store::{BlobStoreError, BlobStoreService},
    project_persistence_sqlite::{SqliteProjectPersistence, plan_project_identity},
    source_baseline::{IsolatedSourceBaselineProducer, SourceBaselineError},
    source_ingress::{
        ConsumeUploadRequest, IngressError, IssueUploadRequest, ProjectCreateResult, UploadPurpose,
        UploadRecord, UploadState, plan_upload_candidate, upload_admission_reservation_id,
    },
    source_ingress_async::{AsyncSourceSecurityScanner, AsyncSourceValidationRuntime},
    source_ingress_sqlite::SqliteSourceIngressRepository,
    upload_admission::{
        ReserveUploadOutcome, SqliteUploadAdmissionAuthority, UploadAdmissionError,
        UploadAdmissionRequest,
    },
    workspace_context::{SqliteWorkspaceContextResolver, WorkspaceContextError},
};

const STREAM_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct SourceIngressHttpConfig {
    pub upload_ttl: Duration,
    pub direct_grant_ttl: Duration,
}

impl SourceIngressHttpConfig {
    pub fn validate(
        &self,
        admission: &SqliteUploadAdmissionAuthority,
    ) -> Result<(), SourceIngressHttpError> {
        if self.upload_ttl.is_zero() || self.direct_grant_ttl.is_zero() {
            return Err(SourceIngressHttpError::internal(
                "source_ingress_http_config_invalid",
            ));
        }
        if self.upload_ttl > admission.lease_duration() {
            return Err(SourceIngressHttpError::internal(
                "source_ingress_upload_ttl_exceeds_admission_lease",
            ));
        }
        if self.direct_grant_ttl > self.upload_ttl {
            return Err(SourceIngressHttpError::internal(
                "source_ingress_grant_ttl_exceeds_upload_ttl",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct SourceIngressHttpState {
    auth: AuthHttpState,
    workspace: SqliteWorkspaceContextResolver,
    admission: SqliteUploadAdmissionAuthority,
    repo: SqliteSourceIngressRepository,
    blob_store: BlobStoreService,
    validation: AsyncSourceValidationRuntime,
    scanner: Arc<dyn AsyncSourceSecurityScanner>,
    baseline: IsolatedSourceBaselineProducer,
    projects: SqliteProjectPersistence,
    config: SourceIngressHttpConfig,
}

#[allow(clippy::too_many_arguments)]
impl SourceIngressHttpState {
    pub fn new(
        auth: AuthHttpState,
        workspace: SqliteWorkspaceContextResolver,
        admission: SqliteUploadAdmissionAuthority,
        repo: SqliteSourceIngressRepository,
        blob_store: BlobStoreService,
        scanner: Arc<dyn AsyncSourceSecurityScanner>,
        baseline: IsolatedSourceBaselineProducer,
        projects: SqliteProjectPersistence,
        config: SourceIngressHttpConfig,
    ) -> Result<Self, SourceIngressHttpError> {
        config.validate(&admission)?;
        let validation = AsyncSourceValidationRuntime::new(repo.clone(), blob_store.clone());
        Ok(Self {
            auth,
            workspace,
            admission,
            repo,
            blob_store,
            validation,
            scanner,
            baseline,
            projects,
            config,
        })
    }
}

pub fn router(state: SourceIngressHttpState) -> Router {
    Router::new()
        .route("/v1/uploads", post(issue_upload))
        .route("/v1/uploads/{upload_id}/content", put(put_upload_content))
        .route("/v1/uploads/{upload_id}/complete", post(complete_upload))
        .route("/v1/uploads/{upload_id}", get(upload_status))
        .route("/v1/projects/from-upload", post(create_project))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssueUploadBody {
    workspace_id: String,
    purpose: UploadPurpose,
    expected_byte_len: u64,
    declared_content_type: Option<String>,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProjectBody {
    workspace_id: String,
    upload_id: String,
    expected_upload_generation: u64,
    name: String,
    client_idempotency_id: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum UploadTransportResponse {
    Direct { grant: String, expires_at_ms: u64 },
    Streamed { path: String },
}

#[derive(Debug, Serialize)]
struct IssueUploadResponse {
    upload: UploadStatusResponse,
    transport: Option<UploadTransportResponse>,
}

#[derive(Debug, Serialize)]
struct UploadStatusResponse {
    upload_id: String,
    purpose: UploadPurpose,
    state: UploadState,
    upload_generation: u64,
    expected_byte_len: u64,
    observed_byte_len: Option<u64>,
    expires_at_ms: u64,
    terminal_code: Option<String>,
}

impl From<&UploadRecord> for UploadStatusResponse {
    fn from(upload: &UploadRecord) -> Self {
        Self {
            upload_id: upload.upload_id.clone(),
            purpose: upload.purpose,
            state: upload.state,
            upload_generation: upload.upload_generation,
            expected_byte_len: upload.expected_byte_len,
            observed_byte_len: upload.observed_byte_len,
            expires_at_ms: upload.expires_at_ms,
            terminal_code: upload.terminal_code.clone(),
        }
    }
}

async fn issue_upload(
    State(state): State<SourceIngressHttpState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<IssueUploadBody>,
) -> Result<Json<IssueUploadResponse>, SourceIngressHttpError> {
    let principal = state
        .auth
        .authenticate_mutation_request(&headers, &jar)
        .await?;
    let workspace = state
        .workspace
        .resolve(&principal.principal_id, &body.workspace_id)
        .await
        .map_err(map_workspace_error)?;

    if body.purpose != UploadPurpose::PubSource {
        return Err(SourceIngressHttpError::bad_request(
            "upload_purpose_unsupported",
        ));
    }

    let now_ms = now_ms_u64()?;
    let expires_at_ms = checked_add_duration(now_ms, state.config.upload_ttl)?;
    let upload_id = new_upload_id()?;
    let request = IssueUploadRequest {
        tenant_id: workspace.tenant_id.clone(),
        principal_id: principal.principal_id.clone(),
        expected_byte_len: body.expected_byte_len,
        declared_content_type: body.declared_content_type,
        idempotency_key: body.idempotency_key,
        now_ms,
        expires_at_ms,
    };
    let max_source_bytes = u64::try_from(state.admission.max_single_upload_bytes())
        .map_err(|_| SourceIngressHttpError::internal("upload_admission_max_bytes_invalid"))?;
    let candidate =
        plan_upload_candidate(max_source_bytes, upload_id, request).map_err(map_ingress_error)?;
    let reservation_id =
        upload_admission_reservation_id(&candidate.tenant_id, &candidate.idempotency_key)
            .map_err(map_ingress_error)?;
    let expected_bytes = i64::try_from(candidate.expected_byte_len)
        .map_err(|_| SourceIngressHttpError::payload_too_large("upload_bytes_too_large"))?;
    let now_i64 = i64::try_from(now_ms)
        .map_err(|_| SourceIngressHttpError::internal("clock_out_of_range"))?;
    let reservation = state
        .admission
        .reserve(
            UploadAdmissionRequest {
                reservation_id: reservation_id.clone(),
                tenant_id: candidate.tenant_id.clone(),
                principal_id: candidate.principal_id.clone(),
                expected_bytes,
                request_hash: candidate.request_hash.clone(),
            },
            now_i64,
        )
        .await
        .map_err(map_admission_error)?;
    let reservation = match reservation {
        ReserveUploadOutcome::Reserved(value) | ReserveUploadOutcome::Existing(value) => value,
    };

    let upload = match state.repo.issue_idempotent(candidate).await {
        Ok(upload) => upload,
        Err(error) => {
            let _ = state
                .admission
                .release(
                    &workspace.tenant_id,
                    &reservation_id,
                    reservation.lease_generation,
                    now_i64,
                )
                .await;
            return Err(map_ingress_error(error));
        }
    };

    let transport = if upload.state == UploadState::Issued {
        let capabilities = state.blob_store.provider_capabilities();
        let direct_safe = capabilities.hard_create_only
            && capabilities.hard_exact_or_max_upload_size
            && (upload.declared_content_type.is_none() || capabilities.signed_content_type);
        if direct_safe {
            let grant_expires_at_ms = checked_add_duration(now_ms, state.config.direct_grant_ttl)?;
            let grant = state
                .blob_store
                .issue_quarantine_upload_grant(
                    &upload.tenant_id,
                    &upload.upload_id,
                    upload.expected_byte_len,
                    upload.declared_content_type.clone(),
                    now_ms,
                    grant_expires_at_ms,
                )
                .await
                .map_err(map_blob_error)?;
            Some(UploadTransportResponse::Direct {
                grant: grant.opaque_url,
                expires_at_ms: grant.expires_at_ms,
            })
        } else {
            Some(UploadTransportResponse::Streamed {
                path: format!("/v1/uploads/{}/content", upload.upload_id),
            })
        }
    } else {
        None
    };

    Ok(Json(IssueUploadResponse {
        upload: (&upload).into(),
        transport,
    }))
}

async fn put_upload_content(
    State(state): State<SourceIngressHttpState>,
    Path(upload_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Body,
) -> Result<Json<UploadStatusResponse>, SourceIngressHttpError> {
    let principal = state
        .auth
        .authenticate_mutation_request(&headers, &jar)
        .await?;
    let upload = authorized_upload(&state, &principal.principal_id, &upload_id).await?;
    if upload.state != UploadState::Issued {
        return Err(SourceIngressHttpError::conflict(
            "upload_content_state_conflict",
        ));
    }
    let now_ms = now_ms_u64()?;
    if now_ms >= upload.expires_at_ms {
        return Err(SourceIngressHttpError::conflict("upload_expired"));
    }
    if let Some(raw) = headers.get(CONTENT_LENGTH) {
        let declared = raw
            .to_str()
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| SourceIngressHttpError::bad_request("content_length_invalid"))?;
        if declared != upload.expected_byte_len {
            return Err(SourceIngressHttpError::bad_request(
                "upload_length_mismatch",
            ));
        }
    }

    let (mut writer, mut reader) = tokio::io::duplex(STREAM_BUFFER_BYTES);
    let mut stream = body.into_data_stream();
    let pump = async move {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|_| SourceIngressHttpError::bad_request("upload_body_read_failed"))?;
            writer
                .write_all(&chunk)
                .await
                .map_err(|_| SourceIngressHttpError::bad_request("upload_body_write_failed"))?;
        }
        writer
            .shutdown()
            .await
            .map_err(|_| SourceIngressHttpError::bad_request("upload_body_write_failed"))
    };
    let create = state.blob_store.create_quarantine_streamed(
        &upload.tenant_id,
        &upload.upload_id,
        upload.expected_byte_len,
        &mut reader,
    );
    let (create_result, pump_result) = tokio::join!(create, pump);
    create_result.map_err(map_blob_error)?;
    pump_result?;

    Ok(Json((&upload).into()))
}

async fn complete_upload(
    State(state): State<SourceIngressHttpState>,
    Path(upload_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<UploadStatusResponse>, SourceIngressHttpError> {
    let principal = state
        .auth
        .authenticate_mutation_request(&headers, &jar)
        .await?;
    let mut upload = authorized_upload(&state, &principal.principal_id, &upload_id).await?;
    let now_ms = now_ms_u64()?;

    if upload.state == UploadState::Issued {
        if now_ms >= upload.expires_at_ms {
            let mut expired = upload.clone();
            expired.state = UploadState::Expired;
            expired.upload_generation = expired
                .upload_generation
                .checked_add(1)
                .ok_or_else(|| SourceIngressHttpError::internal("upload_generation_overflow"))?;
            expired.completed_at_ms = Some(now_ms);
            expired.terminal_code = Some("upload_expired".to_owned());
            upload = state
                .repo
                .compare_and_swap(&upload.upload_id, upload.upload_generation, expired)
                .await
                .map_err(map_ingress_error)?;
            release_admission_best_effort(&state, &upload, now_ms).await;
            return Ok(Json((&upload).into()));
        }

        let metadata = state
            .blob_store
            .inspect_quarantine_upload(&upload.tenant_id, &upload.upload_id)
            .await
            .map_err(map_blob_error)?
            .ok_or_else(|| SourceIngressHttpError::conflict("quarantine_object_missing"))?;
        if metadata.byte_len != upload.expected_byte_len {
            return Err(SourceIngressHttpError::bad_request(
                "upload_length_mismatch",
            ));
        }
        let mut next = upload.clone();
        next.state = UploadState::StoredUnverified;
        next.upload_generation = next
            .upload_generation
            .checked_add(1)
            .ok_or_else(|| SourceIngressHttpError::internal("upload_generation_overflow"))?;
        next.object_version = Some(metadata.storage_generation);
        next.object_etag = Some(metadata.etag);
        next.observed_byte_len = Some(metadata.byte_len);
        next.completed_at_ms = Some(now_ms);
        upload = state
            .repo
            .compare_and_swap(&upload.upload_id, upload.upload_generation, next)
            .await
            .map_err(map_ingress_error)?;
    }

    if matches!(
        upload.state,
        UploadState::StoredUnverified | UploadState::Validating
    ) {
        let validation = state
            .validation
            .validate_and_promote(
                &upload.tenant_id,
                &upload.upload_id,
                now_ms,
                state.scanner.as_ref(),
            )
            .await;
        release_admission_best_effort(&state, &upload, now_ms).await;
        upload = validation.map_err(map_ingress_error)?;
    } else {
        release_admission_best_effort(&state, &upload, now_ms).await;
    }

    Ok(Json((&upload).into()))
}

async fn upload_status(
    State(state): State<SourceIngressHttpState>,
    Path(upload_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<UploadStatusResponse>, SourceIngressHttpError> {
    let principal = state.auth.authenticate_read_request(&headers, &jar).await?;
    let upload = authorized_upload(&state, &principal.principal_id, &upload_id).await?;
    Ok(Json((&upload).into()))
}

async fn create_project(
    State(state): State<SourceIngressHttpState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<CreateProjectBody>,
) -> Result<Json<ProjectCreateResult>, SourceIngressHttpError> {
    let principal = state
        .auth
        .authenticate_mutation_request(&headers, &jar)
        .await?;
    let workspace = state
        .workspace
        .resolve(&principal.principal_id, &body.workspace_id)
        .await
        .map_err(map_workspace_error)?;
    let upload = authorized_upload(&state, &principal.principal_id, &body.upload_id).await?;
    if upload.tenant_id != workspace.tenant_id {
        return Err(SourceIngressHttpError::not_found("upload_not_found"));
    }

    let request = ConsumeUploadRequest {
        tenant_id: workspace.tenant_id,
        upload_id: body.upload_id,
        expected_upload_generation: body.expected_upload_generation,
        workspace_id: body.workspace_id,
        name: body.name,
        client_idempotency_id: body.client_idempotency_id,
        now_ms: now_ms_u64()?,
    };
    let planned = plan_project_identity(&request).map_err(map_ingress_error)?;
    let binding_id = upload
        .durable_binding_id
        .as_deref()
        .ok_or_else(|| SourceIngressHttpError::conflict("source_not_validated_durable"))?;
    let source_sha256 = upload
        .canonical_sha256
        .as_deref()
        .ok_or_else(|| SourceIngressHttpError::conflict("source_not_validated_durable"))?;
    let source_byte_len = upload
        .observed_byte_len
        .ok_or_else(|| SourceIngressHttpError::conflict("source_not_validated_durable"))?;

    let baseline = state
        .baseline
        .produce(
            &upload.tenant_id,
            binding_id,
            source_sha256,
            source_byte_len,
            &planned.document_id,
        )
        .await
        .map_err(map_baseline_error)?;
    let project = state
        .projects
        .create_project_from_upload(request, baseline)
        .await
        .map_err(map_ingress_error)?;
    Ok(Json(project))
}

async fn authorized_upload(
    state: &SourceIngressHttpState,
    principal_id: &str,
    upload_id: &str,
) -> Result<UploadRecord, SourceIngressHttpError> {
    let upload = state
        .repo
        .get(upload_id)
        .await
        .map_err(map_ingress_error)?
        .ok_or_else(|| SourceIngressHttpError::not_found("upload_not_found"))?;
    if upload.principal_id != principal_id {
        return Err(SourceIngressHttpError::not_found("upload_not_found"));
    }
    Ok(upload)
}

async fn release_admission_best_effort(
    state: &SourceIngressHttpState,
    upload: &UploadRecord,
    now_ms: u64,
) {
    let Ok(reservation_id) =
        upload_admission_reservation_id(&upload.tenant_id, &upload.idempotency_key)
    else {
        return;
    };
    let Ok(now_i64) = i64::try_from(now_ms) else {
        return;
    };
    let _ = state
        .admission
        .release(&upload.tenant_id, &reservation_id, 0, now_i64)
        .await;
}

fn new_upload_id() -> Result<String, SourceIngressHttpError> {
    let mut bytes = [0_u8; 16];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| SourceIngressHttpError::internal("upload_id_random_failed"))?;
    let mut out = String::from("upload:");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}")
            .map_err(|_| SourceIngressHttpError::internal("upload_id_format_failed"))?;
    }
    Ok(out)
}

fn now_ms_u64() -> Result<u64, SourceIngressHttpError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SourceIngressHttpError::internal("clock_invalid"))?
        .as_millis();
    u64::try_from(millis).map_err(|_| SourceIngressHttpError::internal("clock_out_of_range"))
}

fn checked_add_duration(now_ms: u64, duration: Duration) -> Result<u64, SourceIngressHttpError> {
    let delta = u64::try_from(duration.as_millis())
        .map_err(|_| SourceIngressHttpError::internal("duration_out_of_range"))?;
    now_ms
        .checked_add(delta)
        .ok_or_else(|| SourceIngressHttpError::internal("clock_out_of_range"))
}

#[derive(Debug)]
pub enum SourceIngressHttpError {
    Auth(AuthHttpError),
    Api {
        status: StatusCode,
        code: &'static str,
        retry_after_seconds: Option<u64>,
    },
}

impl SourceIngressHttpError {
    fn api(status: StatusCode, code: &'static str) -> Self {
        Self::Api {
            status,
            code,
            retry_after_seconds: None,
        }
    }

    fn rate_limited(code: &'static str, retry_at_ms: Option<i64>) -> Self {
        Self::Api {
            status: StatusCode::TOO_MANY_REQUESTS,
            code,
            retry_after_seconds: retry_after_seconds(retry_at_ms),
        }
    }

    fn bad_request(code: &'static str) -> Self {
        Self::api(StatusCode::BAD_REQUEST, code)
    }

    fn not_found(code: &'static str) -> Self {
        Self::api(StatusCode::NOT_FOUND, code)
    }

    fn conflict(code: &'static str) -> Self {
        Self::api(StatusCode::CONFLICT, code)
    }

    fn payload_too_large(code: &'static str) -> Self {
        Self::api(StatusCode::PAYLOAD_TOO_LARGE, code)
    }

    fn internal(code: &'static str) -> Self {
        Self::api(StatusCode::INTERNAL_SERVER_ERROR, code)
    }
}

impl From<AuthHttpError> for SourceIngressHttpError {
    fn from(value: AuthHttpError) -> Self {
        Self::Auth(value)
    }
}

impl fmt::Display for SourceIngressHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auth(error) => error.fmt(formatter),
            Self::Api { code, .. } => formatter.write_str(code),
        }
    }
}

impl std::error::Error for SourceIngressHttpError {}

impl IntoResponse for SourceIngressHttpError {
    fn into_response(self) -> Response {
        match self {
            Self::Auth(error) => error.into_response(),
            Self::Api {
                status,
                code,
                retry_after_seconds,
            } => {
                let mut response =
                    (status, Json(serde_json::json!({ "error": code }))).into_response();
                if let Some(seconds) = retry_after_seconds
                    && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
                {
                    response.headers_mut().insert(RETRY_AFTER, value);
                }
                response
            }
        }
    }
}

fn retry_after_seconds(retry_at_ms: Option<i64>) -> Option<u64> {
    let retry_at_ms = retry_at_ms?;
    if retry_at_ms < 0 {
        return None;
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())?;
    let remaining_ms = retry_at_ms.saturating_sub(now_ms);
    let seconds = if remaining_ms <= 0 {
        1
    } else {
        u64::try_from(remaining_ms).ok()?.saturating_add(999) / 1000
    };
    Some(seconds.max(1))
}

fn map_workspace_error(error: WorkspaceContextError) -> SourceIngressHttpError {
    match error.code {
        "workspace_membership_denied" => {
            SourceIngressHttpError::api(StatusCode::FORBIDDEN, error.code)
        }
        "workspace_context_identity_invalid" => SourceIngressHttpError::bad_request(error.code),
        _ => SourceIngressHttpError::internal(error.code),
    }
}

fn map_admission_error(error: UploadAdmissionError) -> SourceIngressHttpError {
    match error.code {
        "upload_bytes_too_large" => SourceIngressHttpError::payload_too_large(error.code),
        "upload_admission_principal_missing" | "upload_admission_principal_disabled" => {
            SourceIngressHttpError::api(StatusCode::FORBIDDEN, error.code)
        }
        "upload_admission_principal_concurrency"
        | "upload_admission_tenant_concurrency"
        | "upload_admission_principal_bytes"
        | "upload_admission_tenant_bytes" => {
            SourceIngressHttpError::rate_limited(error.code, error.retry_at_ms)
        }
        "upload_admission_idempotency_conflict"
        | "upload_admission_already_released"
        | "upload_admission_lease_expired" => SourceIngressHttpError::conflict(error.code),
        "upload_admission_identity_invalid" | "upload_admission_request_hash_invalid" => {
            SourceIngressHttpError::bad_request(error.code)
        }
        _ => SourceIngressHttpError::internal(error.code),
    }
}

fn map_ingress_error(error: IngressError) -> SourceIngressHttpError {
    match error.code {
        "upload_not_found" => SourceIngressHttpError::not_found(error.code),
        "upload_access_denied" | "tenant_mismatch" => {
            SourceIngressHttpError::not_found("upload_not_found")
        }
        "upload_size_rejected" => SourceIngressHttpError::payload_too_large(error.code),
        "idempotency_conflict"
        | "stale_upload_generation"
        | "source_not_validated_durable"
        | "quarantine_object_missing"
        | "upload_not_complete" => SourceIngressHttpError::conflict(error.code),
        "invalid_config"
        | "invalid_expiry"
        | "invalid_content_type"
        | "invalid_project_name"
        | "upload_length_mismatch"
        | "source_ingress_identity_invalid" => SourceIngressHttpError::bad_request(error.code),
        "source_malware_scanner_unavailable" | "source_malware_scanner_timeout" => {
            SourceIngressHttpError::api(StatusCode::SERVICE_UNAVAILABLE, error.code)
        }
        "malware_detected" | "pub_size_limit" => {
            SourceIngressHttpError::api(StatusCode::UNPROCESSABLE_ENTITY, error.code)
        }
        _ => SourceIngressHttpError::internal(error.code),
    }
}

fn map_blob_error(error: BlobStoreError) -> SourceIngressHttpError {
    match error.code {
        "invalid_upload_size" | "blob_length_mismatch" | "content_hash_mismatch" => {
            SourceIngressHttpError::bad_request(error.code)
        }
        "quarantine_object_missing" => SourceIngressHttpError::conflict(error.code),
        "quarantine_object_exists" | "physical_key_collision" => {
            SourceIngressHttpError::conflict(error.code)
        }
        "blob_provider_config_invalid"
        | "blob_provider_error"
        | "provider_unknown_unreconciled" => {
            SourceIngressHttpError::api(StatusCode::BAD_GATEWAY, error.code)
        }
        _ => SourceIngressHttpError::internal(error.code),
    }
}

fn map_baseline_error(error: SourceBaselineError) -> SourceIngressHttpError {
    match error.code {
        "source_baseline_worker_failed"
        | "source_baseline_receipt_missing"
        | "source_baseline_receipt_invalid"
        | "source_baseline_receipt_identity_mismatch" => {
            SourceIngressHttpError::api(StatusCode::UNPROCESSABLE_ENTITY, error.code)
        }
        "source_baseline_worker_timeout" => {
            SourceIngressHttpError::api(StatusCode::GATEWAY_TIMEOUT, error.code)
        }
        _ => SourceIngressHttpError::internal(error.code),
    }
}
