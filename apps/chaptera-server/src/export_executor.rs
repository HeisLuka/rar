use std::{fmt, str::FromStr, sync::Arc};

use async_trait::async_trait;
use pub_editor::{EditorEditableTarget, Sha256Digest, open_mature_0x2c_editor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::revision_materializer::{
    ExactRevisionMaterializedState, ExactRevisionMaterializer, RevisionMaterializerError,
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
    fn payload_rejects_reference_pdf_profile_until_edited_pdf_is_proven() {
        let payload = ExportJobPayloadV1 {
            schema_version: EXPORT_JOB_PAYLOAD_SCHEMA_V1.into(),
            tenant_id: "tenant:1".into(),
            document_id: "doc:1".into(),
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
                exact_revision_id: format!("sha256:{}", "a".repeat(64)),
                canonical_authoring_revision_id: "c".repeat(64),
                target_profile: profile.into(),
                layout_environment_id: format!("sha256:{}", "b".repeat(64)),
            };
            payload.validate().unwrap();
        }
    }
}
