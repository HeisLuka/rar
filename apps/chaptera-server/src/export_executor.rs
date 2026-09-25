use std::{
    fmt,
    future::Future,
    pin::Pin,
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use pub_editor::{EditorEditableTarget, Sha256Digest, open_mature_0x2c_editor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    blob_store::{BlobStoreService, CreateBindingRequest, ResourceKind},
    derived_artifacts::{DerivedArtifactFenceV1, SqliteDerivedArtifactStore},
    export_publication::ExportPublicationInputV1,
    job_queue::{JobKind, JobRecord},
    job_worker::{CancellationFlag, JobExecutor, JobFailure, JobFuture, JobSuccess},
    revision_materializer::{
        ExactRevisionMaterializedState, ExactRevisionMaterializer, RevisionMaterializerError,
    },
};

pub const EXPORT_JOB_PAYLOAD_SCHEMA_V1: &str = "chaptera.export-job-payload.v1";
pub const IDML_BOUNDED_EDITABLE_PROFILE: &str = "idml:bounded-editable";
pub const ODG_BOUNDED_EDITABLE_PROFILE: &str = "odg:bounded-editable";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportExecutorError {
    pub code: &'static str,
    pub message: String,
}

impl ExportExecutorError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ExportExecutorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ExportExecutorError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportJobPayloadV1 {
    pub schema_version: String,
    pub tenant_id: String,
    pub document_id: String,
    pub requesting_principal_id: String,
    pub exact_revision_id: String,
    pub canonical_authoring_revision_id: String,
    pub target_profile: String,
    pub layout_environment_id: String,
}

impl ExportJobPayloadV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, ExportExecutorError> {
        let payload: Self = serde_json::from_slice(bytes).map_err(|error| {
            ExportExecutorError::new(
                "export_payload_invalid",
                format!("export job payload is not valid V1 JSON: {error}"),
            )
        })?;
        payload.validate()?;
        Ok(payload)
    }

    fn validate(&self) -> Result<(), ExportExecutorError> {
        if self.schema_version != EXPORT_JOB_PAYLOAD_SCHEMA_V1 {
            return Err(ExportExecutorError::new(
                "export_payload_schema_unsupported",
                "export job payload schema is unsupported",
            ));
        }
        for (label, value) in [
            ("tenant_id", self.tenant_id.as_str()),
            ("document_id", self.document_id.as_str()),
            (
                "requesting_principal_id",
                self.requesting_principal_id.as_str(),
            ),
            ("exact_revision_id", self.exact_revision_id.as_str()),
        ] {
            require_ident(value, label)?;
        }
        require_hex_sha256(
            &self.canonical_authoring_revision_id,
            "canonical_authoring_revision_id",
        )?;
        require_prefixed_sha256(&self.layout_environment_id, "layout_environment_id")?;
        target_from_profile(&self.target_profile)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedEditableExport {
    pub target_profile: String,
    pub exact_revision_id: String,
    pub canonical_authoring_revision_id: String,
    pub layout_environment_id: String,
    pub source_sha256: String,
    pub project_sha256: String,
    pub artifact_bytes: Vec<u8>,
    pub artifact_sha256: String,
    pub loss_report_json: Vec<u8>,
    pub loss_report_sha256: String,
    pub loss_report_text: String,
}

#[async_trait]
pub trait ExactRevisionStateProvider: Send + Sync {
    async fn materialize_state(
        &self,
        tenant_id: &str,
        document_id: &str,
        exact_revision_id: &str,
    ) -> Result<ExactRevisionMaterializedState, ExportExecutorError>;
}

#[async_trait]
impl ExactRevisionStateProvider for ExactRevisionMaterializer {
    async fn materialize_state(
        &self,
        tenant_id: &str,
        document_id: &str,
        exact_revision_id: &str,
    ) -> Result<ExactRevisionMaterializedState, ExportExecutorError> {
        ExactRevisionMaterializer::materialize_state(
            self,
            tenant_id,
            document_id,
            exact_revision_id,
        )
        .await
        .map_err(materializer_error)
    }
}

pub struct ExactRevisionEditableExporter {
    materializer: Arc<dyn ExactRevisionStateProvider>,
}

impl ExactRevisionEditableExporter {
    pub fn new(materializer: Arc<dyn ExactRevisionStateProvider>) -> Self {
        Self { materializer }
    }

    pub async fn produce(
        &self,
        payload: &ExportJobPayloadV1,
    ) -> Result<ProducedEditableExport, ExportExecutorError> {
        payload.validate()?;
        let state = self
            .materializer
            .materialize_state(
                &payload.tenant_id,
                &payload.document_id,
                &payload.exact_revision_id,
            )
            .await?;

        if state.receipt.requested_revision_id != payload.exact_revision_id
            || state.receipt.tenant_id != payload.tenant_id
            || state.receipt.document_id != payload.document_id
            || state.receipt.canonical_authoring_revision_id
                != payload.canonical_authoring_revision_id
        {
            return Err(ExportExecutorError::new(
                "materialized_identity_mismatch",
                "exact materializer returned a different tenant/document/service-or-canonical revision identity",
            ));
        }

        let source_hash =
            Sha256Digest::from_str(&state.receipt.source_sha256).map_err(|error| {
                ExportExecutorError::new(
                    "materialized_source_hash_invalid",
                    format!("materialized source hash is invalid: {error}"),
                )
            })?;
        let mut session =
            open_mature_0x2c_editor(&state.source_bytes, source_hash).map_err(|error| {
                ExportExecutorError::new(
                    "editor_source_unsupported",
                    format!("canonical editor could not open exact source bytes: {error}"),
                )
            })?;
        session
            .apply_project(&state.receipt.project)
            .map_err(|error| {
                ExportExecutorError::new(
                    "editor_project_replay_rejected",
                    format!("canonical editor rejected exact materialized project: {error}"),
                )
            })?;
        if session.project() != state.receipt.project {
            return Err(ExportExecutorError::new(
                "editor_project_replay_mismatch",
                "canonical editor replay did not reproduce exact materialized project",
            ));
        }

        let target = target_from_profile(&payload.target_profile)?;
        let export = session
            .export_editable(
                target,
                format!("{}@{}", payload.document_id, payload.exact_revision_id),
            )
            .map_err(|error| {
                ExportExecutorError::new(
                    "editable_export_failed",
                    format!("canonical editable exporter rejected exact revision: {error}"),
                )
            })?;

        let loss_report_json = serde_json::to_vec(&export.report).map_err(|error| {
            ExportExecutorError::new(
                "loss_report_serialize_failed",
                format!("canonical LossReport could not be serialized: {error}"),
            )
        })?;

        Ok(ProducedEditableExport {
            target_profile: payload.target_profile.clone(),
            exact_revision_id: payload.exact_revision_id.clone(),
            canonical_authoring_revision_id: payload.canonical_authoring_revision_id.clone(),
            layout_environment_id: payload.layout_environment_id.clone(),
            source_sha256: state.receipt.source_sha256,
            project_sha256: state.receipt.project_sha256,
            artifact_sha256: sha256_prefixed(&export.bytes),
            loss_report_sha256: sha256_prefixed(&loss_report_json),
            artifact_bytes: export.bytes,
            loss_report_json,
            loss_report_text: export.human_summary,
        })
    }
}

pub type ExportPublishAuthFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), ExportExecutorError>> + Send + 'a>>;

pub trait ExportPublishAuthorizer: Send + Sync {
    /// Early fail-closed preflight. This is not the final revoke barrier.
    fn authorize<'a>(
        &'a self,
        job: &'a JobRecord,
        payload: &'a ExportJobPayloadV1,
    ) -> ExportPublishAuthFuture<'a>;
}

