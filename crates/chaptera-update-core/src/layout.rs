use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub struct MutationLock {
    _file: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceBudget {
    pub required_bytes: u64,
    pub available_bytes: u64,
}

impl SpaceBudget {
    pub fn is_sufficient(self) -> bool {
        self.available_bytes >= self.required_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallLayout {
    root: PathBuf,
}

impl InstallLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn current(&self) -> PathBuf {
        self.root.join("current")
    }

    pub fn staging_root(&self) -> PathBuf {
        self.root.join(".staging")
    }

    pub fn rollback_root(&self) -> PathBuf {
        self.root.join(".rollback")
    }

    pub fn journal(&self) -> PathBuf {
        self.root.join("update-journal.json")
    }

    pub fn journal_next(&self) -> PathBuf {
        self.root.join("update-journal.json.next")
    }

    pub fn journal_prev(&self) -> PathBuf {
        self.root.join("update-journal.json.prev")
    }

    pub fn mutation_lock(&self) -> PathBuf {
        self.root.join(".chaptera-install.lock")
    }

    pub fn staging_for(&self, attempt_id: &str) -> Result<PathBuf> {
        validate_component(attempt_id)?;
        Ok(self.staging_root().join(attempt_id))
    }

    pub fn rollback_for(&self, version: &str, attempt_id: &str) -> Result<PathBuf> {
        validate_component(version)?;
        validate_component(attempt_id)?;
        Ok(self
            .rollback_root()
            .join(format!("{version}-{attempt_id}")))
    }

    pub fn failed_candidate_for(&self, attempt_id: &str) -> Result<PathBuf> {
        validate_component(attempt_id)?;
        Ok(self.staging_root().join(format!("{attempt_id}-failed")))
    }

    pub fn acquire_mutation_lock(&self) -> Result<MutationLock> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("create install root {}", self.root.display()))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(self.mutation_lock())
            .context("open Chaptera install-root mutation lock")?;
        match file.try_lock() {
            Ok(()) => Ok(MutationLock { _file: file }),
            Err(TryLockError::WouldBlock) => {
                bail!("another Chaptera install/update/repair/uninstall mutation owns this root")
            }
            Err(TryLockError::Error(error)) => Err(error)
                .context("acquire Chaptera install-root mutation lock"),
        }
    }

    pub fn space_budget(
        &self,
        compressed_target_bytes: u64,
        extracted_tree_bytes: u64,
        temp_overhead_bytes: u64,
        retained_predecessor_bytes: u64,
    ) -> Result<SpaceBudget> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("create install root {}", self.root.display()))?;
        let required_bytes = compressed_target_bytes
            .saturating_add(extracted_tree_bytes)
            .saturating_add(temp_overhead_bytes)
            .saturating_add(retained_predecessor_bytes);
        let available_bytes = fs4::available_space(&self.root)
            .with_context(|| format!("query available space for {}", self.root.display()))?;
        Ok(SpaceBudget {
            required_bytes,
            available_bytes,
        })
    }
}

fn validate_component(value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("path component must not be empty");
    }
    if value == "." || value == ".." {
        bail!("unsafe path component");
    }
    if value
        .chars()
        .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_')))
    {
        bail!("unsafe path component: {value:?}");
    }
    Ok(())
}

#[derive(Debug)]
enum TreeEntry {
    Directory { relative: String },
    File {
        relative: String,
        absolute: PathBuf,
        len: u64,
    },
}

impl TreeEntry {
    fn relative(&self) -> &str {
        match self {
            Self::Directory { relative } | Self::File { relative, .. } => relative,
        }
    }
}

pub fn tree_sha256(root: &Path) -> Result<String> {
    if !root.is_dir() {
        bail!("tree root is not a directory: {}", root.display());
    }

    let mut entries = Vec::new();
    collect_tree_entries(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.relative().cmp(right.relative()));

    let mut digest = Sha256::new();
    for entry in entries {
        match entry {
            TreeEntry::Directory { relative } => {
                digest.update(b"D\0");
                digest.update(relative.as_bytes());
                digest.update(b"\0");
            }
            TreeEntry::File {
                relative,
                absolute,
                len,
            } => {
                digest.update(b"F\0");
                digest.update(relative.as_bytes());
                digest.update(b"\0");
                digest.update(len.to_le_bytes());
                digest.update(b"\0");

                let mut file = File::open(&absolute)
                    .with_context(|| format!("open tree file {}", absolute.display()))?;
                let mut buffer = [0u8; 64 * 1024];
                loop {
                    let read = file
                        .read(&mut buffer)
                        .with_context(|| format!("read tree file {}", absolute.display()))?;
                    if read == 0 {
                        break;
                    }
                    digest.update(&buffer[..read]);
                }
                digest.update(b"\0");
            }
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn collect_tree_entries(root: &Path, current: &Path, out: &mut Vec<TreeEntry>) -> Result<()> {
    let mut children = fs::read_dir(current)
        .with_context(|| format!("read tree directory {}", current.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("enumerate tree directory {}", current.display()))?;
    children.sort_by_key(|entry| entry.file_name());

    for entry in children {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("inspect tree entry {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("symlinks are forbidden in Chaptera update trees: {}", path.display());
        }
        let relative = normalize_relative(root, &path)?;
        if metadata.is_dir() {
            out.push(TreeEntry::Directory {
                relative: relative.clone(),
            });
            collect_tree_entries(root, &path, out)?;
        } else if metadata.is_file() {
            out.push(TreeEntry::File {
                relative,
                absolute: path,
                len: metadata.len(),
            });
        } else {
            bail!("unsupported update-tree entry type: {}", path.display());
        }
    }
    Ok(())
}

fn normalize_relative(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{} is outside {}", path.display(), root.display()))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(os_component(value)),
            _ => bail!("unsafe relative path in update tree: {}", path.display()),
        }
    }
    Ok(parts.join("/"))
}

fn os_component(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn tree_digest_is_order_independent_and_content_sensitive() {
        let a = tempdir().unwrap();
        fs::create_dir_all(a.path().join("nested")).unwrap();
        fs::write(a.path().join("b.txt"), b"b").unwrap();
        fs::write(a.path().join("nested/a.txt"), b"a").unwrap();
        let first = tree_sha256(a.path()).unwrap();

        let b = tempdir().unwrap();
        fs::create_dir_all(b.path().join("nested")).unwrap();
        fs::write(b.path().join("nested/a.txt"), b"a").unwrap();
        fs::write(b.path().join("b.txt"), b"b").unwrap();
        let second = tree_sha256(b.path()).unwrap();
        assert_eq!(first, second);

        fs::write(b.path().join("b.txt"), b"changed").unwrap();
        assert_ne!(first, tree_sha256(b.path()).unwrap());
    }

    #[test]
    fn rejects_unsafe_attempt_component() {
        let layout = InstallLayout::new("unused");
        assert!(layout.staging_for("../escape").is_err());
        assert!(layout.staging_for("attempt-01").is_ok());
    }

    #[test]
    fn mutation_lock_is_exclusive() {
        let dir = tempdir().unwrap();
        let layout = InstallLayout::new(dir.path());
        let _first = layout.acquire_mutation_lock().unwrap();
        assert!(layout.acquire_mutation_lock().is_err());
    }
}
