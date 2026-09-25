use std::{
    env, fmt, fs as stdfs,
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use chaptera_cdm_model::{
    AUTHORING_REVISION_SCHEMA_V1, canonical_revision_json_v1, derive_authoring_revision_id_v1,
};
use chaptera_untrusted_pub_scan::install_post_read_filesystem_default_deny;
use pub_editor::EditorProject;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{fs, io::AsyncWriteExt, process::Command, time::timeout};

use crate::{
    blob_store::BlobStoreService,
    project_persistence_sqlite::ProjectBaselineIdentity,
    revision_materializer::{EditorReplayEngine, PubEditorReplayEngine},
};

pub const SOURCE_BASELINE_RECEIPT_V1: &str = "chaptera.source-baseline-identity.v1";
const RECEIPT_MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBaselineError {
    pub code: &'static str,
    pub message: String,
}

impl SourceBaselineError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for SourceBaselineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SourceBaselineError {}

#[derive(Debug, Clone)]
pub struct SourceBaselineProducerConfig {
    pub isolation_python: PathBuf,
    pub isolation_harness: PathBuf,
    pub worker_binary: PathBuf,
    pub worker_wall_timeout: Duration,
    pub worker_address_space_mb: u64,
    pub worker_cpu_seconds: u64,
    pub worker_open_files: u64,
    pub worker_output_file_mb: u64,
    pub temp_root: PathBuf,
}

impl SourceBaselineProducerConfig {
    pub fn validate(&self) -> Result<(), SourceBaselineError> {
        if self.worker_wall_timeout.is_zero() {
            return Err(SourceBaselineError::new(
                "source_baseline_config_invalid",
                "worker wall timeout must be positive",
            ));
        }
        if self.worker_address_space_mb < 64
            || self.worker_cpu_seconds == 0
            || self.worker_open_files < 16
            || self.worker_output_file_mb == 0
        {
            return Err(SourceBaselineError::new(
                "source_baseline_config_invalid",
                "worker resource limits are outside the admitted range",
            ));
        }
        for (path, label) in [
            (&self.isolation_python, "isolation_python"),
            (&self.isolation_harness, "isolation_harness"),
            (&self.worker_binary, "worker_binary"),
            (&self.temp_root, "temp_root"),
        ] {
            if path.as_os_str().is_empty() {
                return Err(SourceBaselineError::new(
                    "source_baseline_config_invalid",
                    format!("{label} must be configured"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBaselineReceiptV1 {
    pub protocol_version: String,
    pub document_id: String,
    pub source_sha256: String,
    pub source_byte_len: u64,
    pub project_schema_version: String,
    pub project_hash: String,
    pub state_id: String,
    pub service_revision_id: String,
    pub canonical_schema_version: String,
    pub canonical_authoring_revision_id: String,
    pub filesystem_confinement: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineIdentityDerivation {
    pub project_hash: String,
    pub state_id: String,
    pub service_revision_id: String,
    pub canonical_authoring_revision_id: String,
}

#[derive(Serialize)]
struct WebAuthoringStateV1<'a> {
    protocol_version: &'static str,
    document_id: &'a str,
    source_hash: &'a str,
    project_schema_version: &'a str,
    project_hash: &'a str,
}

#[derive(Serialize)]
struct WebRevisionNodeV1<'a> {
    protocol_version: &'static str,
    document_id: &'a str,
    source_hash: &'a str,
    parent_revision_id: Option<&'a str>,
    state_id: &'a str,
    transition_kind: &'static str,
    transition_hash: Option<&'a str>,
}

pub fn derive_import_baseline_identities<G: Serialize + ?Sized>(
    document_id: &str,
    source_sha256: &str,
    project_schema_version: &str,
    project: &G,
) -> Result<BaselineIdentityDerivation, SourceBaselineError> {
    require_ident(document_id, "document_id")?;
    require_sha256(source_sha256, "source_sha256")?;
    require_ident(project_schema_version, "project_schema_version")?;

    let project_hash = web_hash_id(project)?;
    let state_id = web_hash_id(&WebAuthoringStateV1 {
        protocol_version: "chaptera.authoring-state.v1",
        document_id,
        source_hash: source_sha256,
        project_schema_version,
        project_hash: &project_hash,
    })?;
    let service_revision_id = web_hash_id(&WebRevisionNodeV1 {
        protocol_version: "chaptera.revision-node.v1",
        document_id,
        source_hash: source_sha256,
        parent_revision_id: None,
        state_id: &state_id,
        transition_kind: "baseline",
        transition_hash: None,
    })?;
    let canonical_authoring_revision_id = derive_authoring_revision_id_v1(project, None)
        .map_err(|_| {
            SourceBaselineError::new(
                "source_baseline_authoring_identity_failed",
                "canonical AuthoringRevisionId derivation failed",
            )
        })?
        .to_string();

    Ok(BaselineIdentityDerivation {
        project_hash,
        state_id,
        service_revision_id,
        canonical_authoring_revision_id,
    })
}

fn web_hash_id<T: Serialize + ?Sized>(value: &T) -> Result<String, SourceBaselineError> {
    // The public Web RevisionKernel law uses sorted-key compact UTF-8 JSON.
    // REVISION-MODEL-01's public canonicalizer implements that exact JSON
    // profile; only its domain-separated AuthoringRevisionId envelope differs.
    let canonical = canonical_revision_json_v1(value).map_err(|_| {
        SourceBaselineError::new(
            "source_baseline_web_identity_failed",
            "canonical Web revision serialization failed",
        )
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(canonical)))
}

#[derive(Clone)]
pub struct IsolatedSourceBaselineProducer {
    config: SourceBaselineProducerConfig,
    blob_store: BlobStoreService,
}

impl IsolatedSourceBaselineProducer {
    pub fn new(
        config: SourceBaselineProducerConfig,
        blob_store: BlobStoreService,
    ) -> Result<Self, SourceBaselineError> {
        config.validate()?;
        Ok(Self { config, blob_store })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn produce(
        &self,
        tenant_id: &str,
        binding_id: &str,
        expected_source_sha256: &str,
        expected_source_byte_len: u64,
        document_id: &str,
    ) -> Result<ProjectBaselineIdentity, SourceBaselineError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(binding_id, "binding_id")?;
        require_sha256(expected_source_sha256, "expected_source_sha256")?;
        require_ident(document_id, "document_id")?;
        if expected_source_byte_len == 0 {
            return Err(SourceBaselineError::new(
                "source_baseline_length_invalid",
                "expected source byte length must be positive",
            ));
        }

        let temp = BaselineTempDir::create(&self.config.temp_root).await?;
        let input_path = temp.path().join("source.pub");
        let output_dir = temp.path().join("worker-result");

        let mut input = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&input_path)
            .await
            .map_err(|_| {
                SourceBaselineError::new(
                    "source_baseline_temp_failed",
                    "could not create private baseline input",
                )
            })?;

        let copied = self
            .blob_store
            .stream_binding_verified(tenant_id, binding_id, &mut input)
            .await
            .map_err(|error| SourceBaselineError::new(error.code, error.message))?;
        input.flush().await.map_err(|_| {
            SourceBaselineError::new(
                "source_baseline_temp_failed",
                "could not flush private baseline input",
            )
        })?;
        input.sync_all().await.map_err(|_| {
            SourceBaselineError::new(
                "source_baseline_temp_failed",
                "could not sync private baseline input",
            )
        })?;
        drop(input);

        if copied != expected_source_byte_len {
            return Err(SourceBaselineError::new(
                "source_baseline_length_mismatch",
                "verified durable binding byte length differs from upload authority",
            ));
        }

        let timeout_seconds = self.config.worker_wall_timeout.as_secs_f64().to_string();
        let child = Command::new(&self.config.isolation_python)
            .arg(&self.config.isolation_harness)
            .arg("run")
            .arg("--output-dir")
            .arg(&output_dir)
            .arg("--input")
            .arg(&input_path)
            .arg("--timeout")
            .arg(timeout_seconds)
            .arg("--address-space-mb")
            .arg(self.config.worker_address_space_mb.to_string())
            .arg("--cpu-seconds")
            .arg(self.config.worker_cpu_seconds.to_string())
            .arg("--open-files")
            .arg(self.config.worker_open_files.to_string())
            .arg("--output-file-mb")
            .arg(self.config.worker_output_file_mb.to_string())
            .arg("--clear-environment")
            .arg("--")
            .arg(&self.config.worker_binary)
            .arg("source-baseline")
            .arg("--document-id")
            .arg(document_id)
            .arg("--expected-sha256")
            .arg(expected_source_sha256)
            .arg("--expected-byte-len")
            .arg(expected_source_byte_len.to_string())
            .kill_on_drop(true)
            .output();

        let process_timeout = self
            .config
            .worker_wall_timeout
            .checked_add(Duration::from_secs(5))
            .ok_or_else(|| {
                SourceBaselineError::new(
                    "source_baseline_config_invalid",
                    "worker timeout overflow",
                )
            })?;

        let output = timeout(process_timeout, child)
            .await
            .map_err(|_| {
                SourceBaselineError::new(
                    "source_baseline_worker_timeout",
                    "isolated baseline worker exceeded parent timeout",
                )
            })?
            .map_err(|_| {
                SourceBaselineError::new(
                    "source_baseline_worker_failed",
                    "isolated baseline worker could not be started",
                )
            })?;

        if !output.status.success() {
            return Err(SourceBaselineError::new(
                "source_baseline_worker_failed",
                "isolated baseline worker rejected or failed",
            ));
        }

        let receipt_path = output_dir.join("result.json");
        let metadata = fs::metadata(&receipt_path).await.map_err(|_| {
            SourceBaselineError::new(
                "source_baseline_receipt_missing",
                "isolated baseline worker receipt is missing",
            )
        })?;
        if metadata.len() == 0 || metadata.len() > RECEIPT_MAX_BYTES {
            return Err(SourceBaselineError::new(
                "source_baseline_receipt_invalid",
                "isolated baseline worker receipt size is invalid",
            ));
        }
        let bytes = fs::read(&receipt_path).await.map_err(|_| {
            SourceBaselineError::new(
                "source_baseline_receipt_missing",
                "isolated baseline worker receipt could not be read",
            )
        })?;
        let receipt: SourceBaselineReceiptV1 = serde_json::from_slice(&bytes).map_err(|_| {
            SourceBaselineError::new(
                "source_baseline_receipt_invalid",
                "isolated baseline worker receipt is malformed",
            )
        })?;
        validate_receipt(
            &receipt,
            document_id,
            expected_source_sha256,
            expected_source_byte_len,
        )?;

        Ok(ProjectBaselineIdentity {
            service_revision_id: receipt.service_revision_id,
            canonical_schema_version: receipt.canonical_schema_version,
            canonical_authoring_revision_id: receipt.canonical_authoring_revision_id,
        })
    }
}

fn validate_receipt(
    receipt: &SourceBaselineReceiptV1,
    document_id: &str,
    expected_source_sha256: &str,
    expected_source_byte_len: u64,
) -> Result<(), SourceBaselineError> {
    if receipt.protocol_version != SOURCE_BASELINE_RECEIPT_V1
        || receipt.document_id != document_id
        || receipt.source_sha256 != expected_source_sha256
        || receipt.source_byte_len != expected_source_byte_len
        || receipt.canonical_schema_version != AUTHORING_REVISION_SCHEMA_V1
        || !receipt.filesystem_confinement
    {
        return Err(SourceBaselineError::new(
            "source_baseline_receipt_identity_mismatch",
            "isolated baseline worker receipt does not match the authorized source identity/profile",
        ));
    }
    require_ident(
        &receipt.project_schema_version,
        "receipt.project_schema_version",
    )?;
    require_prefixed_sha256(&receipt.project_hash, "receipt.project_hash")?;
    require_prefixed_sha256(&receipt.state_id, "receipt.state_id")?;
    require_prefixed_sha256(&receipt.service_revision_id, "receipt.service_revision_id")?;
    require_sha256(
        &receipt.canonical_authoring_revision_id,
        "receipt.canonical_authoring_revision_id",
    )?;
    Ok(())
}

pub fn run_source_baseline_worker(
    document_id: &str,
    expected_source_sha256: &str,
    expected_source_byte_len: u64,
) -> Result<(), SourceBaselineError> {
    require_ident(document_id, "document_id")?;
    require_sha256(expected_source_sha256, "expected_source_sha256")?;
    if expected_source_byte_len == 0 {
        return Err(SourceBaselineError::new(
            "source_baseline_length_invalid",
            "expected source byte length must be positive",
        ));
    }

    let input_path = PathBuf::from(required_env("CHAPTERA_WORKER_INPUT")?);
    let output_root = PathBuf::from(required_env("CHAPTERA_WORKER_OUTPUT_DIR")?);
    stdfs::create_dir_all(&output_root).map_err(|_| {
        SourceBaselineError::new(
            "source_baseline_worker_output_failed",
            "baseline worker output directory is unavailable",
        )
    })?;
    let output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output_root.join("result.json"))
        .map(BufWriter::new)
        .map_err(|_| {
            SourceBaselineError::new(
                "source_baseline_worker_output_failed",
                "baseline worker result file could not be created",
            )
        })?;

    let metadata = stdfs::metadata(&input_path).map_err(|_| {
        SourceBaselineError::new(
            "source_baseline_worker_input_failed",
            "authorized baseline input is unavailable",
        )
    })?;
    if metadata.len() != expected_source_byte_len {
        return Err(SourceBaselineError::new(
            "source_baseline_length_mismatch",
            "authorized baseline input length differs from expected identity",
        ));
    }

    let source_bytes = stdfs::read(&input_path).map_err(|_| {
        SourceBaselineError::new(
            "source_baseline_worker_input_failed",
            "authorized baseline input could not be read",
        )
    })?;
    let actual_sha256 = format!("{:x}", Sha256::digest(&source_bytes));
    if actual_sha256 != expected_source_sha256 {
        return Err(SourceBaselineError::new(
            "source_baseline_hash_mismatch",
            "authorized baseline input hash differs from expected identity",
        ));
    }

    install_post_read_filesystem_default_deny().map_err(|_| {
        SourceBaselineError::new(
            "source_baseline_sandbox_failed",
            "post-read filesystem default-deny could not be installed",
        )
    })?;

    let editor = PubEditorReplayEngine;
    let project: EditorProject = editor
        .baseline_project(&source_bytes, expected_source_sha256)
        .map_err(|error| SourceBaselineError::new(error.code, error.message))?;
    let project_source = project.source_hash.to_string();
    if project_source != expected_source_sha256 {
        return Err(SourceBaselineError::new(
            "source_baseline_project_source_mismatch",
            "canonical baseline project is bound to a different immutable source",
        ));
    }

    let identities = derive_import_baseline_identities(
        document_id,
        expected_source_sha256,
        &project.schema_version,
        &project,
    )?;
    let receipt = SourceBaselineReceiptV1 {
        protocol_version: SOURCE_BASELINE_RECEIPT_V1.to_owned(),
        document_id: document_id.to_owned(),
        source_sha256: expected_source_sha256.to_owned(),
        source_byte_len: expected_source_byte_len,
        project_schema_version: project.schema_version.clone(),
        project_hash: identities.project_hash,
        state_id: identities.state_id,
        service_revision_id: identities.service_revision_id,
        canonical_schema_version: AUTHORING_REVISION_SCHEMA_V1.to_owned(),
        canonical_authoring_revision_id: identities.canonical_authoring_revision_id,
        filesystem_confinement: true,
    };
    write_receipt(output, &receipt)
}

fn write_receipt(
    mut output: BufWriter<File>,
    receipt: &SourceBaselineReceiptV1,
) -> Result<(), SourceBaselineError> {
    serde_json::to_writer(&mut output, receipt).map_err(|_| {
        SourceBaselineError::new(
            "source_baseline_worker_output_failed",
            "baseline worker receipt serialization failed",
        )
    })?;
    output.write_all(b"\n").map_err(|_| {
        SourceBaselineError::new(
            "source_baseline_worker_output_failed",
            "baseline worker receipt write failed",
        )
    })?;
    output.flush().map_err(|_| {
        SourceBaselineError::new(
            "source_baseline_worker_output_failed",
            "baseline worker receipt flush failed",
        )
    })
}

fn required_env(name: &str) -> Result<String, SourceBaselineError> {
    env::var(name).map_err(|_| {
        SourceBaselineError::new(
            "source_baseline_worker_environment_missing",
            format!("{name} is required"),
        )
    })
}

fn require_ident(value: &str, label: &str) -> Result<(), SourceBaselineError> {
    if value.is_empty()
        || value.len() > 256
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(SourceBaselineError::new(
            "source_baseline_identity_invalid",
            format!("{label} must be a bounded non-whitespace identity"),
        ));
    }
    Ok(())
}

fn require_sha256(value: &str, label: &str) -> Result<(), SourceBaselineError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SourceBaselineError::new(
            "source_baseline_identity_invalid",
            format!("{label} must be 64 lowercase SHA-256 hex characters"),
        ));
    }
    Ok(())
}

fn require_prefixed_sha256(value: &str, label: &str) -> Result<(), SourceBaselineError> {
    let Some(raw) = value.strip_prefix("sha256:") else {
        return Err(SourceBaselineError::new(
            "source_baseline_identity_invalid",
            format!("{label} must use sha256: identity syntax"),
        ));
    };
    require_sha256(raw, label)
}

struct BaselineTempDir {
    path: PathBuf,
}

impl BaselineTempDir {
    async fn create(root: &Path) -> Result<Self, SourceBaselineError> {
        fs::create_dir_all(root).await.map_err(|_| {
            SourceBaselineError::new(
                "source_baseline_temp_failed",
                "baseline temp root is unavailable",
            )
        })?;

        for _ in 0..8 {
            let mut random = [0_u8; 16];
            OsRng.try_fill_bytes(&mut random).map_err(|_| {
                SourceBaselineError::new(
                    "source_baseline_random_failed",
                    "baseline temp identity generation failed",
                )
            })?;
            let path = root.join(format!(
                "chaptera-source-baseline-{:032x}",
                u128::from_be_bytes(random)
            ));
            match fs::create_dir(&path).await {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&path, stdfs::Permissions::from_mode(0o700))
                            .await
                            .map_err(|_| {
                                SourceBaselineError::new(
                                    "source_baseline_temp_failed",
                                    "baseline temp permissions could not be constrained",
                                )
                            })?;
                    }
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => {
                    return Err(SourceBaselineError::new(
                        "source_baseline_temp_failed",
                        "baseline temp directory could not be created",
                    ));
                }
            }
        }
        Err(SourceBaselineError::new(
            "source_baseline_temp_failed",
            "baseline temp collision budget exhausted",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for BaselineTempDir {
    fn drop(&mut self) {
        let _ = stdfs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const DOCUMENT_ID: &str = "9c2f8d5e-3f1b-4a57-9d40-6a7e2c11b001";
    const SOURCE_SHA: &str = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf";

    #[test]
    fn sample_newsletter_baseline_matches_existing_web_and_authoring_goldens() {
        let project = json!({
            "operations": [],
            "schema_version": "pub-editor-v0.2",
            "source_hash": SOURCE_SHA,
        });
        let ids =
            derive_import_baseline_identities(DOCUMENT_ID, SOURCE_SHA, "pub-editor-v0.2", &project)
                .unwrap();

        assert_eq!(
            ids.project_hash,
            "sha256:575fbcb664f2a6b672a05861a4d2aff6aca339204a50e3401d04e1920d946348"
        );
        assert_eq!(
            ids.state_id,
            "sha256:ce6753aab36f31db5508763601d078d2d5a2766541ef6d3377eaab9ed02be191"
        );
        assert_eq!(
            ids.service_revision_id,
            "sha256:853fa2471bbf5ce479340e391550c5ee7b972f208a36d81c26808ab52c672d6c"
        );
        assert_eq!(
            ids.canonical_authoring_revision_id,
            "5e246c364ec168c876ed07306a1ff99d5b2eb78913f5bd36777bc2863a5360f3"
        );
    }

    #[test]
    fn receipt_rejects_any_authority_identity_mismatch() {
        let receipt = SourceBaselineReceiptV1 {
            protocol_version: SOURCE_BASELINE_RECEIPT_V1.into(),
            document_id: DOCUMENT_ID.into(),
            source_sha256: SOURCE_SHA.into(),
            source_byte_len: 291_840,
            project_schema_version: "pub-editor-v0.2".into(),
            project_hash: "sha256:575fbcb664f2a6b672a05861a4d2aff6aca339204a50e3401d04e1920d946348"
                .into(),
            state_id: "sha256:ce6753aab36f31db5508763601d078d2d5a2766541ef6d3377eaab9ed02be191"
                .into(),
            service_revision_id:
                "sha256:853fa2471bbf5ce479340e391550c5ee7b972f208a36d81c26808ab52c672d6c".into(),
            canonical_schema_version: AUTHORING_REVISION_SCHEMA_V1.into(),
            canonical_authoring_revision_id:
                "5e246c364ec168c876ed07306a1ff99d5b2eb78913f5bd36777bc2863a5360f3".into(),
            filesystem_confinement: true,
        };
        validate_receipt(&receipt, DOCUMENT_ID, SOURCE_SHA, 291_840).unwrap();

        let mut wrong = receipt.clone();
        wrong.source_sha256 = "0".repeat(64);
        assert_eq!(
            validate_receipt(&wrong, DOCUMENT_ID, SOURCE_SHA, 291_840)
                .unwrap_err()
                .code,
            "source_baseline_receipt_identity_mismatch"
        );

        let mut unconfined = receipt;
        unconfined.filesystem_confinement = false;
        assert_eq!(
            validate_receipt(&unconfined, DOCUMENT_ID, SOURCE_SHA, 291_840)
                .unwrap_err()
                .code,
            "source_baseline_receipt_identity_mismatch"
        );
    }
}