pub type ExportPublicationCommitFuture<'a> =
    Pin<Box<dyn Future<Output = Result<String, ExportExecutorError>> + Send + 'a>>;

pub trait ExportPublicationCommitter: Send + Sync {
    /// Final publication authority.
    ///
    /// A production implementation must verify current document.export
    /// authorization for payload.requesting_principal_id and prepare the
    /// durable logical publication under the same access-generation
    /// barrier/fence. A separate authorize-then-write sequence is not
    /// sufficient for live-revocation correctness.
    fn commit_authorized<'a>(
        &'a self,
        job: &'a JobRecord,
        payload: &'a ExportJobPayloadV1,
        input: ExportPublicationInputV1,
        created_at_ms: i64,
    ) -> ExportPublicationCommitFuture<'a>;
}

pub struct PublishedExportJobExecutor {
    producer: Arc<ExactRevisionEditableExporter>,
    blob_store: BlobStoreService,
    artifacts: SqliteDerivedArtifactStore,
    publication_committer: Arc<dyn ExportPublicationCommitter>,
    authorizer: Arc<dyn ExportPublishAuthorizer>,
}

impl PublishedExportJobExecutor {
    pub fn new(
        producer: Arc<ExactRevisionEditableExporter>,
        blob_store: BlobStoreService,
        artifacts: SqliteDerivedArtifactStore,
        publication_committer: Arc<dyn ExportPublicationCommitter>,
        authorizer: Arc<dyn ExportPublishAuthorizer>,
    ) -> Self {
        Self {
            producer,
            blob_store,
            artifacts,
            publication_committer,
            authorizer,
        }
    }

