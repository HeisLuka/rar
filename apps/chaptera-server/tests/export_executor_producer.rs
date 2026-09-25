use std::{fs, path::PathBuf, str::FromStr, sync::Arc};

use async_trait::async_trait;
use chaptera_server::{
    export_executor::{
        EXPORT_JOB_PAYLOAD_SCHEMA_V1, ExactRevisionEditableExporter, ExactRevisionStateProvider,
        ExportExecutorError, ExportJobPayloadV1, IDML_BOUNDED_EDITABLE_PROFILE,
    },
    revision_materializer::{
        ExactRevisionMaterializationReceipt, ExactRevisionMaterializedState,
        MATERIALIZATION_RECEIPT_SCHEMA_V1,
    },
};
use pub_editor::{Sha256Digest, open_mature_0x2c_editor};
use sha2::{Digest, Sha256};

const CANONICAL_SCHEMA: &str = "chaptera.cdm.authoring-revision.v1";

#[derive(Clone)]
struct FixtureStateProvider {
    state: ExactRevisionMaterializedState,
}

#[async_trait]
impl ExactRevisionStateProvider for FixtureStateProvider {
    async fn materialize_state(
        &self,
        tenant_id: &str,
        document_id: &str,
        exact_revision_id: &str,
    ) -> Result<ExactRevisionMaterializedState, ExportExecutorError> {
        if tenant_id != self.state.receipt.tenant_id
            || document_id != self.state.receipt.document_id
            || exact_revision_id != self.state.receipt.requested_revision_id
        {
            return Err(ExportExecutorError::new(
                "fixture_identity_mismatch",
                "test provider received a different exact revision identity",
            ));
        }
        Ok(self.state.clone())
    }
}

fn fixture_path() -> PathBuf {
    std::env::var_os("CHAPTERA_EXPORT_FIXTURE")
        .map(PathBuf::from)
        .expect("CHAPTERA_EXPORT_FIXTURE must point to the pinned real PUB fixture")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[tokio::test]
#[ignore = "requires pinned real PUB fixture from CLOUD-EXPORT-EXECUTOR-01 acceptance"]
async fn exact_edited_real_pub_produces_deterministic_idml_and_loss_report() {
    let source_bytes = fs::read(fixture_path()).expect("read pinned SampleNewsletter.pub");
    let source_sha256 = sha256_hex(&source_bytes);
    assert_eq!(
        source_sha256,
        "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
    );

    let source_hash = Sha256Digest::from_str(&source_sha256).expect("fixture SHA-256");
    let mut session =
        open_mature_0x2c_editor(&source_bytes, source_hash).expect("open canonical real PUB");

    let story_id = session
        .graph()
        .stories
        .keys()
        .copied()
        .find(|story_id| session.can_replace_story_text(*story_id).is_ok())
        .expect("SampleNewsletter must expose one bounded editable story");
    let before = session
        .graph()
        .stories
        .get(&story_id)
        .expect("chosen story exists")
        .text
        .clone();
    session
        .replace_story_text(story_id, format!("{before} Chaptera export proof"))
        .expect("apply real story edit");
    let project = session.project();
    assert!(
        !project.operations.is_empty(),
        "fixture project must contain a real edit"
    );

    let project_json = serde_json::to_vec(&project).expect("serialize fixture project");
    let project_sha256 = sha256_hex(&project_json);
    let canonical_revision = "c".repeat(64);
    let service_revision = "service-rev-1".to_owned();

    let state = ExactRevisionMaterializedState {
        receipt: ExactRevisionMaterializationReceipt {
            schema_version: MATERIALIZATION_RECEIPT_SCHEMA_V1.to_owned(),
            tenant_id: "tenant:fixture".to_owned(),
            document_id: "document:fixture".to_owned(),
            source_binding_id: "binding:fixture".to_owned(),
            source_sha256: source_sha256.clone(),
            baseline_revision_id: "service-rev-0".to_owned(),
            baseline_cursor: 0,
            requested_revision_id: service_revision.clone(),
            canonical_revision_schema_version: CANONICAL_SCHEMA.to_owned(),
            canonical_authoring_revision_id: canonical_revision.clone(),
            replayed_edges: 1,
            project_sha256,
            authoring_root_hash: None,
            project,
        },
        source_bytes,
    };

    let exporter = ExactRevisionEditableExporter::new(Arc::new(FixtureStateProvider { state }));
    let payload = ExportJobPayloadV1 {
        schema_version: EXPORT_JOB_PAYLOAD_SCHEMA_V1.to_owned(),
        tenant_id: "tenant:fixture".to_owned(),
        document_id: "document:fixture".to_owned(),
        exact_revision_id: service_revision,
        canonical_authoring_revision_id: canonical_revision,
        target_profile: IDML_BOUNDED_EDITABLE_PROFILE.to_owned(),
        layout_environment_id: format!("sha256:{}", "d".repeat(64)),
    };

    let first = exporter
        .produce(&payload)
        .await
        .expect("produce exact edited IDML");
    let second = exporter
        .produce(&payload)
        .await
        .expect("exact retry produces the same edited IDML");

    assert!(!first.artifact_bytes.is_empty());
    assert!(
        first.artifact_bytes.starts_with(b"PK"),
        "IDML must be a ZIP package"
    );
    assert!(!first.loss_report_json.is_empty());
    let loss_json: serde_json::Value =
        serde_json::from_slice(&first.loss_report_json).expect("canonical LossReport JSON");
    assert!(loss_json.is_object());

    assert_eq!(first.artifact_bytes, second.artifact_bytes);
    assert_eq!(first.artifact_sha256, second.artifact_sha256);
    assert_eq!(first.loss_report_json, second.loss_report_json);
    assert_eq!(first.loss_report_sha256, second.loss_report_sha256);
    assert_eq!(first.exact_revision_id, payload.exact_revision_id);
    assert_eq!(
        first.canonical_authoring_revision_id,
        payload.canonical_authoring_revision_id
    );
    assert_eq!(first.layout_environment_id, payload.layout_environment_id);
}
