use std::{
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use chaptera_cdm_model::AUTHORING_REVISION_SCHEMA_V1;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    auth_http::{AuthHttpError, AuthHttpState},
    blob_store::{BlobStoreError, BlobStoreService},
    jobs_runtime::{
        AuthorizedExportDownloadV1, CreateExportJobRequestV1, ExportJobSnapshotV1, JobsRuntime,
        JobsRuntimeError,
    },
    source_authority::{SourceAuthorityError, SqliteDocumentSourceAuthority},
    sqlite_store::{SqliteRevisionStore, SqliteStoreError},
};

pub const EXPORT_CREATE_V1: &str = "chaptera.export-create.v1";
pub const EXPORT_JOB_HTTP_V1: &str = "chaptera.export-job-http.v1";
pub const EXPORT_DOWNLOAD_V1: &str = "chaptera.export-download.v1";
const DOWNLOAD_GRANT_TTL_MS: u64 = 5 * 60 * 1000;

#[derive(Clone)]
pub struct ProductExportHttpState {
    auth: AuthHttpState,
    source: SqliteDocumentSourceAuthority,
    revisions: SqliteRevisionStore,
    jobs: JobsRuntime,
    blobs: BlobStoreService,
}

impl ProductExportHttpState {
    pub fn new(
        auth: AuthHttpState,
        source: SqliteDocumentSourceAuthority,
        revisions: SqliteRevisionStore,
        jobs: JobsRuntime,
        blobs: BlobStoreService,
    ) -> Self {
        Self {
            auth,
            source,
            revisions,
            jobs,
            blobs,
        }
    }
}