    async fn execute_export(
        &self,
        job: &JobRecord,
        cancellation: &CancellationFlag,
    ) -> Result<JobSuccess, ExportExecutorError> {
        if job.job_kind != JobKind::Export {
            return Err(ExportExecutorError::new(
                "export_job_kind_mismatch",
                "published export executor received a non-export job",
            ));
        }
        let payload = ExportJobPayloadV1::decode(&job.payload)?;
        if payload.tenant_id != job.tenant_id {
            return Err(ExportExecutorError::new(
                "export_job_tenant_mismatch",
                "job queue tenant differs from typed export payload tenant",
            ));
        }
        if cancellation.is_cancelled() {
            return Err(ExportExecutorError::new(
                "export_cancelled",
                "export was cancelled before producer execution",
            ));
        }

        self.authorizer.authorize(job, &payload).await?;
        let produced = self.producer.produce(&payload).await?;

        if cancellation.is_cancelled() {
            return Err(ExportExecutorError::new(
                "export_cancelled",
                "export was cancelled before immutable artifact preparation",
            ));
        }

        let now_ms = unix_now_ms()?;
        let artifact_binding = self
            .store_export_resource(
                &payload,
                produced.artifact_sha256.clone(),
                produced.artifact_bytes.as_slice(),
                target_mime(&payload.target_profile)?,
                payload.target_profile.clone(),
                now_ms,
            )
            .await?;
        let loss_binding = self
            .store_export_resource(
                &payload,
                produced.loss_report_sha256.clone(),
                produced.loss_report_json.as_slice(),
                "application/json".to_owned(),
                "chaptera.loss-report.v1".to_owned(),
                now_ms,
            )
            .await?;

        let fence = DerivedArtifactFenceV1 {
            document_id: payload.document_id.clone(),
            service_revision_id: payload.exact_revision_id.clone(),
            canonical_revision_id: payload.canonical_authoring_revision_id.clone(),
            stage: "export".to_owned(),
            stage_version: payload.target_profile.clone(),
            environment_fingerprint: payload.layout_environment_id.clone(),
            input_fingerprint: format!("sha256:{}", produced.project_sha256),
        };
        let fence_id = fence
            .fence_id()
            .map_err(|error| ExportExecutorError::new(error.code, error.message))?;
        self.artifacts
            .publish(
                fence,
                produced.artifact_sha256.clone(),
                i64::try_from(now_ms).map_err(|_| {
                    ExportExecutorError::new("clock_overflow", "publication time does not fit i64")
                })?,
            )
            .await
            .map_err(|error| ExportExecutorError::new(error.code, error.message))?;

        if cancellation.is_cancelled() {
            return Err(ExportExecutorError::new(
                "export_cancelled",
                "export was cancelled before logical publication preparation",
            ));
        }

        let publication_input = ExportPublicationInputV1 {
            tenant_id: payload.tenant_id.clone(),
            job_id: job.job_id.clone(),
            document_id: payload.document_id.clone(),
            exact_revision_id: payload.exact_revision_id.clone(),
            canonical_revision_id: payload.canonical_authoring_revision_id.clone(),
            target_profile: payload.target_profile.clone(),
            layout_environment_id: payload.layout_environment_id.clone(),
            fence_id,
            artifact_binding_id: artifact_binding.binding_id,
            artifact_content_hash: produced.artifact_sha256,
            loss_binding_id: loss_binding.binding_id,
            loss_report_hash: produced.loss_report_sha256,
        };
        let effect_key = self
            .publication_committer
            .commit_authorized(
                job,
                &payload,
                publication_input,
                i64::try_from(now_ms).map_err(|_| {
                    ExportExecutorError::new("clock_overflow", "publication time does not fit i64")
                })?,
            )
            .await?;

        Ok(JobSuccess { effect_key })
    }

