use std::{
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use chaptera_cdm_model::{
    AUTHORING_REVISION_SCHEMA_V1, AuthoringRevisionIdV1, canonical_revision_json_v1,
    derive_authoring_revision_id_v1,
};
use pub_editor::{EditOperation, LengthEmu, NodeId, Sha256Digest, open_mature_0x2c_editor};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    auth_http::{AuthHttpError, AuthHttpState},
    authz_runtime::{
        AuthzError, CAP_VIEW, SqliteAuthorizedRevisionCommitter, SqliteAuthzAuthority,
    },
    revision_materializer::{
        BlobStoreExactSourceLoader, EDITOR_REVISION_EVENT_SCHEMA_V1,
        EDITOR_REVISION_EVENT_SEMANTIC_SCHEMA_VERSION, EditorRevisionEventV1,
        ExactRevisionMaterializer, ExactSourceLoader, PubEditorReplayEngine,
        RevisionMaterializerError,
        decode_editor_revision_event_v1, encode_editor_revision_event_v1, project_sha256,
    },
    source_authority::{SourceAuthorityError, SqliteDocumentSourceAuthority},
    source_baseline::{SourceBaselineError, derive_commit_revision_identities},
    sqlite_store::{RevisionEdge, RevisionIdentityBinding, SqliteRevisionStore},
};

pub const COMMIT_REQUEST_V1: &str = "chaptera.commit-request.v1";
pub const COMMIT_ACCEPTED_V1: &str = "chaptera.commit-accepted.v1";
pub const CURRENT_DOCUMENT_V1: &str = "chaptera.current-document.v1";

#[derive(Clone)]
pub struct ProductApiHttpState {
    auth: AuthHttpState,
    source: SqliteDocumentSourceAuthority,
    authz: SqliteAuthzAuthority,
    revisions: SqliteRevisionStore,
    committer: SqliteAuthorizedRevisionCommitter,
    materializer: Arc<ExactRevisionMaterializer>,
}

impl ProductApiHttpState {
    pub fn new(
        auth: AuthHttpState,
        source: SqliteDocumentSourceAuthority,
        authz: SqliteAuthzAuthority,
        revisions: SqliteRevisionStore,
        source_loader: BlobStoreExactSourceLoader,
    ) -> Result<Self, AuthzError> {
        Self::with_source_loader(
            auth,
            source,
            authz,
            revisions,
            Arc::new(source_loader),
        )
    }

    fn with_source_loader(
        auth: AuthHttpState,
        source: SqliteDocumentSourceAuthority,
        authz: SqliteAuthzAuthority,
        revisions: SqliteRevisionStore,
        source_loader: Arc<dyn ExactSourceLoader>,
    ) -> Result<Self, AuthzError> {
        let committer = SqliteAuthorizedRevisionCommitter::new(authz.clone(), revisions.clone())?;
        let materializer = Arc::new(ExactRevisionMaterializer::new(
            Arc::new(source.clone()),
            source_loader,
            revisions.clone(),
            Arc::new(PubEditorReplayEngine),
        ));
        Ok(Self {
            auth,
            source,
            authz,
            revisions,
            committer,
            materializer,
        })
    }
}

