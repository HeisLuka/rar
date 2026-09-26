use chaptera_update_engine::{RecoveryOutcome, UpdateEngine, UpdateError};
use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const INSTALL_LOCK_FILENAME: &str = ".chaptera-install.lock";

#[derive(Debug)]
pub struct InstallLock {
    file: File,
    path: PathBuf,
}

impl InstallLock {
    /// Blocking acquisition for a copied control updater. The control process
    /// can start while its parent still owns the lock; it cannot mutate the
    /// install until the parent's file handle is closed or explicitly unlocked.
    pub fn acquire(root: &Path) -> Result<Self> {
        let (file, path) = Self::open_file(root)?;
        file.lock()?;
        Self::finish_acquire(file, path)
    }

    /// Non-blocking acquisition used by an interactive/front-door updater.
    pub fn try_acquire(root: &Path) -> Result<Self> {
        let (file, path) = Self::open_file(root)?;

        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(OrchestrationError::LockBusy),
            Err(TryLockError::Error(err)) => return Err(err.into()),
        }

        Self::finish_acquire(file, path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn open_file(root: &Path) -> Result<(File, PathBuf)> {
        std::fs::create_dir_all(root)?;
        let path = root.join(INSTALL_LOCK_FILENAME);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)?;
        Ok((file, path))
    }

    fn finish_acquire(mut file: File, path: PathBuf) -> Result<Self> {
        // The file may survive a previous crash; the OS lock, not existence,
        // is authoritative. Rewrite diagnostic metadata only after locking.
        file.set_len(0)?;
        file.write_all(format!("pid={}\n", std::process::id()).as_bytes())?;
        file.sync_all()?;
        Ok(Self { file, path })
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub trait UpdateHooks {
    /// Stop or detach processes that can keep current-tree binaries open.
    /// The path points at the copied U1 control updater, not candidate U2.
    fn quiesce(&mut self, control_updater: &Path) -> std::result::Result<(), String>;

    /// Validate the activated candidate while U1 still owns the transaction.
    fn health_check(&mut self, current_tree: &Path) -> std::result::Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    Confirmed {
        candidate_version: String,
        startup_recovery: RecoveryOutcome,
    },
    RolledBack {
        candidate_version: String,
        reason: String,
        startup_recovery: RecoveryOutcome,
    },
}

#[derive(Debug)]
pub enum OrchestrationError {
    Io(io::Error),
    Engine(UpdateError),
    LockBusy,
    QuiesceFailed {
        reason: String,
    },
    RecoveryFailed {
        context: &'static str,
        primary: String,
        recovery: String,
    },
    EngineStepFailed {
        step: &'static str,
        primary: String,
        recovery: RecoveryOutcome,
    },
}

impl fmt::Display for OrchestrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Engine(err) => write!(f, "update engine error: {err}"),
            Self::LockBusy => write!(f, "another Chaptera update owns the install lock"),
            Self::QuiesceFailed { reason } => write!(f, "quiesce failed: {reason}"),
            Self::RecoveryFailed {
                context,
                primary,
                recovery,
            } => write!(
                f,
                "{context} failed ({primary}) and recovery also failed ({recovery})"
            ),
            Self::EngineStepFailed {
                step,
                primary,
                recovery,
            } => write!(
                f,
                "update step {step} failed ({primary}); recovery outcome: {recovery:?}"
            ),
        }
    }
}

impl std::error::Error for OrchestrationError {}

impl From<io::Error> for OrchestrationError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<UpdateError> for OrchestrationError {
    fn from(value: UpdateError) -> Self {
        Self::Engine(value)
    }
}

pub type Result<T> = std::result::Result<T, OrchestrationError>;

#[derive(Debug, Clone)]
pub struct UpdateOrchestrator {
    engine: UpdateEngine,
}

impl UpdateOrchestrator {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            engine: UpdateEngine::new(root),
        }
    }

    pub fn engine(&self) -> &UpdateEngine {
        &self.engine
    }

    pub fn apply_verified_candidate<H: UpdateHooks>(
        &self,
        transaction_id: &str,
        candidate_version: &str,
        candidate_source: &Path,
        updater_relative_path: &Path,
        hooks: &mut H,
    ) -> Result<ApplyOutcome> {
        let _lock = InstallLock::try_acquire(self.engine.root())?;

        // Crash leftovers are repaired while the exclusive OS lock is held,
        // before a new transaction can stage or switch anything.
        let startup_recovery = self.engine.recover()?;

        let control_updater = self.engine.begin_verified_candidate(
            transaction_id,
            candidate_version,
            candidate_source,
            updater_relative_path,
        )?;

        if let Err(reason) = hooks.quiesce(&control_updater) {
            self.recover_or_combine("quiesce", &reason)?;
            return Err(OrchestrationError::QuiesceFailed { reason });
        }

        self.engine_step("retain_previous", self.engine.retain_previous())?;
        self.engine_step("activate_candidate", self.engine.activate_candidate())?;

        if let Err(reason) = hooks.health_check(&self.engine.current_dir()) {
            match self.engine.recover() {
                Ok(_) => {
                    return Ok(ApplyOutcome::RolledBack {
                        candidate_version: candidate_version.to_owned(),
                        reason,
                        startup_recovery,
                    });
                }
                Err(recovery) => {
                    return Err(OrchestrationError::RecoveryFailed {
                        context: "health check",
                        primary: reason,
                        recovery: recovery.to_string(),
                    });
                }
            }
        }

        self.engine_step("confirm_candidate", self.engine.confirm_candidate())?;

        Ok(ApplyOutcome::Confirmed {
            candidate_version: candidate_version.to_owned(),
            startup_recovery,
        })
    }

    fn engine_step(
        &self,
        step: &'static str,
        result: chaptera_update_engine::Result<()>,
    ) -> Result<()> {
        match result {
            Ok(()) => Ok(()),
            Err(primary) => match self.engine.recover() {
                Ok(recovery) => Err(OrchestrationError::EngineStepFailed {
                    step,
                    primary: primary.to_string(),
                    recovery,
                }),
                Err(recovery) => Err(OrchestrationError::RecoveryFailed {
                    context: step,
                    primary: primary.to_string(),
                    recovery: recovery.to_string(),
                }),
            },
        }
    }

    fn recover_or_combine(&self, context: &'static str, primary: &str) -> Result<RecoveryOutcome> {
        self.engine
            .recover()
            .map_err(|recovery| OrchestrationError::RecoveryFailed {
                context,
                primary: primary.to_owned(),
                recovery: recovery.to_string(),
            })
    }
}
