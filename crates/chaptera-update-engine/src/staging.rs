use crate::{Result, UpdateError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestFile {
    pub path: String,
    pub length: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeManifest {
    pub files: Vec<ManifestFile>,
    pub file_count: u64,
    pub installed_tree_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagingLimits {
    pub max_files: u64,
    pub max_installed_bytes: u64,
    pub temp_overhead_bytes: u64,
    pub retained_predecessor_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceBudget {
    pub required_bytes: u64,
    pub available_bytes: u64,
}

pub fn validate_manifest(manifest: &TreeManifest, limits: StagingLimits) -> Result<()> {
    if manifest.file_count != manifest.files.len() as u64 {
        return invariant("manifest file_count does not match file list");
    }
    if manifest.file_count > limits.max_files {
        return invariant("manifest exceeds configured file-count limit");
    }
    if manifest.installed_tree_bytes > limits.max_installed_bytes {
        return invariant("manifest exceeds configured installed-byte limit");
    }

    let mut total = 0u64;
    let mut names = HashSet::new();
    for file in &manifest.files {
        validate_manifest_path(&file.path)?;
        validate_sha256(&file.sha256)?;
        total = total
            .checked_add(file.length)
            .ok_or_else(|| UpdateError::LayoutInvariant("manifest byte count overflow".into()))?;
        let folded = file.path.to_ascii_lowercase();
        if !names.insert(folded) {
            return invariant("manifest contains a case-insensitive path collision");
        }
    }
    if total != manifest.installed_tree_bytes {
        return invariant("manifest installed_tree_bytes does not match file lengths");
    }
    Ok(())
}

pub fn preflight_space(
    install_root: &Path,
    compressed_bytes: u64,
    manifest: &TreeManifest,
    limits: StagingLimits,
) -> Result<SpaceBudget> {
    validate_manifest(manifest, limits)?;
    fs::create_dir_all(install_root)?;
    let required_bytes = compressed_bytes
        .checked_add(manifest.installed_tree_bytes)
        .and_then(|value| value.checked_add(limits.temp_overhead_bytes))
        .and_then(|value| value.checked_add(limits.retained_predecessor_bytes))
        .ok_or_else(|| UpdateError::LayoutInvariant("staging disk budget overflow".into()))?;
    let available_bytes = fs4::available_space(install_root)?;
    if available_bytes < required_bytes {
        return invariant(format!(
            "insufficient staging space: need {required_bytes} bytes, have {available_bytes}"
        ));
    }
    Ok(SpaceBudget { required_bytes, available_bytes })
}

pub fn stage_verified_zip(
    install_root: &Path,
    zip_path: &Path,
    manifest: &TreeManifest,
    limits: StagingLimits,
    destination: &Path,
) -> Result<SpaceBudget> {
    let compressed_bytes = fs::metadata(zip_path)?.len();
    let budget = preflight_space(install_root, compressed_bytes, manifest, limits)?;
    if destination.exists() {
        return invariant(format!("staging destination already exists: {}", destination.display()));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| UpdateError::LayoutInvariant("staging destination has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let temp = destination.with_extension("extracting");
    if temp.exists() {
        return invariant(format!("temporary staging path already exists: {}", temp.display()));
    }
    fs::create_dir(&temp)?;

    let result = extract_and_verify(zip_path, manifest, &temp);
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temp);
        return Err(error);
    }
    fs::rename(&temp, destination)?;
    sync_directory_if_supported(parent)?;
    Ok(budget)
}

fn extract_and_verify(zip_path: &Path, manifest: &TreeManifest, destination: &Path) -> Result<()> {
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| UpdateError::LayoutInvariant(format!("invalid candidate ZIP: {error}")))?;

    let expected: std::collections::HashMap<&str, &ManifestFile> =
        manifest.files.iter().map(|file| (file.path.as_str(), file)).collect();
    let mut seen = HashSet::new();
    let mut extracted_bytes = 0u64;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| UpdateError::LayoutInvariant(format!("read candidate ZIP entry: {error}")))?;
        let raw = entry.name().to_owned();
        if entry.is_dir() {
            let trimmed = raw.trim_end_matches('/');
            if !trimmed.is_empty() {
                validate_manifest_path(trimmed)?;
                fs::create_dir_all(destination.join(path_from_manifest(trimmed)?))?;
            }
            continue;
        }
        validate_manifest_path(&raw)?;
        if entry.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000) {
            return invariant(format!("candidate ZIP contains symlink entry: {raw}"));
        }
        let expected_file = expected
            .get(raw.as_str())
            .ok_or_else(|| UpdateError::LayoutInvariant(format!("candidate ZIP contains extra file: {raw}")))?;
        if !seen.insert(raw.clone()) {
            return invariant(format!("candidate ZIP contains duplicate file: {raw}"));
        }
        if entry.size() != expected_file.length {
            return invariant(format!("candidate ZIP length mismatch for {raw}"));
        }

        let target = destination.join(path_from_manifest(&raw)?);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = OpenOptions::new().create_new(true).write(true).open(&target)?;
        let mut digest = Sha256::new();
        let mut copied = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|error| UpdateError::LayoutInvariant(format!("read candidate ZIP payload: {error}")))?;
            if read == 0 {
                break;
            }
            copied = copied
                .checked_add(read as u64)
                .ok_or_else(|| UpdateError::LayoutInvariant("extracted byte count overflow".into()))?;
            extracted_bytes = extracted_bytes
                .checked_add(read as u64)
                .ok_or_else(|| UpdateError::LayoutInvariant("total extracted byte count overflow".into()))?;
            if copied > expected_file.length || extracted_bytes > manifest.installed_tree_bytes {
                return invariant("candidate ZIP expands beyond authenticated manifest");
            }
            digest.update(&buffer[..read]);
            output.write_all(&buffer[..read])?;
        }
        output.sync_all()?;
        if copied != expected_file.length {
            return invariant(format!("candidate ZIP extracted length mismatch for {raw}"));
        }
        let actual = format!("{:x}", digest.finalize());
        if actual != expected_file.sha256.to_ascii_lowercase() {
            return invariant(format!("candidate ZIP SHA-256 mismatch for {raw}"));
        }
    }

    if seen.len() != manifest.files.len() {
        return invariant("candidate ZIP is missing one or more manifest files");
    }
    if extracted_bytes != manifest.installed_tree_bytes {
        return invariant("candidate ZIP total extracted bytes do not match manifest");
    }
    Ok(())
}

