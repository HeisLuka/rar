use std::{fmt, str::FromStr, sync::Arc};

use pub_editor::{
    EDITOR_PROJECT_VERSION_V0_2, EDITOR_PROJECT_VERSION_V0_4, EditOperation, EditorProject,
    Sha256Digest, open_mature_0x2c_editor,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    blob_store::BlobStoreService,
    sqlite_store::{
        RevisionEdge, SqliteRevisionStore, decode_canonical_event, encode_canonical_event,
    },
};

pub const EDITOR_REVISION_EVENT_SCHEMA_V1: &str = "chaptera.editor-revision-event.v1";
pub const MATERIALIZATION_RECEIPT_SCHEMA_V1: &str =
    "chaptera.exact-revision-materialization.v1";
pub const EDITOR_REVISION_EVENT_SEMANTIC_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionMaterializerError {
    pub code: &'static str,
    pub message: String,
}

impl RevisionMaterializerError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for RevisionMaterializerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RevisionMaterializerError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizedDocumentSource {
    pub tenant_id: String,
    pub document_id: String,
    pub binding_id: String,
    pub source_sha256: String,
    pub byte_len: u64,
    pub baseline_revision_id: String,
    pub baseline_cursor: i64,
}

#[async_trait::async_trait]
pub trait DocumentSourceAuthority: Send + Sync {
    async fn resolve_document_source(
        &self,
        tenant_id: &str,
        document_id: &str,
    ) -> Result<AuthorizedDocumentSource, RevisionMaterializerError>;
}

#[async_trait::async_trait]
pub trait ExactSourceLoader: Send + Sync {
    async fn load_exact_source(
        &self,
        source: &AuthorizedDocumentSource,
    ) -> Result<Vec<u8>, RevisionMaterializerError>;
}

#[derive(Clone)]
pub struct BlobStoreExactSourceLoader {
    blob_store: BlobStoreService,
}

impl BlobStoreExactSourceLoader {
    pub fn new(blob_store: BlobStoreService) -> Self {
        Self { blob_store }
    }
}

#[async_trait::async_trait]
impl ExactSourceLoader for BlobStoreExactSourceLoader {
    async fn load_exact_source(
        &self,
        source: &AuthorizedDocumentSource,
    ) -> Result<Vec<u8>, RevisionMaterializerError> {
        let mut bytes = Vec::new();
        self.blob_store
            .stream_binding_verified(&source.tenant_id, &source.binding_id, &mut bytes)
            .await
            .map_err(|error| RevisionMaterializerError::new(error.code, error.message))?;

        if bytes.len() as u64 != source.byte_len {
            return Err(RevisionMaterializerError::new(
                "source_length_mismatch",
                "authorized source byte length differs from the verified binding bytes",
            ));
        }
        let actual_sha256 = sha256_hex(&bytes);
        if actual_sha256 != source.source_sha256 {
            return Err(RevisionMaterializerError::new(
                "source_hash_mismatch",
                "authorized source hash differs from the verified binding bytes",
            ));
        }
        Ok(bytes)
    }
}

pub trait EditorReplayEngine: Send + Sync {
    fn baseline_project(
        &self,
        source_bytes: &[u8],
        source_sha256: &str,
    ) -> Result<EditorProject, RevisionMaterializerError>;

    fn replay_project(
        &self,
        source_bytes: &[u8],
        source_sha256: &str,
        project: &EditorProject,
    ) -> Result<EditorProject, RevisionMaterializerError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PubEditorReplayEngine;

impl PubEditorReplayEngine {
    fn open(
        source_bytes: &[u8],
        source_sha256: &str,
    ) -> Result<pub_editor::EditorSession, RevisionMaterializerError> {
        let source_hash = Sha256Digest::from_str(source_sha256).map_err(|error| {
            RevisionMaterializerError::new(
                "invalid_source_hash",
                format!("authorized source hash is not canonical SHA-256: {error}"),
            )
        })?;
        open_mature_0x2c_editor(source_bytes, source_hash).map_err(|error| {
            RevisionMaterializerError::new(
                "editor_source_unsupported",
                format!("canonical editor could not open the immutable source: {error}"),
            )
        })
    }
}

impl EditorReplayEngine for PubEditorReplayEngine {
    fn baseline_project(
        &self,
        source_bytes: &[u8],
        source_sha256: &str,
    ) -> Result<EditorProject, RevisionMaterializerError> {
        let session = Self::open(source_bytes, source_sha256)?;
        let project = session.project();
        require_project_source(&project, source_sha256)?;
        if !project.assets.is_empty() {
            return Err(RevisionMaterializerError::new(
                "editor_asset_replay_unsupported",
                "baseline EditorProject unexpectedly requires external asset bytes",
            ));
        }
        Ok(project)
    }

