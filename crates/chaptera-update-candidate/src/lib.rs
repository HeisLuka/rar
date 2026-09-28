use anyhow::{Context, Result, bail};
use chaptera_update_trust::{VerifiedPayloadReceipt, verify_payload_against_receipt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

pub const CANDIDATE_RECEIPT_SCHEMA_VERSION: &str = "chaptera.verified-candidate.v1";
const EXPECTED_ENTRIES: [&str; 2] = ["Chaptera-Reader.exe", "README.md"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedCandidateReceipt {
    pub schema_version: String,
    pub product_id: String,
    pub architecture: String,
    pub channel: String,
    pub package_version: String,
    pub source_payload_sha256: String,
    pub source_payload_byte_len: u64,
    pub installed_tree_bytes: u64,
    pub tree_sha256: String,
    pub files: Vec<VerifiedCandidateFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedCandidateFile {
    pub relative_path: String,
    pub sha256: String,
    pub byte_len: u64,
}

pub fn extract_verified_reader_candidate(
    payload: &[u8],
    payload_receipt: &VerifiedPayloadReceipt,
    destination: &Path,
) -> Result<VerifiedCandidateReceipt> {
    verify_payload_against_receipt(payload, payload_receipt)?;

    if destination.exists() {
        bail!(
            "candidate destination already exists: {}",
            destination.display()
        );
    }

    let mut archive =
        ZipArchive::new(Cursor::new(payload)).context("open verified Reader ZIP payload")?;
    if archive.len() != EXPECTED_ENTRIES.len() {
        bail!(
            "Reader payload entry count mismatch: expected {}, got {}",
            EXPECTED_ENTRIES.len(),
            archive.len()
        );
    }

    let mut extracted = BTreeMap::<String, Vec<u8>>::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).context("read Reader ZIP entry")?;
        if entry.is_dir() {
            bail!("Reader payload must not contain directory entries");
        }

        let name = entry.name().replace('\\', "/");
        if name.starts_with('/')
            || name.contains("../")
            || name.contains("/..")
            || name.contains('/')
            || !EXPECTED_ENTRIES.contains(&name.as_str())
        {
            bail!("unexpected or unsafe Reader payload entry: {name}");
        }
        if extracted.contains_key(&name) {
            bail!("duplicate Reader payload entry: {name}");
        }

        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .with_context(|| format!("read Reader payload entry {name}"))?;
        extracted.insert(name, bytes);
    }

    for expected in EXPECTED_ENTRIES {
        if !extracted.contains_key(expected) {
            bail!("Reader payload is missing required entry: {expected}");
        }
    }

    let installed_tree_bytes = extracted
        .values()
        .try_fold(0_u64, |total, bytes| total.checked_add(bytes.len() as u64))
        .ok_or_else(|| anyhow::anyhow!("installed tree byte count overflow"))?;
    if installed_tree_bytes != payload_receipt.installed_tree_bytes {
        bail!(
            "installed tree size mismatch: receipt {}, extracted {}",
            payload_receipt.installed_tree_bytes,
            installed_tree_bytes
        );
    }

    fs::create_dir_all(destination)
        .with_context(|| format!("create candidate tree {}", destination.display()))?;

    let mut files = Vec::new();
    let mut tree_hasher = Sha256::new();
    for (name, bytes) in &extracted {
        let path = destination.join(name);
        fs::write(&path, bytes)
            .with_context(|| format!("write candidate file {}", path.display()))?;

        let sha256 = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        tree_hasher.update((name.len() as u64).to_le_bytes());
        tree_hasher.update(name.as_bytes());
        tree_hasher.update((bytes.len() as u64).to_le_bytes());
        tree_hasher.update(bytes);

        files.push(VerifiedCandidateFile {
            relative_path: name.clone(),
            sha256,
            byte_len: bytes.len() as u64,
        });
    }

    let tree_sha256 = tree_hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    Ok(VerifiedCandidateReceipt {
        schema_version: CANDIDATE_RECEIPT_SCHEMA_VERSION.to_owned(),
        product_id: payload_receipt.product_id.clone(),
        architecture: payload_receipt.architecture.clone(),
        channel: payload_receipt.channel.clone(),
        package_version: payload_receipt.package_version.clone(),
        source_payload_sha256: payload_receipt.payload_sha256.clone(),
        source_payload_byte_len: payload_receipt.payload_byte_len,
        installed_tree_bytes,
        tree_sha256,
        files,
    })
}

pub fn verify_candidate_tree(root: &Path, receipt: &VerifiedCandidateReceipt) -> Result<()> {
    if receipt.schema_version != CANDIDATE_RECEIPT_SCHEMA_VERSION {
        bail!("unsupported verified candidate receipt schema");
    }

    let mut files = BTreeMap::new();
    for entry in
        fs::read_dir(root).with_context(|| format!("read candidate tree {}", root.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            bail!(
                "candidate tree contains non-file entry: {}",
                entry.path().display()
            );
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("candidate file name is not UTF-8"))?;
        if !EXPECTED_ENTRIES.contains(&name.as_str()) {
            bail!("candidate tree contains unexpected file: {name}");
        }
        files.insert(name, fs::read(entry.path())?);
    }

    if files.len() != receipt.files.len() {
        bail!("candidate tree file count mismatch");
    }

    let mut tree_hasher = Sha256::new();
    let mut total = 0_u64;
    for expected in &receipt.files {
        let bytes = files
            .get(&expected.relative_path)
            .ok_or_else(|| anyhow::anyhow!("candidate file missing: {}", expected.relative_path))?;
        if bytes.len() as u64 != expected.byte_len {
            bail!("candidate file length mismatch: {}", expected.relative_path);
        }
        let sha256 = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if sha256 != expected.sha256 {
            bail!("candidate file digest mismatch: {}", expected.relative_path);
        }

        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("candidate byte count overflow"))?;
        tree_hasher.update((expected.relative_path.len() as u64).to_le_bytes());
        tree_hasher.update(expected.relative_path.as_bytes());
        tree_hasher.update((bytes.len() as u64).to_le_bytes());
        tree_hasher.update(bytes);
    }

    if total != receipt.installed_tree_bytes {
        bail!("candidate installed tree byte count mismatch");
    }
    let actual_tree = tree_hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual_tree != receipt.tree_sha256 {
        bail!("candidate tree digest mismatch");
    }
    Ok(())
}

pub fn candidate_reader_executable(root: &Path) -> PathBuf {
    root.join("Chaptera-Reader.exe")
}