fn validate_manifest_path(path: &str) -> Result<()> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
        return invariant(format!("unsafe manifest path: {path:?}"));
    }
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        return invariant(format!("drive-qualified manifest path: {path:?}"));
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return invariant(format!("absolute manifest path: {path:?}"));
    }
    let mut count = 0usize;
    for component in candidate.components() {
        let Component::Normal(value) = component else {
            return invariant(format!("unsafe manifest path component: {path:?}"));
        };
        let value = value
            .to_str()
            .ok_or_else(|| UpdateError::LayoutInvariant("manifest path is not UTF-8".into()))?;
        validate_windows_component(value)?;
        count += 1;
    }
    if count == 0 {
        return invariant("empty manifest path");
    }
    Ok(())
}

fn validate_windows_component(value: &str) -> Result<()> {
    if value.ends_with(' ') || value.ends_with('.') {
        return invariant(format!("Windows-ambiguous path component: {value:?}"));
    }
    if value.chars().any(|ch| ch < ' ' || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*')) {
        return invariant(format!("Windows-invalid path component: {value:?}"));
    }
    let stem = value.split('.').next().unwrap_or(value).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if reserved {
        return invariant(format!("Windows-reserved path component: {value:?}"));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return invariant("manifest SHA-256 must be exactly 64 hexadecimal characters");
    }
    Ok(())
}

fn path_from_manifest(path: &str) -> Result<PathBuf> {
    validate_manifest_path(path)?;
    Ok(path.split('/').collect())
}

fn invariant<T>(message: impl Into<String>) -> Result<T> {
    Err(UpdateError::LayoutInvariant(message.into()))
}

#[cfg(unix)]
fn sync_directory_if_supported(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory_if_supported(_path: &Path) -> Result<()> {
    Ok(())
}
