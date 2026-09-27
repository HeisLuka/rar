use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use tough::{ExpirationEnforcement, FilesystemTransport, IntoVec, Repository, RepositoryLoader, TargetName};
use url::Url;

pub const UPDATE_PROTOCOL_VERSION: u32 = 1;
pub const INSTALL_LAYOUT_EPOCH: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    PayloadSwap,
    InstallerRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapteraReleaseSemantics {
    pub product_id: String,
    pub architecture: String,
    pub channel: String,
    pub package_version: String,
    pub install_layout_epoch: u32,
    pub update_protocol_version: u32,
    pub update_mode: UpdateMode,
    pub state_schema: String,
    pub rollback_compatible_from: Vec<String>,
    pub installed_tree_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledUpdateContext<'a> {
    pub product_id: &'a str,
    pub architecture: &'a str,
    pub channel: &'a str,
    pub install_layout_epoch: u32,
    pub max_update_protocol_version: u32,
}

impl ChapteraReleaseSemantics {
    pub fn validate_for(&self, installed: InstalledUpdateContext<'_>) -> Result<()> {
        if self.product_id != installed.product_id {
            bail!(
                "update product mismatch: expected {}, got {}",
                installed.product_id,
                self.product_id
            );
        }
        if self.architecture != installed.architecture {
            bail!(
                "update architecture mismatch: expected {}, got {}",
                installed.architecture,
                self.architecture
            );
        }
        if self.channel != installed.channel {
            bail!(
                "update channel mismatch: expected {}, got {}",
                installed.channel,
                self.channel
            );
        }
        if self.install_layout_epoch != installed.install_layout_epoch {
            bail!(
                "unsupported install layout epoch: installed {}, target {}",
                installed.install_layout_epoch,
                self.install_layout_epoch
            );
        }
        if self.update_protocol_version > installed.max_update_protocol_version {
            bail!(
                "unsupported updater protocol version: client {}, target {}",
                installed.max_update_protocol_version,
                self.update_protocol_version
            );
        }
        if self.installed_tree_bytes == 0 {
            bail!("installed_tree_bytes must be non-zero");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedPayloadReceipt {
    pub schema_version: String,
    pub target_name: String,
    pub product_id: String,
    pub architecture: String,
    pub channel: String,
    pub package_version: String,
    pub payload_sha256: String,
    pub payload_byte_len: u64,
    pub installed_tree_bytes: u64,
    pub timestamp_version: u64,
    pub snapshot_version: u64,
    pub targets_version: u64,
}

pub async fn read_verified_release_payload(
    repository: &Repository,
    target_name: &str,
    release: &ChapteraReleaseSemantics,
) -> Result<(Vec<u8>, VerifiedPayloadReceipt)> {
    let target_name_parsed =
        TargetName::new(target_name).context("invalid TUF target name")?;
    let stream = repository
        .read_target(&target_name_parsed)
        .await
        .context("read verified TUF target")?
        .ok_or_else(|| anyhow::anyhow!("selected TUF target is missing: {target_name}"))?;
    let bytes = stream.into_vec().await.context("read verified target bytes")?;

    let payload_sha256 = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let receipt = VerifiedPayloadReceipt {
        schema_version: "chaptera.verified-payload.v1".to_owned(),
        target_name: target_name.to_owned(),
        product_id: release.product_id.clone(),
        architecture: release.architecture.clone(),
        channel: release.channel.clone(),
        package_version: release.package_version.clone(),
        payload_sha256,
        payload_byte_len: bytes.len() as u64,
        installed_tree_bytes: release.installed_tree_bytes,
        timestamp_version: repository.timestamp().signed.version.get(),
        snapshot_version: repository.snapshot().signed.version.get(),
        targets_version: repository.targets().signed.version.get(),
    };

    Ok((bytes, receipt))
}

pub fn verify_payload_against_receipt(
    payload: &[u8],
    receipt: &VerifiedPayloadReceipt,
) -> Result<()> {
    if receipt.schema_version != "chaptera.verified-payload.v1" {
        bail!("unsupported verified payload receipt schema");
    }
    if payload.len() as u64 != receipt.payload_byte_len {
        bail!(
            "verified payload length mismatch: receipt {}, actual {}",
            receipt.payload_byte_len,
            payload.len()
        );
    }
    let actual_sha256 = Sha256::digest(payload)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual_sha256 != receipt.payload_sha256 {
        bail!("verified payload digest mismatch");
    }
    Ok(())
}

/// Loads and verifies a TUF repository from a filesystem-backed test/local origin.
///
/// Expiration is explicitly enforced in Safe mode. The datastore must live outside
/// the versioned product tree so rollback of binaries cannot roll back trusted
/// metadata/high-water state. The wrapper creates the caller-selected datastore
/// directory when absent; choosing its durable location remains a product/runtime
/// responsibility.
pub async fn load_local_tuf_repository(
    trusted_root: &[u8],
    metadata_dir: &Path,
    targets_dir: &Path,
    datastore_dir: &Path,
) -> Result<Repository> {
    std::fs::create_dir_all(datastore_dir)
        .with_context(|| format!("create TUF datastore {}", datastore_dir.display()))?;
    let metadata_base_url = Url::from_directory_path(metadata_dir)
        .map_err(|_| anyhow::anyhow!("metadata directory cannot be represented as a file URL"))?;
    let targets_base_url = Url::from_directory_path(targets_dir)
        .map_err(|_| anyhow::anyhow!("targets directory cannot be represented as a file URL"))?;

    RepositoryLoader::new(&trusted_root, metadata_base_url, targets_base_url)
        .transport(FilesystemTransport)
        .datastore(datastore_dir)
        .expiration_enforcement(ExpirationEnforcement::Safe)
        .load()
        .await
        .context("TUF repository verification failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed() -> InstalledUpdateContext<'static> {
        InstalledUpdateContext {
            product_id: "chaptera.reader",
            architecture: "windows-x86_64",
            channel: "stable",
            install_layout_epoch: INSTALL_LAYOUT_EPOCH,
            max_update_protocol_version: UPDATE_PROTOCOL_VERSION,
        }
    }

    fn release() -> ChapteraReleaseSemantics {
        ChapteraReleaseSemantics {
            product_id: "chaptera.reader".into(),
            architecture: "windows-x86_64".into(),
            channel: "stable".into(),
            package_version: "0.2.0".into(),
            install_layout_epoch: INSTALL_LAYOUT_EPOCH,
            update_protocol_version: UPDATE_PROTOCOL_VERSION,
            update_mode: UpdateMode::PayloadSwap,
            state_schema: "reader-state-v1".into(),
            rollback_compatible_from: vec!["0.1.0".into()],
            installed_tree_bytes: 1024,
        }
    }

    #[test]
    fn verified_payload_receipt_rejects_payload_substitution() {
        let payload = b"signed-release-payload";
        let receipt = VerifiedPayloadReceipt {
            schema_version: "chaptera.verified-payload.v1".into(),
            target_name: "chaptera-reader-win-x64.zip".into(),
            product_id: "chaptera.reader".into(),
            architecture: "windows-x86_64".into(),
            channel: "stable".into(),
            package_version: "0.2.0".into(),
            payload_sha256: Sha256::digest(payload)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            payload_byte_len: payload.len() as u64,
            installed_tree_bytes: 1024,
            timestamp_version: 2,
            snapshot_version: 2,
            targets_version: 2,
        };

        verify_payload_against_receipt(payload, &receipt).unwrap();
        assert!(verify_payload_against_receipt(b"substituted-payload", &receipt).is_err());
    }

    #[test]
    fn verified_payload_receipt_rejects_receipt_tamper() {
        let payload = b"signed-release-payload";
        let mut receipt = VerifiedPayloadReceipt {
            schema_version: "chaptera.verified-payload.v1".into(),
            target_name: "chaptera-reader-win-x64.zip".into(),
            product_id: "chaptera.reader".into(),
            architecture: "windows-x86_64".into(),
            channel: "stable".into(),
            package_version: "0.2.0".into(),
            payload_sha256: Sha256::digest(payload)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            payload_byte_len: payload.len() as u64,
            installed_tree_bytes: 1024,
            timestamp_version: 2,
            snapshot_version: 2,
            targets_version: 2,
        };

        receipt.payload_sha256 = "00".repeat(32);
        assert!(verify_payload_against_receipt(payload, &receipt).is_err());
        receipt.payload_sha256 = Sha256::digest(payload)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        receipt.payload_byte_len += 1;
        assert!(verify_payload_against_receipt(payload, &receipt).is_err());
    }

    #[test]
    fn accepts_matching_release_semantics() {
        release().validate_for(installed()).unwrap();
    }

    #[test]
    fn rejects_wrong_product() {
        let mut value = release();
        value.product_id = "chaptera.editor".into();
        assert!(value.validate_for(installed()).is_err());
    }

    #[test]
    fn rejects_wrong_channel() {
        let mut value = release();
        value.channel = "beta".into();
        assert!(value.validate_for(installed()).is_err());
    }

    #[test]
    fn rejects_unknown_layout_epoch() {
        let mut value = release();
        value.install_layout_epoch += 1;
        assert!(value.validate_for(installed()).is_err());
    }

    #[test]
    fn rejects_newer_updater_protocol() {
        let mut value = release();
        value.update_protocol_version += 1;
        assert!(value.validate_for(installed()).is_err());
    }

    #[test]
    fn rejects_zero_tree_size() {
        let mut value = release();
        value.installed_tree_bytes = 0;
        assert!(value.validate_for(installed()).is_err());
    }
}
