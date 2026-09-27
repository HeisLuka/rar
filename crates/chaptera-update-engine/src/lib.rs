pub mod staging;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

pub const JOURNAL_SCHEMA_VERSION: &str = "chaptera.update-journal.v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Preparing,
    Prepared,
    PreviousRetained,
    CandidateActivated,
    CandidateConfirmed,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateJournal {
    pub schema_version: String,
    pub transaction_id: String,
    pub candidate_version: String,
    pub updater_relative_path: PathBuf,
    pub phase: UpdatePhase,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct JournalEnvelope {
    generation: u64,
    journal: UpdateJournal,
    journal_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionPaths {
    pub staging_transaction: PathBuf,
    pub staged_candidate: PathBuf,
    pub control_updater: PathBuf,
    pub rollback_transaction: PathBuf,
    pub previous_tree: PathBuf,
    pub rejected_candidate: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryOutcome {
    NothingToDo,
    PreparedTransactionAborted,
    UnconfirmedCandidateRolledBack,
    ConfirmedCandidateRetained,
    RolledBackTransactionFinalized,
}

#[derive(Debug)]
pub enum UpdateError {
    Io(io::Error),
    Json(serde_json::Error),
    InvalidTransactionId(String),
    InvalidUpdaterPath(PathBuf),
    ActiveTransaction(String),
    UnexpectedPhase {
        expected: UpdatePhase,
        actual: UpdatePhase,
    },
    LayoutInvariant(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Json(err) => write!(f, "journal JSON error: {err}"),
            Self::InvalidTransactionId(id) => write!(f, "invalid transaction id: {id}"),
            Self::InvalidUpdaterPath(path) => {
                write!(f, "updater path must be a safe relative path: {}", path.display())
            }
            Self::ActiveTransaction(id) => write!(f, "active update transaction already exists: {id}"),
            Self::UnexpectedPhase { expected, actual } => {
                write!(f, "unexpected update phase: expected {expected:?}, got {actual:?}")
            }
            Self::LayoutInvariant(message) => write!(f, "install-layout invariant failed: {message}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<io::Error> for UpdateError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for UpdateError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub type Result<T> = std::result::Result<T, UpdateError>;

#[derive(Debug, Clone)]
pub struct UpdateEngine {
    root: PathBuf,
}

impl UpdateEngine {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn current_dir(&self) -> PathBuf {
        self.root.join("current")
    }

    pub fn journal_path(&self) -> PathBuf {
        self.root.join("update-journal.json")
    }

    pub fn journal_next_path(&self) -> PathBuf {
        self.root.join("update-journal.json.next")
    }

    pub fn journal_previous_path(&self) -> PathBuf {
        self.root.join("update-journal.json.prev")
    }

    pub fn paths_for(&self, transaction_id: &str, updater_relative_path: &Path) -> Result<TransactionPaths> {
        validate_transaction_id(transaction_id)?;
        validate_relative_path(updater_relative_path)?;

        let staging_transaction = self.root.join(".staging").join(transaction_id);
        let rollback_transaction = self.root.join(".rollback").join(transaction_id);
        Ok(TransactionPaths {
            staged_candidate: staging_transaction.join("candidate"),
            control_updater: staging_transaction.join("control").join(updater_relative_path),
            previous_tree: rollback_transaction.join("previous"),
            rejected_candidate: staging_transaction.join("rejected-current"),
            staging_transaction,
            rollback_transaction,
        })
    }

    pub fn read_journal(&self) -> Result<Option<UpdateJournal>> {
        let candidates = self.read_journal_candidates()?;
        if candidates.is_empty() {
            return Ok(None);
        }

        let highest = candidates.iter().map(|entry| entry.generation).max().unwrap_or(0);
        let mut winners = candidates.iter().filter(|entry| entry.generation == highest);
        let winner = winners.next().expect("highest generation has a candidate");
        if winners.any(|other| other.journal_sha256 != winner.journal_sha256) {
            return Err(UpdateError::LayoutInvariant(format!(
                "ambiguous update journals at generation {highest}"
            )));
        }
        validate_journal(&winner.journal)?;
        Ok(Some(winner.journal.clone()))
    }

    fn read_journal_candidates(&self) -> Result<Vec<JournalEnvelope>> {
        let mut valid = Vec::new();
        let mut existing = 0usize;
        for path in [self.journal_path(), self.journal_next_path(), self.journal_previous_path()] {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
            };
            existing += 1;
            if let Ok(envelope) = decode_journal_envelope(&bytes) {
                valid.push(envelope);
                continue;
            }
            if let Ok(journal) = serde_json::from_slice::<UpdateJournal>(&bytes) {
                if validate_journal(&journal).is_ok() {
                    let journal_sha256 = journal_digest(&journal)?;
                    valid.push(JournalEnvelope { generation: 0, journal, journal_sha256 });
                }
            }
        }
        if existing != 0 && valid.is_empty() {
            return Err(UpdateError::LayoutInvariant(
                "all updater journal copies are invalid".into(),
            ));
        }
        Ok(valid)
    }

    /// Removes transaction directories left by a terminal updater process.
    ///
    /// A copied control updater executes from .staging/<tx>/control. Windows
    /// cannot reliably delete a running executable, so terminal confirmation
    /// and rollback deliberately leave staging cleanup to the next owner after
    /// the control process exits. Call this only while holding the install lock.
    pub fn cleanup_orphaned_transactions(&self) -> Result<usize> {
        if let Some(active) = self.read_journal()? {
            return Err(UpdateError::ActiveTransaction(active.transaction_id));
        }

        let mut removed = 0usize;
        for root in [self.root.join(".staging"), self.root.join(".rollback")] {
            let entries = match fs::read_dir(&root) {
                Ok(entries) => entries,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
            };
            for entry in entries {
                let path = entry?.path();
                remove_path_if_exists(&path)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Begins a transaction from a candidate tree that the caller has already
    /// authenticated and policy-checked. The current updater is copied to a
    /// transaction-local control path before any active-tree rename occurs.
    pub fn begin_verified_candidate(
        &self,
        transaction_id: &str,
        candidate_version: &str,
        candidate_source: &Path,
        updater_relative_path: &Path,
    ) -> Result<PathBuf> {
        validate_transaction_id(transaction_id)?;
        validate_relative_path(updater_relative_path)?;
        if self.read_journal()?.is_some() {
            let active = self.read_journal()?.expect("checked above");
            return Err(UpdateError::ActiveTransaction(active.transaction_id));
        }
        if !candidate_source.is_dir() {
            return Err(UpdateError::LayoutInvariant(format!(
                "verified candidate source is not a directory: {}",
                candidate_source.display()
            )));
        }

        let current = self.current_dir();
        if !current.is_dir() {
            return Err(UpdateError::LayoutInvariant(format!(
                "current tree missing: {}",
                current.display()
            )));
        }
        let current_updater = current.join(updater_relative_path);
        if !current_updater.is_file() {
            return Err(UpdateError::LayoutInvariant(format!(
                "current updater missing: {}",
                current_updater.display()
            )));
        }

        fs::create_dir_all(self.root.join(".staging"))?;
        fs::create_dir_all(self.root.join(".rollback"))?;
        let paths = self.paths_for(transaction_id, updater_relative_path)?;
        if paths.staging_transaction.exists() || paths.rollback_transaction.exists() {
            return Err(UpdateError::LayoutInvariant(format!(
                "transaction paths already exist for {transaction_id}"
            )));
        }

        let mut journal = UpdateJournal {
            schema_version: JOURNAL_SCHEMA_VERSION.to_owned(),
            transaction_id: transaction_id.to_owned(),
            candidate_version: candidate_version.to_owned(),
            updater_relative_path: updater_relative_path.to_path_buf(),
            phase: UpdatePhase::Preparing,
        };
        self.write_journal(&journal)?;

        fs::create_dir_all(&paths.staging_transaction)?;
        copy_tree(candidate_source, &paths.staged_candidate)?;
        if let Some(parent) = paths.control_updater.parent() {
            fs::create_dir_all(parent)?;
        }
        copy_file_synced(&current_updater, &paths.control_updater)?;

        journal.phase = UpdatePhase::Prepared;
        self.write_journal(&journal)?;
        Ok(paths.control_updater)
    }

    pub fn retain_previous(&self) -> Result<()> {
        let mut journal = self.require_phase(UpdatePhase::Prepared)?;
        let paths = self.paths_for(&journal.transaction_id, &journal.updater_relative_path)?;
        let current = self.current_dir();

        if !current.is_dir() {
            return Err(UpdateError::LayoutInvariant("current tree missing before retain".into()));
        }
        if paths.previous_tree.exists() {
            return Err(UpdateError::LayoutInvariant("rollback previous tree already exists".into()));
        }
        fs::create_dir_all(&paths.rollback_transaction)?;
        rename_path(&current, &paths.previous_tree)?;

        journal.phase = UpdatePhase::PreviousRetained;
        self.write_journal(&journal)?;
        Ok(())
    }

    pub fn activate_candidate(&self) -> Result<()> {
        let mut journal = self.require_phase(UpdatePhase::PreviousRetained)?;
        let paths = self.paths_for(&journal.transaction_id, &journal.updater_relative_path)?;
        let current = self.current_dir();

        if current.exists() {
            return Err(UpdateError::LayoutInvariant(
                "current must be absent between retain and activation".into(),
            ));
        }
        if !paths.staged_candidate.is_dir() {
            return Err(UpdateError::LayoutInvariant("staged candidate tree missing".into()));
        }

        rename_path(&paths.staged_candidate, &current)?;
        journal.phase = UpdatePhase::CandidateActivated;
        self.write_journal(&journal)?;
        Ok(())
    }

    pub fn confirm_candidate(&self) -> Result<()> {
        let mut journal = self.require_phase(UpdatePhase::CandidateActivated)?;
        let paths = self.paths_for(&journal.transaction_id, &journal.updater_relative_path)?;

        journal.phase = UpdatePhase::CandidateConfirmed;
        self.write_journal(&journal)?;

        remove_path_if_exists(&paths.rollback_transaction)?;
        // Keep .staging/<tx>/control until the copied U1 process exits. The
        // next exclusive lock owner removes this terminal transaction tree.
        self.archive_terminal_journal()?;
        Ok(())
    }

    /// Recovers conservatively. Any candidate that was activated but not
    /// durably confirmed is rolled back to the retained previous tree.
    pub fn recover(&self) -> Result<RecoveryOutcome> {
        let Some(mut journal) = self.read_journal()? else {
            remove_path_if_exists(&self.journal_next_path())?;
            return Ok(RecoveryOutcome::NothingToDo);
        };
        let paths = self.paths_for(&journal.transaction_id, &journal.updater_relative_path)?;
        let current = self.current_dir();

        match journal.phase {
            UpdatePhase::Preparing | UpdatePhase::Prepared => {
                if paths.previous_tree.exists() {
                    if current.exists() {
                        return Err(UpdateError::LayoutInvariant(
                            "both current and retained previous tree exist in pre-activation phase".into(),
                        ));
                    }
                    rename_path(&paths.previous_tree, &current)?;
                } else if !current.is_dir() {
                    return Err(UpdateError::LayoutInvariant(
                        "pre-activation recovery found neither current nor retained previous tree".into(),
                    ));
                }

                journal.phase = UpdatePhase::RolledBack;
                self.write_journal(&journal)?;
                remove_path_if_exists(&paths.rollback_transaction)?;
                self.archive_terminal_journal()?;
                Ok(RecoveryOutcome::PreparedTransactionAborted)
            }
            UpdatePhase::PreviousRetained | UpdatePhase::CandidateActivated => {
                if paths.previous_tree.exists() {
                    if current.exists() {
                        remove_path_if_exists(&paths.rejected_candidate)?;
                        if let Some(parent) = paths.rejected_candidate.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        rename_path(&current, &paths.rejected_candidate)?;
                    }
                    rename_path(&paths.previous_tree, &current)?;
                } else if !(current.is_dir() && paths.rejected_candidate.exists()) {
                    return Err(UpdateError::LayoutInvariant(
                        "unconfirmed recovery cannot identify a retained previous tree".into(),
                    ));
                }

                journal.phase = UpdatePhase::RolledBack;
                self.write_journal(&journal)?;
                remove_path_if_exists(&paths.rollback_transaction)?;
                self.archive_terminal_journal()?;
                Ok(RecoveryOutcome::UnconfirmedCandidateRolledBack)
            }
            UpdatePhase::CandidateConfirmed => {
                if !current.is_dir() {
                    return Err(UpdateError::LayoutInvariant(
                        "confirmed candidate current tree is missing".into(),
                    ));
                }
                remove_path_if_exists(&paths.rollback_transaction)?;
                self.archive_terminal_journal()?;
                Ok(RecoveryOutcome::ConfirmedCandidateRetained)
            }
            UpdatePhase::RolledBack => {
                if !current.is_dir() {
                    return Err(UpdateError::LayoutInvariant(
                        "rolled-back current tree is missing".into(),
                    ));
                }
                remove_path_if_exists(&paths.rollback_transaction)?;
                self.archive_terminal_journal()?;
                Ok(RecoveryOutcome::RolledBackTransactionFinalized)
            }
        }
    }

    fn require_phase(&self, expected: UpdatePhase) -> Result<UpdateJournal> {
        let journal = self
            .read_journal()?
            .ok_or_else(|| UpdateError::LayoutInvariant("active update journal is missing".into()))?;
        if journal.phase != expected {
            return Err(UpdateError::UnexpectedPhase {
                expected,
                actual: journal.phase,
            });
        }
        Ok(journal)
    }

    fn write_journal(&self, journal: &UpdateJournal) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        let canonical = self.journal_path();
        let next = self.journal_next_path();
        let previous = self.journal_previous_path();

        remove_path_if_exists(&next)?;
        let generation = self
            .read_journal_candidates()?
            .iter()
            .map(|entry| entry.generation)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| UpdateError::LayoutInvariant("journal generation overflow".into()))?;
        let envelope = JournalEnvelope {
            generation,
            journal: journal.clone(),
            journal_sha256: journal_digest(journal)?,
        };
        let bytes = serde_json::to_vec_pretty(&envelope)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&next)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);

        if canonical.exists() {
            remove_path_if_exists(&previous)?;
            rename_path(&canonical, &previous)?;
        }
        rename_path(&next, &canonical)?;
        sync_directory_if_supported(&self.root)?;
        Ok(())
    }

    fn archive_terminal_journal(&self) -> Result<()> {
        let canonical = self.journal_path();
        if canonical.exists() {
            let previous = self.journal_previous_path();
            remove_path_if_exists(&previous)?;
            rename_path(&canonical, &previous)?;
            sync_directory_if_supported(&self.root)?;
        }
        remove_path_if_exists(&self.journal_next_path())?;
        Ok(())
    }
}

fn validate_journal(journal: &UpdateJournal) -> Result<()> {
    if journal.schema_version != JOURNAL_SCHEMA_VERSION {
        return Err(UpdateError::LayoutInvariant(format!(
            "unsupported journal schema {}",
            journal.schema_version
        )));
    }
    validate_transaction_id(&journal.transaction_id)?;
    validate_relative_path(&journal.updater_relative_path)?;
    Ok(())
}

fn journal_digest(journal: &UpdateJournal) -> Result<String> {
    let canonical = serde_json::to_vec(journal)?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

fn decode_journal_envelope(bytes: &[u8]) -> Result<JournalEnvelope> {
    let envelope: JournalEnvelope = serde_json::from_slice(bytes)?;
    validate_journal(&envelope.journal)?;
    let actual = journal_digest(&envelope.journal)?;
    if actual != envelope.journal_sha256 {
        return Err(UpdateError::LayoutInvariant("journal digest mismatch".into()));
    }
    Ok(envelope)
}

fn validate_transaction_id(transaction_id: &str) -> Result<()> {
    let valid = !transaction_id.is_empty()
        && transaction_id.len() <= 96
        && transaction_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(UpdateError::InvalidTransactionId(transaction_id.to_owned()))
    }
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(UpdateError::InvalidUpdaterPath(path.to_path_buf()));
    }
    if path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        Ok(())
    } else {
        Err(UpdateError::InvalidUpdaterPath(path.to_path_buf()))
    }
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(src)?;
    if metadata.file_type().is_symlink() {
        return Err(UpdateError::LayoutInvariant(format!(
            "candidate tree contains symlink: {}",
            src.display()
        )));
    }
    if metadata.is_file() {
        copy_file_synced(src, dst)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(UpdateError::LayoutInvariant(format!(
            "unsupported candidate entry: {}",
            src.display()
        )));
    }

    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
    }
    Ok(())
}

fn copy_file_synced(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)?;
    OpenOptions::new().write(true).open(dst)?.sync_all()?;
    Ok(())
}

fn rename_path(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(src, dst)?;
    if let Some(parent) = dst.parent() {
        sync_directory_if_supported(parent)?;
    }
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
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
