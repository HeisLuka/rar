use crate::model::{JOURNAL_SCHEMA_VERSION, UpdateJournal};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
struct JournalEnvelope {
    journal: UpdateJournal,
    journal_sha256: String,
}

#[derive(Debug, Clone)]
pub struct JournalStore {
    root: PathBuf,
}

impl JournalStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn current(&self) -> PathBuf {
        self.root.join("update-journal.json")
    }

    fn next(&self) -> PathBuf {
        self.root.join("update-journal.json.next")
    }

    fn prev(&self) -> PathBuf {
        self.root.join("update-journal.json.prev")
    }

    pub fn persist(&self, journal: &mut UpdateJournal) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("create journal root {}", self.root.display()))?;

        let mut candidate = journal.clone();
        candidate.generation = candidate
            .generation
            .checked_add(1)
            .context("journal generation overflow")?;
        let encoded = encode_envelope(&candidate)?;

        let next = self.next();
        {
            let mut file = File::create(&next)
                .with_context(|| format!("create journal next {}", next.display()))?;
            file.write_all(&encoded)
                .with_context(|| format!("write journal next {}", next.display()))?;
            file.sync_all()
                .with_context(|| format!("sync journal next {}", next.display()))?;
        }

        let prev = self.prev();
        if prev.exists() {
            fs::remove_file(&prev)
                .with_context(|| format!("remove stale journal prev {}", prev.display()))?;
        }

        let current = self.current();
        if current.exists() {
            fs::rename(&current, &prev).with_context(|| {
                format!(
                    "rotate journal current {} -> {}",
                    current.display(),
                    prev.display()
                )
            })?;
        }

        fs::rename(&next, &current).with_context(|| {
            format!(
                "promote journal next {} -> {}",
                next.display(),
                current.display()
            )
        })?;

        sync_parent_best_effort(&self.root)?;
        *journal = candidate;
        Ok(())
    }

    pub fn load_latest(&self) -> Result<Option<UpdateJournal>> {
        let paths = [self.current(), self.next(), self.prev()];
        let mut existing = 0usize;
        let mut valid = Vec::new();

        for path in paths {
            if !path.exists() {
                continue;
            }
            existing += 1;
            match decode_envelope(&path) {
                Ok(envelope) => valid.push((path, envelope)),
                Err(_) => {}
            }
        }

        if existing == 0 {
            return Ok(None);
        }
        if valid.is_empty() {
            bail!("all Chaptera updater journal copies are invalid");
        }

        valid.sort_by(|left, right| {
            right
                .1
                .journal
                .generation
                .cmp(&left.1.journal.generation)
        });

        if valid.len() > 1
            && valid[0].1.journal.generation == valid[1].1.journal.generation
            && valid[0].1.journal_sha256 != valid[1].1.journal_sha256
        {
            bail!(
                "ambiguous Chaptera updater journals at generation {}",
                valid[0].1.journal.generation
            );
        }

        Ok(Some(valid.remove(0).1.journal))
    }
}

fn encode_envelope(journal: &UpdateJournal) -> Result<Vec<u8>> {
    if journal.schema_version != JOURNAL_SCHEMA_VERSION {
        bail!(
            "unsupported Chaptera update journal schema {}",
            journal.schema_version
        );
    }
    let canonical = serde_json::to_vec(journal).context("serialize update journal payload")?;
    let journal_sha256 = format!("{:x}", Sha256::digest(&canonical));
    let envelope = JournalEnvelope {
        journal: journal.clone(),
        journal_sha256,
    };
    serde_json::to_vec_pretty(&envelope).context("serialize update journal envelope")
}

fn decode_envelope(path: &Path) -> Result<JournalEnvelope> {
    let bytes = fs::read(path).with_context(|| format!("read journal {}", path.display()))?;
    let envelope: JournalEnvelope =
        serde_json::from_slice(&bytes).with_context(|| format!("parse journal {}", path.display()))?;
    if envelope.journal.schema_version != JOURNAL_SCHEMA_VERSION {
        bail!(
            "unsupported Chaptera update journal schema {}",
            envelope.journal.schema_version
        );
    }
    let canonical =
        serde_json::to_vec(&envelope.journal).context("serialize journal for digest verification")?;
    let actual = format!("{:x}", Sha256::digest(&canonical));
    if actual != envelope.journal_sha256 {
        bail!("journal digest mismatch at {}", path.display());
    }
    Ok(envelope)
}

#[cfg(unix)]
fn sync_parent_best_effort(root: &Path) -> Result<()> {
    File::open(root)
        .with_context(|| format!("open journal directory {}", root.display()))?
        .sync_all()
        .with_context(|| format!("sync journal directory {}", root.display()))
}

#[cfg(not(unix))]
fn sync_parent_best_effort(_root: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UpdateJournal, UpdatePhase};
    use tempfile::tempdir;

    fn journal() -> UpdateJournal {
        UpdateJournal::new_payload_swap(
            "chaptera.reader",
            "windows-x86_64",
            "stable",
            "0.1.0",
            "0.2.0",
            1,
            1,
            "11".repeat(32),
            "22".repeat(32),
            true,
            "reader-state-v1",
        )
    }

    #[test]
    fn round_trips_and_keeps_previous_generation() {
        let dir = tempdir().unwrap();
        let store = JournalStore::new(dir.path());
        let mut value = journal();

        store.persist(&mut value).unwrap();
        assert_eq!(value.generation, 1);
        value.advance(UpdatePhase::OriginExited);
        store.persist(&mut value).unwrap();
        assert_eq!(value.generation, 2);

        let loaded = store.load_latest().unwrap().unwrap();
        assert_eq!(loaded.generation, 2);
        assert_eq!(loaded.phase, UpdatePhase::OriginExited);
        assert!(dir.path().join("update-journal.json.prev").is_file());
    }

    #[test]
    fn ignores_torn_next_when_current_is_valid() {
        let dir = tempdir().unwrap();
        let store = JournalStore::new(dir.path());
        let mut value = journal();
        store.persist(&mut value).unwrap();

        fs::write(dir.path().join("update-journal.json.next"), b"{torn").unwrap();
        let loaded = store.load_latest().unwrap().unwrap();
        assert_eq!(loaded.generation, 1);
    }

    #[test]
    fn recovers_from_missing_current_using_valid_next() {
        let dir = tempdir().unwrap();
        let store = JournalStore::new(dir.path());
        let mut value = journal();
        store.persist(&mut value).unwrap();

        value.advance(UpdatePhase::OriginExited);
        let mut next_value = value.clone();
        next_value.generation += 1;
        fs::write(
            dir.path().join("update-journal.json.next"),
            encode_envelope(&next_value).unwrap(),
        )
        .unwrap();
        fs::rename(
            dir.path().join("update-journal.json"),
            dir.path().join("update-journal.json.prev"),
        )
        .unwrap();

        let loaded = store.load_latest().unwrap().unwrap();
        assert_eq!(loaded.generation, 2);
        assert_eq!(loaded.phase, UpdatePhase::OriginExited);
    }
}
