use crate::tree_sha256;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

pub const TREE_MANIFEST_SCHEMA_V1: &str = "chaptera.update-tree.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeFile {
    pub path: String,
    pub byte_len: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductTreeManifest {
    pub schema_version: String,
    pub file_count: u64,
    pub installed_tree_bytes: u64,
    pub files: Vec<TreeFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedTree {
    pub tree_sha256: String,
    pub file_count: u64,
    pub installed_tree_bytes: u64,
}

impl ProductTreeManifest {
    pub fn validate(&self) -> Result<BTreeMap<String, TreeFile>> {
        if self.schema_version != TREE_MANIFEST_SCHEMA_V1 {
            bail!("unsupported update tree manifest schema: {}", self.schema_version);
        }
        if self.file_count != self.files.len() as u64 {
            bail!(
                "tree manifest file_count mismatch: declared {}, actual {}",
                self.file_count,
                self.files.len()
            );
        }

        let mut exact = BTreeSet::new();
        let mut windows_folded = BTreeSet::new();
        let mut map = BTreeMap::new();
        let mut total = 0u64;

        for file in &self.files {
            validate_windows_safe_relative_path(&file.path)?;
            if file.byte_len == 0 {
                bail!("zero-length files are not admitted in update tree v1: {}", file.path);
            }
            validate_sha256(&file.sha256)?;

            if !exact.insert(file.path.clone()) {
                bail!("duplicate tree manifest path: {}", file.path);
            }
            let folded = file.path.to_ascii_lowercase();
            if !windows_folded.insert(folded) {
                bail!("Windows case-insensitive path collision: {}", file.path);
            }

            total = total
                .checked_add(file.byte_len)
                .context("tree manifest installed byte count overflow")?;
            map.insert(file.path.clone(), file.clone());
        }

        if total != self.installed_tree_bytes {
            bail!(
                "tree manifest installed_tree_bytes mismatch: declared {}, summed {}",
                self.installed_tree_bytes,
                total
            );
        }
        Ok(map)
    }
}

pub fn stage_zip_payload(
    archive_path: &Path,
    destination: &Path,
    manifest: &ProductTreeManifest,
) -> Result<StagedTree> {
    let expected = manifest.validate()?;
    if destination.exists() {
        bail!("staging destination already exists: {}", destination.display());
    }
    fs::create_dir_all(destination)
        .with_context(|| format!("create staging destination {}", destination.display()))?;

    let result = stage_zip_payload_inner(archive_path, destination, manifest, &expected);
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn stage_zip_payload_inner(
    archive_path: &Path,
    destination: &Path,
    manifest: &ProductTreeManifest,
    expected: &BTreeMap<String, TreeFile>,
) -> Result<StagedTree> {
    let file = File::open(archive_path)
        .with_context(|| format!("open update archive {}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file).context("open update ZIP")?;
    let mut seen = BTreeSet::new();
    let mut total_written = 0u64;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).context("read ZIP entry")?;
        let raw_name = entry.name().to_owned();
        validate_windows_safe_zip_name(&raw_name)?;

        if is_zip_symlink(&entry) {
            bail!("symlink entries are forbidden in update payload: {raw_name}");
        }
        if entry.is_dir() {
            let dir = raw_name.trim_end_matches('/');
            if !dir.is_empty() {
                let prefix = format!("{dir}/");
                if !expected.keys().any(|path| path.starts_with(&prefix)) {
                    bail!("extra directory entry not represented by tree manifest: {raw_name}");
                }
            }
            continue;
        }

        let expected_file = expected
            .get(&raw_name)
            .with_context(|| format!("extra archive file not present in manifest: {raw_name}"))?;
        if !seen.insert(raw_name.clone()) {
            bail!("duplicate archive file entry: {raw_name}");
        }
        if entry.size() != expected_file.byte_len {
            bail!(
                "archive length mismatch for {}: expected {}, got {}",
                raw_name,
                expected_file.byte_len,
                entry.size()
            );
        }

        let output = destination.join(path_from_manifest(&raw_name)?);
        let parent = output
            .parent()
            .context("staged output file must have a parent")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("create staged parent {}", parent.display()))?;

        let mut out = File::create(&output)
            .with_context(|| format!("create staged file {}", output.display()))?;
        let mut digest = Sha256::new();
        let mut written = 0u64;
        let mut buffer = [0u8; 64 * 1024];

        loop {
            let read = entry
                .read(&mut buffer)
                .with_context(|| format!("read archive entry {raw_name}"))?;
            if read == 0 {
                break;
            }
            written = written
                .checked_add(read as u64)
                .context("staged file size overflow")?;
            if written > expected_file.byte_len {
                bail!("archive entry exceeded declared size: {raw_name}");
            }
            digest.update(&buffer[..read]);
            out.write_all(&buffer[..read])
                .with_context(|| format!("write staged file {}", output.display()))?;
        }
        out.sync_all()
            .with_context(|| format!("sync staged file {}", output.display()))?;

        if written != expected_file.byte_len {
            bail!(
                "staged file length mismatch for {}: expected {}, got {}",
                raw_name,
                expected_file.byte_len,
                written
            );
        }
        let actual = format!("{:x}", digest.finalize());
        if actual != expected_file.sha256.to_ascii_lowercase() {
            bail!("staged file SHA-256 mismatch: {raw_name}");
        }
        total_written = total_written
            .checked_add(written)
            .context("staged total byte count overflow")?;
    }

    if seen.len() as u64 != manifest.file_count {
        let missing: Vec<_> = expected
            .keys()
            .filter(|path| !seen.contains(*path))
            .cloned()
            .collect();
        bail!("archive is missing manifest files: {missing:?}");
    }
    if total_written != manifest.installed_tree_bytes {
        bail!(
            "staged total byte count mismatch: expected {}, got {}",
            manifest.installed_tree_bytes,
            total_written
        );
    }

    Ok(StagedTree {
        tree_sha256: tree_sha256(destination)?,
        file_count: seen.len() as u64,
        installed_tree_bytes: total_written,
    })
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid SHA-256 hex string");
    }
    Ok(())
}

