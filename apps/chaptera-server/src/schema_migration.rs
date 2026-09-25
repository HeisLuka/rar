use std::{
    env, fmt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{
    Connection, Row, SqliteConnection, raw_sql,
    sqlite::{SqliteConnectOptions, SqliteSynchronous},
};

use crate::{db::MigrationRuntime, runtime_error::RuntimeError};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const LEDGER_TABLE: &str = "chaptera_schema_migrations";

const SOURCE_INGRESS_SQL: &str = include_str!("../migrations/0001_source_ingress.sql");
const BLOB_STORE_SQL: &str = include_str!("../migrations/0002_blob_store.sql");
const JOBS_SQL: &str = include_str!("../migrations/0003_jobs.sql");
const BLOB_GC_SQL: &str = include_str!("../migrations/0004_blob_gc.sql");
const REVISION_STREAM_SQL: &str = include_str!("../migrations/0005_revision_stream.sql");

#[derive(Clone, Copy)]
struct MigrationSpec {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[MigrationSpec] = &[
    MigrationSpec {
        version: 1,
        name: "source_ingress",
        sql: SOURCE_INGRESS_SQL,
    },
    MigrationSpec {
        version: 2,
        name: "blob_store",
        sql: BLOB_STORE_SQL,
    },
    MigrationSpec {
        version: 3,
        name: "jobs",
        sql: JOBS_SQL,
    },
    MigrationSpec {
        version: 4,
        name: "blob_gc",
        sql: BLOB_GC_SQL,
    },
    MigrationSpec {
        version: 5,
        name: "revision_stream",
        sql: REVISION_STREAM_SQL,
    },
];

pub const CURRENT_SCHEMA_VERSION: i64 = 5;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MigrationReport {
    pub state: String,
    pub current_version: i64,
    pub target_version: i64,
    pub pending_versions: Vec<i64>,
    pub applied_versions: Vec<i64>,
    pub rollback_previous_binary_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationError {
    pub code: &'static str,
    pub message: String,
}

impl MigrationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for MigrationError {}

#[derive(Clone)]
pub struct SqliteMigrationRuntime {
    path: PathBuf,
    busy_timeout: Duration,
}

impl SqliteMigrationRuntime {
    pub fn from_env() -> Result<Self, RuntimeError> {
        let raw = env::var("CHAPTERA_SQLITE_PATH").map_err(|_| {
            RuntimeError::new(
                "sqlite_path_required",
                "CHAPTERA_SQLITE_PATH must point to the local Chaptera SQLite database",
            )
        })?;

        if raw.trim().is_empty() {
            return Err(RuntimeError::new(
                "sqlite_path_required",
                "CHAPTERA_SQLITE_PATH must be non-empty",
            ));
        }

        Ok(Self {
            path: PathBuf::from(raw),
            busy_timeout: BUSY_TIMEOUT,
        })
    }

    pub fn new(path: impl AsRef<Path>, busy_timeout: Duration) -> Result<Self, MigrationError> {
        if path.as_ref().as_os_str().is_empty() {
            return Err(MigrationError::new(
                "invalid_database_path",
                "migration database path must be non-empty",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(MigrationError::new(
                "invalid_busy_timeout",
                "migration busy timeout must be >0 and <=30 seconds",
            ));
        }

        Ok(Self {
            path: path.as_ref().to_path_buf(),
            busy_timeout,
        })
    }

    pub async fn status_report(&self) -> Result<MigrationReport, MigrationError> {
        inspect_status(&self.path, self.busy_timeout).await
    }

    pub async fn migrate_up(&self) -> Result<MigrationReport, MigrationError> {
        migrate_to(&self.path, self.busy_timeout, CURRENT_SCHEMA_VERSION).await
    }
}

#[async_trait(?Send)]
impl MigrationRuntime for SqliteMigrationRuntime {
    async fn status(&self) -> Result<(), RuntimeError> {
        let report = self.status_report().await.map_err(runtime_error)?;
        print_report(&report)
    }

    async fn up(&self) -> Result<(), RuntimeError> {
        let report = self.migrate_up().await.map_err(runtime_error)?;
        print_report(&report)
    }
}

fn print_report(report: &MigrationReport) -> Result<(), RuntimeError> {
    let rendered = serde_json::to_string_pretty(report).map_err(|error| {
        RuntimeError::new(
            "migration_report_serialization_failed",
            format!("could not serialize migration report: {error}"),
        )
    })?;
    println!("{rendered}");
    Ok(())
}

fn runtime_error(error: MigrationError) -> RuntimeError {
    RuntimeError::new(error.code, error.message)
}

async fn inspect_status(
    path: &Path,
    busy_timeout: Duration,
) -> Result<MigrationReport, MigrationError> {
    if !path.exists() {
        return Ok(report_from_versions(Vec::new()));
    }

    let mut connection = connect(path, busy_timeout, false).await?;
    let ledger = table_exists(&mut connection, LEDGER_TABLE).await?;

    if !ledger {
        if known_schema_tables_present(&mut connection).await? {
            return Err(MigrationError::new(
                "legacy_unversioned_schema",
                "known Chaptera tables exist without the global migration ledger; refuse implicit adoption",
            ));
        }
        return Ok(report_from_versions(Vec::new()));
    }

    let applied = load_and_validate_applied(&mut connection).await?;
    Ok(report_from_versions(
        applied.into_iter().map(|row| row.version).collect(),
    ))
}

async fn migrate_to(
    path: &Path,
    busy_timeout: Duration,
    target_version: i64,
) -> Result<MigrationReport, MigrationError> {
    if !(0..=CURRENT_SCHEMA_VERSION).contains(&target_version) {
        return Err(MigrationError::new(
            "invalid_migration_target",
            format!(
                "target schema version {target_version} is outside supported 0..={CURRENT_SCHEMA_VERSION}"
            ),
        ));
    }

    let existed = path.exists();
    let mut connection = connect(path, busy_timeout, true).await?;

    if existed
        && !table_exists(&mut connection, LEDGER_TABLE).await?
        && known_schema_tables_present(&mut connection).await?
    {
        return Err(MigrationError::new(
            "legacy_unversioned_schema",
            "known Chaptera tables exist without the global migration ledger; refuse implicit adoption",
        ));
    }

    let mut tx = connection
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(sqlite_error)?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS chaptera_schema_migrations (
            version          INTEGER PRIMARY KEY,
            name             TEXT NOT NULL UNIQUE,
            checksum_sha256  TEXT NOT NULL,
            applied_at_ms    INTEGER NOT NULL
        )
        "#,
    )
    .execute(&mut *tx)
    .await
    .map_err(sqlite_error)?;

    let applied = load_and_validate_applied(&mut tx).await?;
    let current = applied.last().map(|row| row.version).unwrap_or(0);

    if current > target_version {
        return Err(MigrationError::new(
            "schema_version_too_new_for_target",
            format!("database is already at schema {current}, target request was {target_version}"),
        ));
    }

    for spec in MIGRATIONS
        .iter()
        .filter(|spec| spec.version > current && spec.version <= target_version)
    {
        raw_sql(spec.sql).execute(&mut *tx).await.map_err(|error| {
            MigrationError::new(
                "migration_apply_failed",
                format!(
                    "migration {} ({}) failed: {}",
                    spec.version,
                    spec.name,
                    bounded_sqlx_message(&error)
                ),
            )
        })?;

        sqlx::query(
            r#"
            INSERT INTO chaptera_schema_migrations (
                version, name, checksum_sha256, applied_at_ms
            ) VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(spec.version)
        .bind(spec.name)
        .bind(checksum(spec.sql))
        .bind(now_ms()?)
        .execute(&mut *tx)
        .await
        .map_err(sqlite_error)?;
    }

    tx.commit().await.map_err(sqlite_error)?;

    inspect_status(path, busy_timeout).await
}

#[derive(Debug)]
struct AppliedMigration {
    version: i64,
    name: String,
    checksum_sha256: String,
}

async fn load_and_validate_applied(
    connection: &mut SqliteConnection,
) -> Result<Vec<AppliedMigration>, MigrationError> {
    let rows = sqlx::query(
        r#"
        SELECT version, name, checksum_sha256
        FROM chaptera_schema_migrations
        ORDER BY version ASC
        "#,
    )
    .fetch_all(connection)
    .await
    .map_err(sqlite_error)?;

    let mut applied = Vec::with_capacity(rows.len());
    for row in rows {
        applied.push(AppliedMigration {
            version: row.try_get("version").map_err(sqlite_error)?,
            name: row.try_get("name").map_err(sqlite_error)?,
            checksum_sha256: row.try_get("checksum_sha256").map_err(sqlite_error)?,
        });
    }

    validate_applied(&applied)?;
    Ok(applied)
}

fn validate_applied(applied: &[AppliedMigration]) -> Result<(), MigrationError> {
    for (expected_next, row) in (1_i64..).zip(applied.iter()) {
        if row.version > CURRENT_SCHEMA_VERSION {
            return Err(MigrationError::new(
                "schema_version_too_new",
                format!(
                    "database contains migration {} newer than supported {}",
                    row.version, CURRENT_SCHEMA_VERSION
                ),
            ));
        }

        if row.version != expected_next {
            return Err(MigrationError::new(
                "migration_history_gap",
                format!(
                    "expected migration version {expected_next}, found {}",
                    row.version
                ),
            ));
        }

        let spec = MIGRATIONS
            .iter()
            .find(|spec| spec.version == row.version)
            .ok_or_else(|| {
                MigrationError::new(
                    "schema_version_too_new",
                    format!(
                        "database contains migration {} newer/unknown to this binary",
                        row.version
                    ),
                )
            })?;

        if row.name != spec.name {
            return Err(MigrationError::new(
                "migration_identity_mismatch",
                format!(
                    "migration {} name mismatch: database={}, binary={}",
                    row.version, row.name, spec.name
                ),
            ));
        }

        let expected_checksum = checksum(spec.sql);
        if row.checksum_sha256 != expected_checksum {
            return Err(MigrationError::new(
                "migration_checksum_mismatch",
                format!(
                    "migration {} checksum differs from this binary",
                    row.version
                ),
            ));
        }

    }

    Ok(())
}

fn report_from_versions(applied_versions: Vec<i64>) -> MigrationReport {
    let current_version = applied_versions.last().copied().unwrap_or(0);
    let pending_versions = MIGRATIONS
        .iter()
        .filter(|spec| spec.version > current_version)
        .map(|spec| spec.version)
        .collect::<Vec<_>>();

    MigrationReport {
        state: if current_version == CURRENT_SCHEMA_VERSION {
            "current".to_owned()
        } else {
            "pending".to_owned()
        },
        current_version,
        target_version: CURRENT_SCHEMA_VERSION,
        pending_versions,
        applied_versions,
        // V0 has no declared cross-schema binary rollback window yet.
        // Any schema-changing deployment must keep/restore a pre-migration DB
        // backup before the previous binary can be considered safe.
        rollback_previous_binary_safe: current_version == 0,
    }
}

async fn connect(
    path: &Path,
    busy_timeout: Duration,
    create_if_missing: bool,
) -> Result<SqliteConnection, MigrationError> {
    // Do not assert journal_mode on every migration connection.
    // WAL is persistent once configured, and reasserting it is a lock-taking
    // PRAGMA that can race another migrator before BEGIN IMMEDIATE gets a
    // chance to serialize the writers. Migration correctness only requires
    // the transaction lock below; runtime stores may configure WAL when they
    // open the database after the operator migration step.
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(create_if_missing)
        .synchronous(SqliteSynchronous::Full)
        .foreign_keys(true)
        .busy_timeout(busy_timeout);

    SqliteConnection::connect_with(&options)
        .await
        .map_err(sqlite_error)
}

async fn table_exists(
    connection: &mut SqliteConnection,
    table_name: &str,
) -> Result<bool, MigrationError> {
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?")
            .bind(table_name)
            .fetch_one(connection)
            .await
            .map_err(sqlite_error)?;

    Ok(exists == 1)
}

async fn known_schema_tables_present(
    connection: &mut SqliteConnection,
) -> Result<bool, MigrationError> {
    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM sqlite_master
        WHERE type='table'
          AND name IN (
            'uploads',
            'physical_blobs',
            'resource_bindings',
            'jobs',
            'gc_candidates',
            'revision_edges'
          )
        "#,
    )
    .fetch_one(connection)
    .await
    .map_err(sqlite_error)?;

    Ok(count > 0)
}

fn checksum(sql: &str) -> String {
    let digest = Sha256::digest(sql.as_bytes());
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn now_ms() -> Result<i64, MigrationError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| MigrationError::new("clock_before_epoch", error.to_string()))?
        .as_millis();

    i64::try_from(millis)
        .map_err(|_| MigrationError::new("clock_overflow", "system clock does not fit i64 millis"))
}

fn sqlite_error(error: impl fmt::Display) -> MigrationError {
    MigrationError::new(
        "sqlite_migration_error",
        bounded_message(&error.to_string()),
    )
}

fn bounded_sqlx_message(error: &sqlx::Error) -> String {
    bounded_message(&error.to_string())
}

fn bounded_message(message: &str) -> String {
    const MAX: usize = 512;
    if message.len() <= MAX {
        message.to_owned()
    } else {
        format!("{}...", &message[..MAX])
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let n = NEXT_DB.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!(
            "chaptera-migration-{label}-{}-{n}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for candidate in [
            path.to_path_buf(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ] {
            let _ = fs::remove_file(candidate);
        }
    }

    #[tokio::test]
    async fn status_on_absent_database_is_non_mutating_and_pending() {
        let path = temp_db("absent");
        let runtime = SqliteMigrationRuntime::new(&path, Duration::from_secs(2)).unwrap();

        let report = runtime.status_report().await.unwrap();
        assert_eq!(report.state, "pending");
        assert_eq!(report.current_version, 0);
        assert_eq!(report.target_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(report.pending_versions, vec![1, 2, 3, 4, 5]);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn migrate_up_is_idempotent_and_records_all_checksums() {
        let path = temp_db("up");
        let runtime = SqliteMigrationRuntime::new(&path, Duration::from_secs(2)).unwrap();

        let first = runtime.migrate_up().await.unwrap();
        assert_eq!(first.state, "current");
        assert_eq!(first.applied_versions, vec![1, 2, 3, 4, 5]);

        let second = runtime.migrate_up().await.unwrap();
        assert_eq!(second, first);

        let mut connection = connect(&path, Duration::from_secs(2), false).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chaptera_schema_migrations")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(count, CURRENT_SCHEMA_VERSION);

        cleanup(&path);
    }

    #[tokio::test]
    async fn checksum_tamper_fails_closed() {
        let path = temp_db("checksum");
        let runtime = SqliteMigrationRuntime::new(&path, Duration::from_secs(2)).unwrap();
        runtime.migrate_up().await.unwrap();

        let mut connection = connect(&path, Duration::from_secs(2), false).await.unwrap();
        sqlx::query(
            "UPDATE chaptera_schema_migrations SET checksum_sha256='tampered' WHERE version=2",
        )
        .execute(&mut connection)
        .await
        .unwrap();
        drop(connection);

        let error = runtime.status_report().await.unwrap_err();
        assert_eq!(error.code, "migration_checksum_mismatch");
        cleanup(&path);
    }

    #[tokio::test]
    async fn unknown_newer_version_fails_closed() {
        let path = temp_db("too-new");
        let runtime = SqliteMigrationRuntime::new(&path, Duration::from_secs(2)).unwrap();
        runtime.migrate_up().await.unwrap();

        let mut connection = connect(&path, Duration::from_secs(2), false).await.unwrap();
        sqlx::query(
            r#"
            INSERT INTO chaptera_schema_migrations(
                version, name, checksum_sha256, applied_at_ms
            ) VALUES (999, 'future', 'future', 0)
            "#,
        )
        .execute(&mut connection)
        .await
        .unwrap();
        drop(connection);

        let error = runtime.status_report().await.unwrap_err();
        assert_eq!(error.code, "schema_version_too_new");
        cleanup(&path);
    }

    #[tokio::test]
    async fn restart_at_every_migration_boundary_converges_to_current() {
        for boundary in 1..=CURRENT_SCHEMA_VERSION {
            let path = temp_db(&format!("boundary-{boundary}"));

            migrate_to(&path, Duration::from_secs(2), boundary)
                .await
                .unwrap();

            let restarted = SqliteMigrationRuntime::new(&path, Duration::from_secs(2)).unwrap();
            let final_report = restarted.migrate_up().await.unwrap();
            assert_eq!(final_report.state, "current");
            assert_eq!(final_report.current_version, CURRENT_SCHEMA_VERSION);

            cleanup(&path);
        }
    }

    #[tokio::test]
    async fn concurrent_migrate_up_has_one_serialized_history() {
        for attempt in 0..16 {
            let path = temp_db(&format!("concurrent-{attempt}"));
            let left = SqliteMigrationRuntime::new(&path, Duration::from_secs(5)).unwrap();
            let right = left.clone();

            let (left, right) = tokio::join!(left.migrate_up(), right.migrate_up());
            assert_eq!(left.unwrap().state, "current");
            assert_eq!(right.unwrap().state, "current");

            let final_report = SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
                .unwrap()
                .status_report()
                .await
                .unwrap();
            assert_eq!(final_report.applied_versions, vec![1, 2, 3, 4, 5]);

            cleanup(&path);
        }
    }

    #[tokio::test]
    async fn migration_connection_preserves_existing_wal_mode() {
        let path = temp_db("preserve-wal");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(2));
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        drop(connection);

        let runtime = SqliteMigrationRuntime::new(&path, Duration::from_secs(2)).unwrap();
        runtime.migrate_up().await.unwrap();

        connection = connect(&path, Duration::from_secs(2), false).await.unwrap();
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(mode.to_ascii_lowercase(), "wal");

        cleanup(&path);
    }

    #[tokio::test]
    async fn refuses_unversioned_known_schema() {
        let path = temp_db("legacy");
        let mut connection = connect(&path, Duration::from_secs(2), true).await.unwrap();
        sqlx::query("CREATE TABLE jobs(job_id BLOB PRIMARY KEY)")
            .execute(&mut connection)
            .await
            .unwrap();
        drop(connection);

        let runtime = SqliteMigrationRuntime::new(&path, Duration::from_secs(2)).unwrap();
        let error = runtime.migrate_up().await.unwrap_err();
        assert_eq!(error.code, "legacy_unversioned_schema");

        cleanup(&path);
    }
}