pub fn router(state: ProductApiHttpState) -> Router {
    Router::new()
        .route("/v1/documents/{document_id}/current", get(current_document))
        .route("/v1/documents/{document_id}/commit", post(commit_move_node))
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct CurrentDocumentResponse {
    protocol_version: &'static str,
    document_id: String,
    source_hash: String,
    revision_id: String,
    revision_cursor: i64,
    canonical_revision_schema_version: String,
    canonical_authoring_revision_id: String,
    project: pub_editor::EditorProject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitRequestV1 {
    protocol_version: String,
    document_id: String,
    source_hash: String,
    base_revision_id: String,
    client_operation_id: String,
    command: MoveNodeToV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveNodeToV1 {
    kind: String,
    node_id: String,
    x_emu: i64,
    y_emu: i64,
}

#[derive(Debug, Serialize)]
struct CommitAcceptedResponse {
    protocol_version: &'static str,
    document_id: String,
    source_hash: String,
    base_revision_id: String,
    revision_id: String,
    state_id: String,
    client_operation_id: String,
    canonical_operation: EditOperation,
    project_schema_version: String,
    canonical_revision_schema_version: String,
    canonical_authoring_revision_id: String,
    replayed: bool,
    scene_refresh: &'static str,
}

#[derive(Debug, Clone)]
struct CurrentHead {
    revision_id: String,
    cursor: i64,
}

async fn current_document(
    State(state): State<ProductApiHttpState>,
    Path(document_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<CurrentDocumentResponse>, ProductApiError> {
    let principal = state
        .auth
        .authenticate_read_request(&headers, &jar)
        .await
        .map_err(ProductApiError::Auth)?;

    let source = state
        .source
        .resolve_by_document_id(&document_id)
        .await
        .map_err(ProductApiError::Source)?;

    state
        .authz
        .authorize(
            &source.tenant_id,
            &document_id,
            &principal.principal_id,
            CAP_VIEW,
            "product-open",
            now_ms()?,
        )
        .await
        .map_err(ProductApiError::Authz)?;

    let head = current_head(&state.revisions, &source).await?;
    let materialized = state
        .materializer
        .materialize(&source.tenant_id, &document_id, &head.revision_id)
        .await
        .map_err(ProductApiError::Materializer)?;

    Ok(Json(CurrentDocumentResponse {
        protocol_version: CURRENT_DOCUMENT_V1,
        document_id,
        source_hash: source.source_sha256,
        revision_id: head.revision_id,
        revision_cursor: head.cursor,
        canonical_revision_schema_version: materialized.canonical_revision_schema_version,
        canonical_authoring_revision_id: materialized.canonical_authoring_revision_id,
        project: materialized.project,
    }))
}

async fn commit_move_node(
    State(state): State<ProductApiHttpState>,
    Path(document_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(request): Json<CommitRequestV1>,
) -> Result<Json<CommitAcceptedResponse>, ProductApiError> {
    validate_request(&request, &document_id)?;

    let principal = state
        .auth
        .authenticate_mutation_request(&headers, &jar)
        .await
        .map_err(ProductApiError::Auth)?;
    let source = state
        .source
        .resolve_by_document_id(&document_id)
        .await
        .map_err(ProductApiError::Source)?;
    if request.source_hash != source.source_sha256 {
        return Err(ProductApiError::bad_request(
            "source_hash_mismatch",
            "commit request source_hash differs from durable source authority",
        ));
    }

    let request_hash = request_hash(&request)?;
    let now = now_ms()?;

    if let Some(existing) = state
        .committer
        .reconcile_geometry_revision(
            &source.tenant_id,
            &document_id,
            &principal.principal_id,
            &request.client_operation_id,
            &request_hash,
            now,
        )
        .await
        .map_err(ProductApiError::Authz)?
    {
        return accepted_from_receipt(&state, &source, existing, true).await;
    }

    let head = current_head(&state.revisions, &source).await?;
    if request.base_revision_id != head.revision_id {
        return Err(ProductApiError::conflict(
            "stale_revision",
            "base_revision_id is no longer the current RevisionStream head",
        ));
    }

    let materialized = state
        .materializer
        .materialize_state(&source.tenant_id, &document_id, &head.revision_id)
        .await
        .map_err(ProductApiError::Materializer)?;

    let source_hash = Sha256Digest::from_str(&source.source_sha256).map_err(|_| {
        ProductApiError::internal(
            "source_hash_invalid",
            "durable source authority contains an invalid SHA-256 identity",
        )
    })?;
    let mut session =
        open_mature_0x2c_editor(&materialized.source_bytes, source_hash).map_err(|error| {
            ProductApiError::internal(
                "editor_source_unsupported",
                format!("canonical editor could not open durable source: {error}"),
            )
        })?;
    session
        .apply_project(&materialized.receipt.project)
        .map_err(|error| {
            ProductApiError::internal(
                "editor_replay_failed",
                format!("canonical editor could not replay exact base revision: {error}"),
            )
        })?;

    let node_id: NodeId =
        serde_json::from_value(serde_json::Value::String(request.command.node_id.clone()))
            .map_err(|_| {
                ProductApiError::bad_request("node_id_invalid", "node_id is not canonical")
            })?;

    let operation = session
        .move_node_to(
            node_id,
            LengthEmu::new(request.command.x_emu),
            LengthEmu::new(request.command.y_emu),
        )
        .map_err(|error| {
            ProductApiError::bad_request(
                "move_node_rejected",
                format!("canonical MoveNode rejected the intent: {error}"),
            )
        })?;
    if !matches!(operation, EditOperation::MoveNode { .. }) {
        return Err(ProductApiError::internal(
            "move_node_operation_invalid",
            "canonical editor returned a non-MoveNode operation",
        ));
    }

    let resulting_project =
        crate::revision_materializer::cloud_revision_project(&session.project());
    let before_project_sha256 = materialized.receipt.project_sha256.clone();
    let after_project_sha256 =
        project_sha256(&resulting_project).map_err(ProductApiError::Materializer)?;

    let identities = derive_commit_revision_identities(
        &document_id,
        &source.source_sha256,
        &resulting_project.schema_version,
        &resulting_project,
        &head.revision_id,
        &operation,
    )
    .map_err(ProductApiError::Baseline)?;

    let parent_canonical: AuthoringRevisionIdV1 = materialized
        .receipt
        .canonical_authoring_revision_id
        .parse()
        .map_err(|_| {
            ProductApiError::internal(
                "canonical_parent_revision_invalid",
                "materialized parent canonical revision identity is invalid",
            )
        })?;
    let canonical_child =
        derive_authoring_revision_id_v1(&resulting_project, Some(parent_canonical))
            .map_err(|_| {
                ProductApiError::internal(
                    "canonical_child_revision_failed",
                    "canonical child AuthoringRevisionId derivation failed",
                )
            })?
            .to_string();

    let event = EditorRevisionEventV1 {
        schema_version: EDITOR_REVISION_EVENT_SCHEMA_V1.to_owned(),
        source_sha256: source.source_sha256.clone(),
        before_project_sha256,
        after_project_sha256: after_project_sha256.clone(),
        authoring_root_hash: None,
        operation: operation.clone(),
    };
    let canonical_event =
        encode_editor_revision_event_v1(&event).map_err(ProductApiError::Materializer)?;

    let child_cursor = head.cursor.checked_add(1).ok_or_else(|| {
        ProductApiError::internal("revision_cursor_overflow", "revision cursor overflow")
    })?;

    let edge = RevisionEdge {
        document_id: document_id.clone(),
        parent_revision: head.revision_id.clone(),
        parent_cursor: head.cursor,
        operation_id: request.client_operation_id.clone(),
        request_hash,
        canonical_event,
        child_revision: identities.service_revision_id.clone(),
        child_cursor,
        resulting_state_hash: after_project_sha256,
        authoring_root_hash: None,
        semantic_schema_version: EDITOR_REVISION_EVENT_SEMANTIC_SCHEMA_VERSION,
        committed_at_ms: now,
    };
    let binding = RevisionIdentityBinding {
        document_id: document_id.clone(),
        service_revision_id: identities.service_revision_id.clone(),
        canonical_schema_version: AUTHORING_REVISION_SCHEMA_V1.to_owned(),
        canonical_revision_id: canonical_child,
        bound_at_ms: now,
    };

    let committed = state
        .committer
        .commit_geometry_revision(
            &source.tenant_id,
            &principal.principal_id,
            edge,
            binding,
            now,
        )
        .await
        .map_err(ProductApiError::Authz)?;

    accepted_from_receipt(&state, &source, committed, false).await
}

async fn current_head(
    revisions: &SqliteRevisionStore,
    source: &crate::revision_materializer::AuthorizedDocumentSource,
) -> Result<CurrentHead, ProductApiError> {
    let edges = revisions
        .load_document_edges(&source.document_id)
        .await
        .map_err(ProductApiError::Store)?;
    if let Some(last) = edges.last() {
        Ok(CurrentHead {
            revision_id: last.child_revision.clone(),
            cursor: last.child_cursor,
        })
    } else {
        Ok(CurrentHead {
            revision_id: source.baseline_revision_id.clone(),
            cursor: source.baseline_cursor,
        })
    }
}

async fn accepted_from_receipt(
    state: &ProductApiHttpState,
    source: &crate::revision_materializer::AuthorizedDocumentSource,
    receipt: crate::authz_runtime::AuthorizedRevisionCommitReceipt,
    replayed_hint: bool,
) -> Result<Json<CommitAcceptedResponse>, ProductApiError> {
    let event =
        decode_editor_revision_event_v1(&receipt.edge).map_err(ProductApiError::Materializer)?;
    let child = state
        .materializer
        .materialize(
            &source.tenant_id,
            &receipt.edge.document_id,
            &receipt.edge.child_revision,
        )
        .await
        .map_err(ProductApiError::Materializer)?;

    let identities = derive_commit_revision_identities(
        &receipt.edge.document_id,
        &event.source_sha256,
        &child.project.schema_version,
        &child.project,
        &receipt.edge.parent_revision,
        &event.operation,
    )
    .map_err(ProductApiError::Baseline)?;
    if identities.service_revision_id != receipt.edge.child_revision {
        return Err(ProductApiError::internal(
            "accepted_revision_identity_mismatch",
            "durable accepted edge differs from the existing V1 service revision law",
        ));
    }

    Ok(Json(CommitAcceptedResponse {
        protocol_version: COMMIT_ACCEPTED_V1,
        document_id: receipt.edge.document_id.clone(),
        source_hash: event.source_sha256,
        base_revision_id: receipt.edge.parent_revision,
        revision_id: receipt.edge.child_revision,
        state_id: identities.state_id,
        client_operation_id: receipt.edge.operation_id,
        canonical_operation: event.operation,
        project_schema_version: child.project.schema_version,
        canonical_revision_schema_version: receipt.binding.canonical_schema_version,
        canonical_authoring_revision_id: receipt.binding.canonical_revision_id,
        replayed: replayed_hint || receipt.replayed,
        scene_refresh: "full_snapshot",
    }))
}

fn validate_request(
    request: &CommitRequestV1,
    path_document_id: &str,
) -> Result<(), ProductApiError> {
    if request.protocol_version != COMMIT_REQUEST_V1 {
        return Err(ProductApiError::bad_request(
            "protocol_version_invalid",
            "chaptera.commit-request.v1 is required",
        ));
    }
    if request.document_id != path_document_id {
        return Err(ProductApiError::bad_request(
            "document_id_mismatch",
            "path document_id differs from request document_id",
        ));
    }
    if request.command.kind != "move_node_to" {
        return Err(ProductApiError::bad_request(
            "command_unsupported",
            "only move_node_to is admitted by this product slice",
        ));
    }
    require_ident(&request.document_id, "document_id")?;
    require_ident(&request.client_operation_id, "client_operation_id")?;
    require_hash(&request.source_hash, "source_hash")?;
    require_revision_id(&request.base_revision_id)?;
    Ok(())
}

fn request_hash(request: &CommitRequestV1) -> Result<String, ProductApiError> {
    let bytes = canonical_revision_json_v1(request).map_err(|_| {
        ProductApiError::internal(
            "request_hash_failed",
            "commit request canonicalization failed",
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn require_ident(value: &str, label: &'static str) -> Result<(), ProductApiError> {
    if value.is_empty()
        || value.len() > 192
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'@' | b'/' | b'-')
        })
    {
        return Err(ProductApiError::bad_request(
            "invalid_identity",
            format!("invalid {label}"),
        ));
    }
    Ok(())
}

fn require_hash(value: &str, label: &'static str) -> Result<(), ProductApiError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ProductApiError::bad_request(
            "invalid_hash",
            format!("{label} must be 64 lowercase SHA-256 hex characters"),
        ));
    }
    Ok(())
}

fn require_revision_id(value: &str) -> Result<(), ProductApiError> {
    let Some(raw) = value.strip_prefix("sha256:") else {
        return Err(ProductApiError::bad_request(
            "invalid_revision_id",
            "base_revision_id must use sha256: identity",
        ));
    };
    require_hash(raw, "base_revision_id")
}

fn now_ms() -> Result<i64, ProductApiError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProductApiError::internal("clock_invalid", "system clock is before epoch"))?
        .as_millis();
    i64::try_from(millis).map_err(|_| {
        ProductApiError::internal("clock_out_of_range", "system clock is out of range")
    })
}

#[derive(Debug)]
enum ProductApiError {
    Auth(AuthHttpError),
    Authz(AuthzError),
    Source(SourceAuthorityError),
    Materializer(RevisionMaterializerError),
    Store(crate::sqlite_store::SqliteStoreError),
    Baseline(SourceBaselineError),
    Http {
        status: StatusCode,
        code: &'static str,
        message: String,
    },
}

impl ProductApiError {
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

impl IntoResponse for ProductApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Auth(error) => error.into_response(),
            Self::Authz(error) => {
                let status = match error.code {
                    "stale_revision" | "idempotency_conflict" => StatusCode::CONFLICT,
                    "authz_denied" | "authz_expired" => StatusCode::FORBIDDEN,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                (
                    status,
                    Json(json!({"error": {"code": error.code, "message": error.message}})),
                )
                    .into_response()
            }
            Self::Source(error) => {
                let status = if error.code == "document_source_not_found" {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                (
                    status,
                    Json(json!({"error": {"code": error.code, "message": error.message}})),
                )
                    .into_response()
            }
            Self::Materializer(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": {"code": error.code, "message": error.message}})),
            )
                .into_response(),
            Self::Store(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": {"code": error.code, "message": error.message}})),
            )
                .into_response(),
            Self::Baseline(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": {"code": error.code, "message": error.message}})),
            )
                .into_response(),
            Self::Http {
                status,
                code,
                message,
            } => (
                status,
                Json(json!({"error": {"code": code, "message": message}})),
            )
                .into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_shape_rejects_tenant_and_authoritative_before_fields() {
        let value = json!({
            "protocol_version": COMMIT_REQUEST_V1,
            "document_id": "document-1",
            "source_hash": "a".repeat(64),
            "base_revision_id": format!("sha256:{}", "b".repeat(64)),
            "client_operation_id": "operation-1",
            "tenant_id": "tenant-a",
            "command": {
                "kind": "move_node_to",
                "node_id": "00112233-4455-6677-8899-aabbccddeeff",
                "x_emu": 1,
                "y_emu": 2,
                "before": {"x": 0, "y": 0}
            }
        });
        assert!(serde_json::from_value::<CommitRequestV1>(value).is_err());
    }

    #[test]
    fn canonical_request_hash_is_stable() {
        let request = CommitRequestV1 {
            protocol_version: COMMIT_REQUEST_V1.to_owned(),
            document_id: "document-1".to_owned(),
            source_hash: "a".repeat(64),
            base_revision_id: format!("sha256:{}", "b".repeat(64)),
            client_operation_id: "operation-1".to_owned(),
            command: MoveNodeToV1 {
                kind: "move_node_to".to_owned(),
                node_id: "00112233-4455-6677-8899-aabbccddeeff".to_owned(),
                x_emu: 10,
                y_emu: 20,
            },
        };
        assert_eq!(
            request_hash(&request).unwrap(),
            request_hash(&request).unwrap()
        );
    }
}