    fn replay_project(
        &self,
        source_bytes: &[u8],
        source_sha256: &str,
        project: &EditorProject,
    ) -> Result<EditorProject, RevisionMaterializerError> {
        require_project_source(project, source_sha256)?;
        if !project.assets.is_empty() {
            return Err(RevisionMaterializerError::new(
                "editor_asset_replay_unsupported",
                "exact revision materialization does not yet resolve EditorProject asset bytes",
            ));
        }

        let mut session = Self::open(source_bytes, source_sha256)?;
        session.apply_project(project).map_err(|error| {
            RevisionMaterializerError::new(
                "editor_replay_rejected",
                format!("canonical EditorSession rejected persisted project replay: {error}"),
            )
        })?;
        let replayed = session.project();
        if replayed != *project {
            return Err(RevisionMaterializerError::new(
                "editor_replay_mismatch",
                "canonical EditorSession replay did not reproduce the exact persisted EditorProject",
            ));
        }
        Ok(replayed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorRevisionEventV1 {
    pub schema_version: String,
    pub source_sha256: String,
    pub before_project_sha256: String,
    pub after_project_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authoring_root_hash: Option<String>,
    pub operation: EditOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactRevisionMaterializationReceipt {
    pub schema_version: String,
    pub tenant_id: String,
    pub document_id: String,
    pub source_binding_id: String,
    pub source_sha256: String,
    pub baseline_revision_id: String,
    pub baseline_cursor: i64,
    pub requested_revision_id: String,
    pub replayed_edges: usize,
    pub project_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authoring_root_hash: Option<String>,
    pub project: EditorProject,
}

pub struct ExactRevisionMaterializer {
    source_authority: Arc<dyn DocumentSourceAuthority>,
    source_loader: Arc<dyn ExactSourceLoader>,
    revision_store: SqliteRevisionStore,
    editor: Arc<dyn EditorReplayEngine>,
}

impl ExactRevisionMaterializer {
    pub fn new(
        source_authority: Arc<dyn DocumentSourceAuthority>,
        source_loader: Arc<dyn ExactSourceLoader>,
        revision_store: SqliteRevisionStore,
        editor: Arc<dyn EditorReplayEngine>,
    ) -> Self {
        Self {
            source_authority,
            source_loader,
            revision_store,
            editor,
        }
    }

    pub async fn materialize(
        &self,
        tenant_id: &str,
        document_id: &str,
        requested_revision_id: &str,
    ) -> Result<ExactRevisionMaterializationReceipt, RevisionMaterializerError> {
        require_identifier(tenant_id, "tenant_id")?;
        require_identifier(document_id, "document_id")?;
        require_identifier(requested_revision_id, "requested_revision_id")?;

        let source = self
            .source_authority
            .resolve_document_source(tenant_id, document_id)
            .await?;
        validate_authorized_source(&source, tenant_id, document_id)?;

        let source_bytes = self.source_loader.load_exact_source(&source).await?;
        if source_bytes.len() as u64 != source.byte_len {
            return Err(RevisionMaterializerError::new(
                "source_length_mismatch",
                "loaded immutable source does not match the authorized byte length",
            ));
        }
        let actual_source_hash = sha256_hex(&source_bytes);
        if actual_source_hash != source.source_sha256 {
            return Err(RevisionMaterializerError::new(
                "source_hash_mismatch",
                "loaded immutable source does not match the authorized SHA-256",
            ));
        }

        let mut current_project = self
            .editor
            .baseline_project(&source_bytes, &source.source_sha256)?;
        require_project_source(&current_project, &source.source_sha256)?;
        if !current_project.assets.is_empty() {
            return Err(RevisionMaterializerError::new(
                "editor_asset_replay_unsupported",
                "baseline project contains asset metadata without a materialization asset resolver",
            ));
        }

        let edges = self
            .revision_store
            .load_chain_to_revision(
                document_id,
                &source.baseline_revision_id,
                source.baseline_cursor,
                requested_revision_id,
            )
            .await
            .map_err(|error| RevisionMaterializerError::new(error.code, error.message))?;

        let mut authoring_root_hash = None;
        for edge in &edges {
            current_project = self.replay_edge(
                &source_bytes,
                &source,
                current_project,
                edge,
            )?;
            authoring_root_hash = edge.authoring_root_hash.clone();
        }

        let project_sha256 = project_sha256(&current_project)?;
        let receipt = ExactRevisionMaterializationReceipt {
            schema_version: MATERIALIZATION_RECEIPT_SCHEMA_V1.to_owned(),
            tenant_id: tenant_id.to_owned(),
            document_id: document_id.to_owned(),
            source_binding_id: source.binding_id,
            source_sha256: source.source_sha256,
            baseline_revision_id: source.baseline_revision_id,
            baseline_cursor: source.baseline_cursor,
            requested_revision_id: requested_revision_id.to_owned(),
            replayed_edges: edges.len(),
            project_sha256,
            authoring_root_hash,
            project: current_project,
        };
        Ok(receipt)
    }

    fn replay_edge(
        &self,
        source_bytes: &[u8],
        source: &AuthorizedDocumentSource,
        current_project: EditorProject,
        edge: &RevisionEdge,
    ) -> Result<EditorProject, RevisionMaterializerError> {
        if edge.document_id != source.document_id {
            return Err(RevisionMaterializerError::new(
                "revision_document_mismatch",
                "RevisionStream edge belongs to a different document",
            ));
        }
        if edge.semantic_schema_version != EDITOR_REVISION_EVENT_SEMANTIC_SCHEMA_VERSION {
            return Err(RevisionMaterializerError::new(
                "unsupported_event_schema",
                format!(
                    "semantic schema version {} is not supported by exact materialization V1",
                    edge.semantic_schema_version
                ),
            ));
        }

        let event = decode_editor_revision_event_v1(edge)?;
        if event.source_sha256 != source.source_sha256 {
            return Err(RevisionMaterializerError::new(
                "event_source_mismatch",
                "revision event is bound to a different immutable source",
            ));
        }
        if event.authoring_root_hash != edge.authoring_root_hash {
            return Err(RevisionMaterializerError::new(
                "authoring_root_mismatch",
                "revision event authoring root does not match the durable RevisionStream edge",
            ));
        }
        if matches!(event.operation, EditOperation::ReplaceImage { .. }) {
            return Err(RevisionMaterializerError::new(
                "unsupported_event_operation",
                "ReplaceImage replay requires immutable asset-byte resolution and is fail-closed in materialization V1",
            ));
        }

        let before_hash = project_sha256(&current_project)?;
        if event.before_project_sha256 != before_hash {
            return Err(RevisionMaterializerError::new(
                "before_state_mismatch",
                "revision event before-project hash does not match the materialized predecessor state",
            ));
        }

        let candidate = append_event_operation(current_project, event.operation)?;
        let replayed = self
            .editor
            .replay_project(source_bytes, &source.source_sha256, &candidate)?;
        if replayed != candidate {
            return Err(RevisionMaterializerError::new(
                "editor_replay_mismatch",
                "canonical editor replay returned a different project than the durable event sequence",
            ));
        }

        let after_hash = project_sha256(&replayed)?;
        if event.after_project_sha256 != after_hash {
            return Err(RevisionMaterializerError::new(
                "event_state_hash_mismatch",
                "revision event after-project hash does not match canonical replay",
            ));
        }
        if edge.resulting_state_hash != after_hash {
            return Err(RevisionMaterializerError::new(
                "revision_state_hash_mismatch",
                "durable RevisionStream resulting_state_hash does not match canonical replay",
            ));
        }

        Ok(replayed)
    }
}

pub fn encode_editor_revision_event_v1(
    event: &EditorRevisionEventV1,
) -> Result<Vec<u8>, RevisionMaterializerError> {
    validate_event_fields(event)?;
    let payload = canonical_json_bytes(event, "event_encode_failed", "editor revision event")?;
    encode_canonical_event(&payload)
        .map_err(|error| RevisionMaterializerError::new(error.code, error.message))
}

pub fn decode_editor_revision_event_v1(
    edge: &RevisionEdge,
) -> Result<EditorRevisionEventV1, RevisionMaterializerError> {
    let payload = decode_canonical_event(&edge.canonical_event)
        .map_err(|error| RevisionMaterializerError::new(error.code, error.message))?;
    let event: EditorRevisionEventV1 = serde_json::from_slice(&payload).map_err(|error| {
        RevisionMaterializerError::new(
            "event_decode_failed",
            format!("canonical RevisionStream event is not supported V1 JSON: {error}"),
        )
    })?;
    validate_event_fields(&event)?;
    Ok(event)
}

/// Raw lowercase SHA-256 of the existing Rar canonical JSON project law.
pub fn project_sha256(project: &EditorProject) -> Result<String, RevisionMaterializerError> {
    let bytes = canonical_json_bytes(project, "project_encode_failed", "EditorProject")?;
    Ok(sha256_hex(&bytes))
}

fn canonical_json_bytes<T: Serialize>(
    value: &T,
    code: &'static str,
    label: &'static str,
) -> Result<Vec<u8>, RevisionMaterializerError> {
    // serde_json::Value uses its canonical sorted-key map when the optional
    // preserve_order feature is not enabled. The workspace does not enable it.
    // Serializing via Value therefore matches the existing Rar/Python
    // sort_keys=True, separators=(",", ":"), ensure_ascii=False hash law.
    let canonical = serde_json::to_value(value).map_err(|error| {
        RevisionMaterializerError::new(
            code,
            format!("could not normalize {label} into canonical JSON: {error}"),
        )
    })?;
    serde_json::to_vec(&canonical).map_err(|error| {
        RevisionMaterializerError::new(
            code,
            format!("could not serialize canonical {label}: {error}"),
        )
    })
}

fn append_event_operation(
    mut project: EditorProject,
    operation: EditOperation,
) -> Result<EditorProject, RevisionMaterializerError> {
    require_project_source(&project, &project.source_hash.to_string())?;
    if !project.assets.is_empty() {
        return Err(RevisionMaterializerError::new(
            "editor_asset_replay_unsupported",
            "EditorProject asset metadata requires a separate immutable asset resolver",
        ));
    }
    project.operations.push(operation);
    project.schema_version = if project
        .operations
        .iter()
        .any(|item| matches!(item, EditOperation::MoveNode { .. }))
    {
        EDITOR_PROJECT_VERSION_V0_4.to_owned()
    } else {
        EDITOR_PROJECT_VERSION_V0_2.to_owned()
    };
    Ok(project)
}

fn validate_event_fields(event: &EditorRevisionEventV1) -> Result<(), RevisionMaterializerError> {
    if event.schema_version != EDITOR_REVISION_EVENT_SCHEMA_V1 {
        return Err(RevisionMaterializerError::new(
            "unsupported_event_schema",
            format!("unsupported editor revision event schema {:?}", event.schema_version),
        ));
    }
    require_sha256(&event.source_sha256, "event.source_sha256")?;
    require_sha256(
        &event.before_project_sha256,
        "event.before_project_sha256",
    )?;
    require_sha256(&event.after_project_sha256, "event.after_project_sha256")?;
    if let Some(root) = &event.authoring_root_hash {
        require_sha256(root, "event.authoring_root_hash")?;
    }
    Ok(())
}

fn validate_authorized_source(
    source: &AuthorizedDocumentSource,
    tenant_id: &str,
    document_id: &str,
) -> Result<(), RevisionMaterializerError> {
    if source.tenant_id != tenant_id {
        return Err(RevisionMaterializerError::new(
            "source_tenant_mismatch",
            "source authority returned a binding for a different tenant",
        ));
    }
    if source.document_id != document_id {
        return Err(RevisionMaterializerError::new(
            "source_document_mismatch",
            "source authority returned a binding for a different document",
        ));
    }
    require_identifier(&source.binding_id, "binding_id")?;
    require_identifier(&source.baseline_revision_id, "baseline_revision_id")?;
    require_sha256(&source.source_sha256, "source_sha256")?;
    if source.byte_len == 0 {
        return Err(RevisionMaterializerError::new(
            "source_length_invalid",
            "authorized immutable source length must be positive",
        ));
    }
    if source.baseline_cursor < 0 {
        return Err(RevisionMaterializerError::new(
            "invalid_baseline_cursor",
            "authorized baseline cursor must be non-negative",
        ));
    }
    Ok(())
}

fn require_project_source(
    project: &EditorProject,
    source_sha256: &str,
) -> Result<(), RevisionMaterializerError> {
    if project.source_hash.to_string() != source_sha256 {
        return Err(RevisionMaterializerError::new(
            "project_source_mismatch",
            "EditorProject source identity differs from the authorized immutable source",
        ));
    }
    Ok(())
}

fn require_identifier(value: &str, field: &'static str) -> Result<(), RevisionMaterializerError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(RevisionMaterializerError::new(
            "invalid_identifier",
            format!("{field} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_sha256(value: &str, field: &'static str) -> Result<(), RevisionMaterializerError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(RevisionMaterializerError::new(
            "invalid_hash",
            format!("{field} must be 64 lowercase SHA-256 hex characters"),
        ));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("writing SHA-256 hex into String cannot fail");
    }
    out
}