    async fn store_export_resource(
        &self,
        payload: &ExportJobPayloadV1,
        content_sha256: String,
        bytes: &[u8],
        canonical_mime: String,
        validation_profile: String,
        now_ms: u64,
    ) -> Result<crate::blob_store::ResourceBinding, ExportExecutorError> {
        let byte_len = u64::try_from(bytes.len()).map_err(|_| {
            ExportExecutorError::new(
                "export_artifact_too_large",
                "export resource length does not fit u64",
            )
        })?;
        let storage_content_sha256 = content_sha256
            .strip_prefix("sha256:")
            .ok_or_else(|| {
                ExportExecutorError::new(
                    "export_artifact_hash_invalid",
                    "export resource fingerprint must use sha256:<64 lowercase hex>",
                )
            })?
            .to_owned();
        let mut input = bytes;
        self.blob_store
            .create_canonical_binding(
                CreateBindingRequest {
                    tenant_id: payload.tenant_id.clone(),
                    project_id: None,
                    document_id: Some(payload.document_id.clone()),
                    content_sha256: storage_content_sha256,
                    byte_len,
                    canonical_mime: Some(canonical_mime),
                    resource_kind: ResourceKind::ExportArtifact,
                    validation_profile,
                    now_ms,
                },
                &mut input,
            )
            .await
            .map_err(|error| ExportExecutorError::new(error.code, error.message))
    }
}

impl JobExecutor for PublishedExportJobExecutor {
    fn execute<'a>(&'a self, job: &'a JobRecord, cancellation: CancellationFlag) -> JobFuture<'a> {
        Box::pin(async move {
            self.execute_export(job, &cancellation)
                .await
                .map_err(job_failure)
        })
    }
}

fn job_failure(error: ExportExecutorError) -> JobFailure {
    let (retryable, terminal_code) = match error.code {
        "provider_unavailable"
        | "provider_unknown_unreconciled"
        | "sqlite_blob_metadata_error"
        | "sqlite_artifact_error"
        | "sqlite_export_publication_error"
        | "sqlite_authz_error" => (true, "export_transient_failure"),
        "export_cancelled" => (false, "export_cancelled"),
        "export_publication_conflict" => (false, "export_publication_conflict"),
        "artifact_fence_nondeterministic" => (false, "export_artifact_nondeterministic"),
        "export_publish_unauthorized" => (false, "export_publish_unauthorized"),
        _ => (false, "export_execution_rejected"),
    };
    JobFailure {
        retryable,
        terminal_code,
    }
}

fn target_mime(profile: &str) -> Result<String, ExportExecutorError> {
    match profile {
        IDML_BOUNDED_EDITABLE_PROFILE => {
            Ok("application/vnd.adobe.indesign-idml-package".to_owned())
        }
        ODG_BOUNDED_EDITABLE_PROFILE => {
            Ok("application/vnd.oasis.opendocument.graphics".to_owned())
        }
        _ => Err(ExportExecutorError::new(
            "export_target_profile_unsupported",
            "export target has no canonical MIME mapping",
        )),
    }
}

fn unix_now_ms() -> Result<u64, ExportExecutorError> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        ExportExecutorError::new("clock_before_epoch", "system clock is before UNIX epoch")
    })?;
    u64::try_from(elapsed.as_millis()).map_err(|_| {
        ExportExecutorError::new(
            "clock_overflow",
            "system clock does not fit u64 milliseconds",
        )
    })
}