pub fn router(state: ProductExportHttpState) -> Router {
    Router::new()
        .route("/v1/exports", post(create_export))
        .route("/v1/exports/{job_id}", get(export_status))
        .route("/v1/exports/{job_id}/cancel", post(cancel_export))
        .route("/v1/exports/{job_id}/download", post(authorize_download))
        .with_state(state)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateExportHttpV1 {
    protocol_version: String,
    document_id: String,
    revision_id: String,
    target_profile: String,
    layout_environment_id: String,
    client_request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadRequestV1 {
    artifact_id: String,
}

#[derive(Debug, Serialize)]
struct ExportJobHttpV1 {
    protocol_version: &'static str,
    job_id: String,
    document_id: String,
    revision_id: String,
    target_profile: String,
    layout_environment_id: String,
    status: &'static str,
    artifact_id: Option<String>,
    loss_report_id: Option<String>,
    error_code: Option<String>,
    progress_percent: Option<u8>,
}

#[derive(Debug, Serialize)]
struct ExportDownloadResponseV1 {
    protocol_version: &'static str,
    job_id: String,
    artifact_id: String,
    download_handle: String,
    expires_at_ms: u64,
}

async fn create_export(
    State(state): State<ProductExportHttpState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(request): Json<CreateExportHttpV1>,
) -> Result<Json<ExportJobHttpV1>, ProductExportHttpError> {
    validate_create_request(&request)?;

    let principal = state
        .auth
        .authenticate_mutation_request(&headers, &jar)
        .await
        .map_err(ProductExportHttpError::Auth)?;
    let source = state
        .source
        .resolve_by_document_id(&request.document_id)
        .await
        .map_err(ProductExportHttpError::Source)?;
    let binding = state
        .revisions
        .require_revision_identity(&request.document_id, &request.revision_id)
        .await
        .map_err(ProductExportHttpError::Store)?;
    if binding.canonical_schema_version != AUTHORING_REVISION_SCHEMA_V1 {
        return Err(ProductExportHttpError::conflict(
            "canonical_revision_schema_unsupported",
            "requested revision is not bound to the supported canonical AuthoringRevision schema",
        ));
    }

    let now = now_ms()?;
    let snapshot = state
        .jobs
        .create_export(CreateExportJobRequestV1 {
            tenant_id: source.tenant_id,
            document_id: request.document_id,
            principal_id: principal.principal_id,
            exact_revision_id: request.revision_id,
            canonical_authoring_revision_id: binding.canonical_revision_id,
            target_profile: request.target_profile,
            layout_environment_id: request.layout_environment_id,
            client_request_id: request.client_request_id.clone(),
            operation_id: format!("export:create:{}", request.client_request_id),
            now_ms: now,
        })
        .await
        .map_err(ProductExportHttpError::Jobs)?;
    Ok(Json(job_response(snapshot, None)))
}

async fn export_status(
    State(state): State<ProductExportHttpState>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<ExportJobHttpV1>, ProductExportHttpError> {
    require_ident(&job_id, "job_id")?;
    let principal = state
        .auth
        .authenticate_read_request(&headers, &jar)
        .await
        .map_err(ProductExportHttpError::Auth)?;
    let now = now_ms()?;
    let snapshot = state
        .jobs
        .status_by_job_id(
            &principal.principal_id,
            &job_id,
            "export:status",
            now,
        )
        .await
        .map_err(ProductExportHttpError::Jobs)?;

    let publication = if snapshot.status == "succeeded" {
        Some(
            state
                .jobs
                .authorize_download_by_job_id(
                    &principal.principal_id,
                    &job_id,
                    "export:status-ready",
                    now,
                )
                .await
                .map_err(ProductExportHttpError::Jobs)?,
        )
    } else {
        None
    };
    Ok(Json(job_response(snapshot, publication.as_ref())))
}

async fn cancel_export(
    State(state): State<ProductExportHttpState>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<ExportJobHttpV1>, ProductExportHttpError> {
    require_ident(&job_id, "job_id")?;
    let principal = state
        .auth
        .authenticate_mutation_request(&headers, &jar)
        .await
        .map_err(ProductExportHttpError::Auth)?;
    let snapshot = state
        .jobs
        .request_cancel_by_job_id(
            &principal.principal_id,
            &job_id,
            "export:cancel",
            now_ms()?,
        )
        .await
        .map_err(ProductExportHttpError::Jobs)?;
    Ok(Json(job_response(snapshot, None)))
}

async fn authorize_download(
    State(state): State<ProductExportHttpState>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(request): Json<DownloadRequestV1>,
) -> Result<Json<ExportDownloadResponseV1>, ProductExportHttpError> {
    require_ident(&job_id, "job_id")?;
    require_ident(&request.artifact_id, "artifact_id")?;
    let principal = state
        .auth
        .authenticate_mutation_request(&headers, &jar)
        .await
        .map_err(ProductExportHttpError::Auth)?;
    let now = now_ms()?;
    let publication = state
        .jobs
        .authorize_download_by_job_id(
            &principal.principal_id,
            &job_id,
            "export:download",
            now,
        )
        .await
        .map_err(ProductExportHttpError::Jobs)?;
    if request.artifact_id != publication.artifact_binding_id {
        return Err(ProductExportHttpError::conflict(
            "artifact_identity_mismatch",
            "requested artifact_id differs from the visible authorized export publication",
        ));
    }
    let source = state
        .source
        .resolve_by_document_id(&publication.document_id)
        .await
        .map_err(ProductExportHttpError::Source)?;
    let now_u64 = u64::try_from(now)
        .map_err(|_| ProductExportHttpError::internal("clock_out_of_range", "negative clock"))?;
    let expires_at_ms = now_u64
        .checked_add(DOWNLOAD_GRANT_TTL_MS)
        .ok_or_else(|| ProductExportHttpError::internal("clock_out_of_range", "grant expiry overflow"))?;
    let grant = state
        .blobs
        .issue_download_grant(
            &source.tenant_id,
            &publication.artifact_binding_id,
            now_u64,
            expires_at_ms,
        )
        .await
        .map_err(ProductExportHttpError::Blob)?;

    Ok(Json(ExportDownloadResponseV1 {
        protocol_version: EXPORT_DOWNLOAD_V1,
        job_id,
        artifact_id: publication.artifact_binding_id,
        download_handle: grant.opaque_url,
        expires_at_ms: grant.expires_at_ms,
    }))
}

fn job_response(
    snapshot: ExportJobSnapshotV1,
    publication: Option<&AuthorizedExportDownloadV1>,
) -> ExportJobHttpV1 {
    let status = match snapshot.status.as_str() {
        "queued" => "queued",
        "running" => "running",
        "succeeded" => "ready",
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "failed",
    };
    ExportJobHttpV1 {
        protocol_version: EXPORT_JOB_HTTP_V1,
        job_id: snapshot.job_id,
        document_id: snapshot.document_id,
        revision_id: snapshot.exact_revision_id,
        target_profile: snapshot.target_profile,
        layout_environment_id: snapshot.layout_environment_id,
        status,
        artifact_id: publication.map(|value| value.artifact_binding_id.clone()),
        loss_report_id: publication.map(|value| value.loss_binding_id.clone()),
        error_code: if status == "failed" {
            snapshot
                .terminal_code
                .or_else(|| Some("export_failed".to_owned()))
        } else {
            None
        },
        progress_percent: None,
    }
}

fn validate_create_request(request: &CreateExportHttpV1) -> Result<(), ProductExportHttpError> {
    if request.protocol_version != EXPORT_CREATE_V1 {
        return Err(ProductExportHttpError::bad_request(
            "protocol_version_invalid",
            "chaptera.export-create.v1 is required",
        ));
    }
    for (label, value) in [
        ("document_id", request.document_id.as_str()),
        ("revision_id", request.revision_id.as_str()),
        ("target_profile", request.target_profile.as_str()),
        (
            "layout_environment_id",
            request.layout_environment_id.as_str(),
        ),
        ("client_request_id", request.client_request_id.as_str()),
    ] {
        require_ident(value, label)?;
    }
    Ok(())
}

fn require_ident(value: &str, label: &'static str) -> Result<(), ProductExportHttpError> {
    if value.is_empty()
        || value.len() > 256
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b':' | b'@' | b'/' | b'-')
        })
    {
        return Err(ProductExportHttpError::bad_request(
            "invalid_identity",
            format!("invalid {label}"),
        ));
    }
    Ok(())
}

fn now_ms() -> Result<i64, ProductExportHttpError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProductExportHttpError::internal("clock_invalid", "system clock is before epoch"))?
        .as_millis();
    i64::try_from(millis)
        .map_err(|_| ProductExportHttpError::internal("clock_out_of_range", "system clock is out of range"))
}

