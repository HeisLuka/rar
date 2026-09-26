use chaptera_update_engine::{UpdateEngine, UpdateError, UpdatePhase};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

pub const CONTROL_REQUEST_SCHEMA_VERSION: &str = "chaptera.update-control-request.v1";
pub const CONTROL_RECEIPT_SCHEMA_VERSION: &str = "chaptera.update-control-receipt.v1";
pub const CONTROL_MODE_ARG: &str = "--chaptera-update-control";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlHandoffRequest {
    pub schema_version: String,
    pub install_root: PathBuf,
    pub transaction_id: String,
    pub candidate_version: String,
    pub updater_relative_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlReceipt {
    pub schema_version: String,
    pub transaction_id: String,
    pub pid: u32,
    pub executable: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedControlHandoff {
    pub control_updater: PathBuf,
    pub request_path: PathBuf,
}

impl PreparedControlHandoff {
    pub fn spawn(&self) -> Result<Child> {
        if !self.control_updater.is_file() {
            return Err(HandoffError::ControlUpdaterMissing(
                self.control_updater.clone(),
            ));
        }

        Command::new(&self.control_updater)
            .arg(CONTROL_MODE_ARG)
            .arg(&self.request_path)
            .spawn()
            .map_err(HandoffError::Io)
    }
}

#[derive(Debug)]
pub enum HandoffError {
    Io(io::Error),
    Json(serde_json::Error),
    Engine(UpdateError),
    JournalMissing,
    JournalNotPrepared(UpdatePhase),
    Schema(String),
    Mismatch(String),
    ControlUpdaterMissing(PathBuf),
    RequestAlreadyExists(PathBuf),
}

impl fmt::Display for HandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Json(err) => write!(f, "handoff JSON error: {err}"),
            Self::Engine(err) => write!(f, "update engine error: {err}"),
            Self::JournalMissing => write!(f, "active update journal is missing"),
            Self::JournalNotPrepared(phase) => {
                write!(f, "control handoff requires Prepared journal, got {phase:?}")
            }
            Self::Schema(schema) => write!(f, "unsupported control request schema: {schema}"),
            Self::Mismatch(message) => write!(f, "control request mismatch: {message}"),
            Self::ControlUpdaterMissing(path) => {
                write!(f, "copied control updater is missing: {}", path.display())
            }
            Self::RequestAlreadyExists(path) => {
                write!(f, "control request already exists: {}", path.display())
            }
        }
    }
}

impl std::error::Error for HandoffError {}

impl From<io::Error> for HandoffError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for HandoffError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<UpdateError> for HandoffError {
    fn from(value: UpdateError) -> Self {
        Self::Engine(value)
    }
}

pub type Result<T> = std::result::Result<T, HandoffError>;

pub fn prepare_control_handoff(engine: &UpdateEngine) -> Result<PreparedControlHandoff> {
    let journal = engine.read_journal()?.ok_or(HandoffError::JournalMissing)?;
    if journal.phase != UpdatePhase::Prepared {
        return Err(HandoffError::JournalNotPrepared(journal.phase));
    }

    let paths = engine.paths_for(&journal.transaction_id, &journal.updater_relative_path)?;
    if !paths.control_updater.is_file() {
        return Err(HandoffError::ControlUpdaterMissing(paths.control_updater));
    }

    let request_path = paths
        .staging_transaction
        .join("control")
        .join("handoff-request.json");
    if request_path.exists() {
        return Err(HandoffError::RequestAlreadyExists(request_path));
    }

    let request = ControlHandoffRequest {
        schema_version: CONTROL_REQUEST_SCHEMA_VERSION.to_owned(),
        install_root: engine.root().to_path_buf(),
        transaction_id: journal.transaction_id,
        candidate_version: journal.candidate_version,
        updater_relative_path: journal.updater_relative_path,
    };
    write_json_durable(&request_path, &request)?;

    Ok(PreparedControlHandoff {
        control_updater: paths.control_updater,
        request_path,
    })
}

pub fn read_control_request(path: &Path) -> Result<ControlHandoffRequest> {
    let request: ControlHandoffRequest = serde_json::from_slice(&fs::read(path)?)?;
    if request.schema_version != CONTROL_REQUEST_SCHEMA_VERSION {
        return Err(HandoffError::Schema(request.schema_version));
    }
    Ok(request)
}

pub fn validate_request_against_engine(
    request: &ControlHandoffRequest,
    engine: &UpdateEngine,
) -> Result<()> {
    if request.install_root != engine.root() {
        return Err(HandoffError::Mismatch("install_root".into()));
    }

    let journal = engine.read_journal()?.ok_or(HandoffError::JournalMissing)?;
    if journal.phase != UpdatePhase::Prepared {
        return Err(HandoffError::JournalNotPrepared(journal.phase));
    }
    if request.transaction_id != journal.transaction_id {
        return Err(HandoffError::Mismatch("transaction_id".into()));
    }
    if request.candidate_version != journal.candidate_version {
        return Err(HandoffError::Mismatch("candidate_version".into()));
    }
    if request.updater_relative_path != journal.updater_relative_path {
        return Err(HandoffError::Mismatch("updater_relative_path".into()));
    }

    let paths = engine.paths_for(&journal.transaction_id, &journal.updater_relative_path)?;
    if !paths.control_updater.is_file() {
        return Err(HandoffError::ControlUpdaterMissing(paths.control_updater));
    }
    Ok(())
}

pub fn started_path(request_path: &Path) -> PathBuf {
    request_path.with_extension("started")
}

pub fn receipt_path(request_path: &Path) -> PathBuf {
    request_path.with_extension("receipt.json")
}

pub fn write_control_receipt(path: &Path, receipt: &ControlReceipt) -> Result<()> {
    write_json_durable(path, receipt)
}

fn write_json_durable<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let next = path.with_extension("next");
    match fs::remove_file(&next) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&next)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);

    fs::rename(&next, path)?;
    sync_parent(path)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<()> {
    Ok(())
}