fn target_from_profile(profile: &str) -> Result<EditorEditableTarget, ExportExecutorError> {
    match profile {
        IDML_BOUNDED_EDITABLE_PROFILE => Ok(EditorEditableTarget::Idml),
        ODG_BOUNDED_EDITABLE_PROFILE => Ok(EditorEditableTarget::Odg),
        _ => Err(ExportExecutorError::new(
            "export_target_profile_unsupported",
            format!("unsupported edited-state export target profile {profile:?}"),
        )),
    }
}

fn materializer_error(error: RevisionMaterializerError) -> ExportExecutorError {
    ExportExecutorError::new(error.code, error.message)
}

fn require_ident(value: &str, label: &'static str) -> Result<(), ExportExecutorError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(ExportExecutorError::new(
            "export_payload_invalid_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_hex_sha256(value: &str, label: &'static str) -> Result<(), ExportExecutorError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ExportExecutorError::new(
            "export_payload_invalid_hash",
            format!("{label} must be 64 lowercase hex characters"),
        ));
    }
    Ok(())
}

fn require_prefixed_sha256(value: &str, label: &'static str) -> Result<(), ExportExecutorError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(ExportExecutorError::new(
            "export_payload_invalid_hash",
            format!("{label} must use sha256:<64 lowercase hex>"),
        ));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ExportExecutorError::new(
            "export_payload_invalid_hash",
            format!("{label} must use sha256:<64 lowercase hex>"),
        ));
    }
    Ok(())
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_authz_storage_failure_is_retryable_internal_not_denial() {
        let failure = job_failure(ExportExecutorError::new(
            "sqlite_authz_error",
            "synthetic storage fault",
        ));
        assert!(failure.retryable);
        assert_eq!(failure.terminal_code, "export_transient_failure");

        let denial = job_failure(ExportExecutorError::new(
            "export_publish_unauthorized",
            "synthetic denial",
        ));
        assert!(!denial.retryable);
        assert_eq!(denial.terminal_code, "export_publish_unauthorized");
    }

    #[test]
    fn payload_rejects_reference_pdf_profile_until_edited_pdf_is_proven() {
        let payload = ExportJobPayloadV1 {
            schema_version: EXPORT_JOB_PAYLOAD_SCHEMA_V1.into(),
            tenant_id: "tenant:1".into(),
            document_id: "doc:1".into(),
            requesting_principal_id: "principal:1".into(),
            exact_revision_id: format!("sha256:{}", "a".repeat(64)),
            canonical_authoring_revision_id: "c".repeat(64),
            target_profile: "pdf:v1".into(),
            layout_environment_id: format!("sha256:{}", "b".repeat(64)),
        };
        let error = payload.validate().unwrap_err();
        assert_eq!(error.code, "export_target_profile_unsupported");
    }

    #[test]
    fn payload_rejects_missing_or_noncanonical_authoring_revision_identity() {
        let payload = ExportJobPayloadV1 {
            schema_version: EXPORT_JOB_PAYLOAD_SCHEMA_V1.into(),
            tenant_id: "tenant:1".into(),
            document_id: "doc:1".into(),
            requesting_principal_id: "principal:1".into(),
            exact_revision_id: "service-rev:1".into(),
            canonical_authoring_revision_id: "not-a-canonical-authoring-revision".into(),
            target_profile: IDML_BOUNDED_EDITABLE_PROFILE.into(),
            layout_environment_id: format!("sha256:{}", "b".repeat(64)),
        };
        let error = payload.validate().unwrap_err();
        assert_eq!(error.code, "export_payload_invalid_hash");
    }

    #[test]
    fn payload_accepts_proven_editable_profiles() {
        for profile in [IDML_BOUNDED_EDITABLE_PROFILE, ODG_BOUNDED_EDITABLE_PROFILE] {
            let payload = ExportJobPayloadV1 {
                schema_version: EXPORT_JOB_PAYLOAD_SCHEMA_V1.into(),
                tenant_id: "tenant:1".into(),
                document_id: "doc:1".into(),
                requesting_principal_id: "principal:1".into(),
                exact_revision_id: format!("sha256:{}", "a".repeat(64)),
                canonical_authoring_revision_id: "c".repeat(64),
                target_profile: profile.into(),
                layout_environment_id: format!("sha256:{}", "b".repeat(64)),
            };
            payload.validate().unwrap();
        }
    }
}