fn validate_windows_safe_zip_name(raw: &str) -> Result<()> {
    if raw.contains('\') {
        bail!("ZIP entry must use forward slashes only: {raw:?}");
    }
    let normalized = raw.trim_end_matches('/');
    if normalized.is_empty() {
        return Ok(());
    }
    validate_windows_safe_relative_path(normalized)
}

fn validate_windows_safe_relative_path(path: &str) -> Result<()> {
    if path.is_empty() || path.starts_with('/') || path.ends_with('/') {
        bail!("unsafe relative path: {path:?}");
    }
    if path.contains('\') || path.contains(':') {
        bail!("unsafe Windows path syntax: {path:?}");
    }

    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            bail!("unsafe path component in {path:?}");
        }
        if !component.is_ascii() {
            bail!("update tree v1 paths must be ASCII: {path:?}");
        }
        if component.ends_with(' ') || component.ends_with('.') {
            bail!("Windows path component may not end in space/dot: {path:?}");
        }
        if component
            .bytes()
            .any(|byte| byte < 0x20 || matches!(byte, b'<' | b'>' | b'"' | b'|' | b'?' | b'*'))
        {
            bail!("unsafe Windows path characters: {path:?}");
        }

        let stem = component
            .split('.')
            .next()
            .unwrap_or(component)
            .to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if reserved {
            bail!("Windows reserved device name in update tree: {path:?}");
        }
    }
    Ok(())
}

fn path_from_manifest(path: &str) -> Result<PathBuf> {
    validate_windows_safe_relative_path(path)?;
    let mut output = PathBuf::new();
    for component in path.split('/') {
        output.push(component);
    }
    Ok(output)
}

fn is_zip_symlink<R: Read>(entry: &zip::read::ZipFile<'_, R>) -> bool {
    entry
        .unix_mode()
        .map(|mode| mode & 0o170000 == 0o120000)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn manifest(files: &[(&str, &[u8])]) -> ProductTreeManifest {
        ProductTreeManifest {
            schema_version: TREE_MANIFEST_SCHEMA_V1.into(),
            file_count: files.len() as u64,
            installed_tree_bytes: files.iter().map(|(_, bytes)| bytes.len() as u64).sum(),
            files: files
                .iter()
                .map(|(path, bytes)| TreeFile {
                    path: (*path).into(),
                    byte_len: bytes.len() as u64,
                    sha256: sha(bytes),
                })
                .collect(),
        }
    }

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn stages_exact_manifest_tree() {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("payload.zip");
        let files = [
            ("chaptera-reader.exe", b"reader-binary".as_slice()),
            ("resources/help.txt", b"help".as_slice()),
        ];
        write_zip(&archive, &files);
        let manifest = manifest(&files);
        let destination = temp.path().join("staging");

        let staged = stage_zip_payload(&archive, &destination, &manifest).unwrap();
        assert_eq!(staged.file_count, 2);
        assert_eq!(
            fs::read(destination.join("resources/help.txt")).unwrap(),
            b"help"
        );
    }

    #[test]
    fn rejects_case_insensitive_manifest_collision() {
        let files = [("A.dll", b"a".as_slice()), ("a.dll", b"b".as_slice())];
        assert!(manifest(&files).validate().is_err());
    }

    #[test]
    fn rejects_reserved_windows_device_name() {
        let files = [("CON.txt", b"x".as_slice())];
        assert!(manifest(&files).validate().is_err());
    }

    #[test]
    fn rejects_extra_archive_file_and_cleans_staging() {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("payload.zip");
        write_zip(
            &archive,
            &[
                ("chaptera-reader.exe", b"reader".as_slice()),
                ("extra.dll", b"extra".as_slice()),
            ],
        );
        let manifest = manifest(&[("chaptera-reader.exe", b"reader".as_slice())]);
        let destination = temp.path().join("staging");

        assert!(stage_zip_payload(&archive, &destination, &manifest).is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn rejects_tampered_file_bytes() {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("payload.zip");
        write_zip(&archive, &[("chaptera-reader.exe", b"evil".as_slice())]);
        let manifest = manifest(&[("chaptera-reader.exe", b"good".as_slice())]);
        let destination = temp.path().join("staging");

        assert!(stage_zip_payload(&archive, &destination, &manifest).is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn rejects_unsafe_path() {
        let files = [("../escape.dll", b"x".as_slice())];
        assert!(manifest(&files).validate().is_err());
    }
}