#[derive(Debug)]
enum ProductExportHttpError {
    Auth(AuthHttpError),
    Source(SourceAuthorityError),
    Store(SqliteStoreError),
    Jobs(JobsRuntimeError),
    Blob(BlobStoreError),
    Http {
        status: StatusCode,
        code: &'static str,
        message: String,
    },
}

impl ProductExportHttpError {
    fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
        }
    }

    fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::CONFLICT,
            code,
            message: message.into(),
        }
    }

    fn internal(code: &'static str, message: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code,
            message: message.into(),
        }
    }
}

impl IntoResponse for ProductExportHttpError {
    fn into_response(self) -> Response {
        match self {
            Self::Auth(error) => error.into_response(),
            Self::Source(error) => (
                StatusCode::NOT_FOUND,
                Json(json!({"error":{"code":error.code,"message":error.message}})),
            )
                .into_response(),
            Self::Store(error) => {
                let status = if error.code == "canonical_revision_unbound" {
                    StatusCode::CONFLICT
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                (
                    status,
                    Json(json!({"error":{"code":error.code,"message":error.message}})),
                )
                    .into_response()
            }
            Self::Jobs(error) => {
                let status = match error.code {
                    "job_not_found" => StatusCode::NOT_FOUND,
                    "idempotency_conflict"
                    | "job_scope_mismatch"
                    | "job_payload_scope_mismatch"
                    | "export_artifact_not_ready"
                    | "export_artifact_not_visible" => StatusCode::CONFLICT,
                    "grant_missing" | "authz_denied" | "authz_expired" => StatusCode::FORBIDDEN,
                    "invalid_client_request_id" | "invalid_operation_id" => StatusCode::BAD_REQUEST,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                (
                    status,
                    Json(json!({"error":{"code":error.code,"message":error.message}})),
                )
                    .into_response()
            }
            Self::Blob(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":{"code":error.code,"message":error.message}})),
            )
                .into_response(),
            Self::Http {
                status,
                code,
                message,
            } => (
                status,
                Json(json!({"error":{"code":code,"message":message}})),
            )
                .into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_job_shape_never_fabricates_progress_or_artifact_before_ready() {
        let snapshot = ExportJobSnapshotV1 {
            job_id: "export-job:one".into(),
            tenant_id: "tenant:one".into(),
            document_id: "doc:one".into(),
            exact_revision_id: format!("sha256:{}", "a".repeat(64)),
            canonical_authoring_revision_id: "b".repeat(64),
            target_profile: "idml:bounded-editable".into(),
            layout_environment_id: format!("sha256:{}", "c".repeat(64)),
            status: "running".into(),
            attempt: 1,
            max_attempts: 3,
            cancel_requested: false,
            terminal_code: None,
        };
        let response = job_response(snapshot, None);
        assert_eq!(response.status, "running");
        assert_eq!(response.progress_percent, None);
        assert_eq!(response.artifact_id, None);
        assert_eq!(response.loss_report_id, None);
    }
}
