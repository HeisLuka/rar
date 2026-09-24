use std::{
    env,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::{
    db::{MigrationReport, MigrationRuntime},
    runtime_error::RuntimeError,
};

const LEDGER_TABLE: &str = "cloud_schema_migrations";

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "source_ingress",
        sql: include_str!("../migrations/0001_source_ingress.sql"),
    },
    Migration {
        version: 2,
        name: "blob_store",
        sql: include_str!("../migrations/0002_blob_store.sql"),
    },
    Migration {
        version: 3,
        name: "jobs",
        sql: include_str!("../migrations/0003_jobs.sql"),
    },
];

#[derive(Clone)]
pub struct SqliteMigrationRuntime {
    path: PathBuf,
    max_connections: u32,
    busy_timeout: Duration,
}

impl SqliteMigrationRuntime {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let path = path.as_ref().to_path_buf();
        if path.as_os_str().is_empty() {
            return Err(RuntimeError::new(
                "invalid_database_path",
                "SQLite migration path must be non-empty",
            ));
        }

        Ok(Self {
            path,
            max_connections: 1,
            busy_timeout: Duration::from_secs(5),
        })
    }

    pub fn from_env() -> Result<Self, RuntimeError> {
        let raw = env::var("CHAPTERA_SQLITE_PATH").map_err(|_| {
            RuntimeError::new(
                "sqlite_path_not_configured",
                "CHAPTERA_SQLITE_PATH is required for chaptera migrate",
            )
        })?;
        Self::new(raw)
    }

    async fn open(&self) -> Result<SqlitePool, RuntimeError> {
        let options = SqliteConnectOptions::new()
            .filename(&self.path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(self.busy_timeout);

        let pool = SqlitePoolOptions::new()
            .max_connections(self.max_connections)
            .min_connections(1)
            .connect_with(options)
            .await
            .map_err(|error| runtime_sqlite_error("sqlite_migration_open_failed", error))?;

        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS {LEDGER_TABLE} (                version INTEGER PRIMARY KEY,                name TEXT NOT NULL,                sha256 TEXT NOT NULL,                applied_at_ms INTEGER NOT NULL            )"
        ))
        .execute(&pool)
        .await
        .map_err(|error| runtime_sqlite_error("migration_ledger_init_failed", error))?;

        Ok(pool)
    }

    async fn inspect(&self, pool: &SqlitePool) -> Result<MigrationReport, RuntimeError> {
        let rows = sqlx::query(&format!(
            "SELECT version, name, sha256 FROM {LEDGER_TABLE} ORDER BY version ASC"
        ))
        .fetch_all(pool)
        .await
        .map_err(|error| runtime_sqlite_error("migration_ledger_read_failed", error))?;

        if rows.len() > MIGRATIONS.len() {
            return Err(RuntimeError::new(
                "schema_version_too_new",
                "database contains migrations newer than this Chaptera build",
            ));
        }

        let mut applied_version = 0_i64;
        for (index, row) in rows.iter().enumerate() {
            let expected = MIGRATIONS.get(index).ok_or_else(|| {
                RuntimeError::new(
                    "schema_version_too_new",
                    "database contains an unsupported migration version",
                )
            })?;
            let version = row
                .try_get::<i64, _>("version")
                .map_err(|error| runtime_sqlite_error("migration_ledger_corrupt", error))?;
            let name = row
                .try_get::<String, _>("name")
                .map_err(|error| runtime_sqlite_error("migration_ledger_corrupt", error))?;
            let sha256 = row
                .try_get::<String, _>("sha256")
                .map_err(|error| runtime_sqlite_error("migration_ledger_corrupt", error))?;

            if version != expected.version {
                return Err(RuntimeError::new(
                    "migration_sequence_invalid",
                    format!(
                        "expected applied migration version {}, found {version}",
                        expected.version
                    ),
                ));
            }
            if name != expected.name {
                return Err(RuntimeError::new(
                    "migration_name_mismatch",
                    format!(
                        "migration {version} name changed: expected {}, found {name}",
                        expected.name
                    ),
                ));
            }
            let expected_sha = migration_sha256(expected.sql);
            if sha256 != expected_sha {
                return Err(RuntimeError::new(
                    "migration_checksum_mismatch",
                    format!("migration {version} checksum differs from this build"),
                ));
            }

            applied_version = version;
        }

        let pending_versions = MIGRATIONS
            .iter()
            .filter(|migration| migration.version > applied_version)
            .map(|migration| migration.version)
            .collect();

        Ok(MigrationReport {
            applied_version,
            supported_version: supported_version(),
            pending_versions,
        })
    }

    async fn apply_pending(&self, pool: &SqlitePool) -> Result<MigrationReport, RuntimeError> {
        let current = self.inspect(pool).await?;

        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version > current.applied_version)
        {
            let mut tx = pool
                .begin()
                .await
                .map_err(|error| runtime_sqlite_error("migration_transaction_failed", error))?;

            sqlx::raw_sql(migration.sql)
                .execute(&mut *tx)
                .await
                .map_err(|error| runtime_sqlite_error("migration_apply_failed", error))?;

            sqlx::query(&format!(
                "INSERT INTO {LEDGER_TABLE}(version, name, sha256, applied_at_ms) VALUES (?, ?, ?, ?)"
            ))
            .bind(migration.version)
            .bind(migration.name)
            .bind(migration_sha256(migration.sql))
            .bind(now_ms()?)
            .execute(&mut *tx)
            .await
            .map_err(|error| runtime_sqlite_error("migration_ledger_write_failed", error))?;

            tx.commit()
                .await
                .map_err(|error| runtime_sqlite_error("migration_commit_failed", error))?;
        }

        self.inspect(pool).await
    }
}

#[async_trait::async_trait]
impl MigrationRuntime for SqliteMigrationRuntime {
    async fn status(&self) -> Result<MigrationReport, RuntimeError> {
        let pool = self.open().await?;
        let result = self.inspect(&pool).await;
        pool.close().await;
        result
    }

    async fn up(&self) -> Result<MigrationReport, RuntimeError> {
        let pool = self.open().await?;
        let result = self.apply_pending(&pool).await;
        pool.close().await;
        result
    }
}

fn supported_version() -> i64 {
    MIGRATIONS.last().map(|migration| migration.version).unwrap_or(0)
}

fn migration_sha256(sql: &str) -> String {
    format!("{:x}", Sha256::digest(sql.as_bytes()))
}

fn now_ms() -> Result<i64, RuntimeError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RuntimeError::new("system_clock_invalid", "system clock is before UNIX epoch"))?;
    i64::try_from(duration.as_millis()).map_err(|_| {
        RuntimeError::new(
            "system_clock_invalid",
            "system clock milliseconds do not fit migration ledger",
        )
    })
}

fn runtime_sqlite_error(code: &'static str, error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::new(code, error.to_string().chars().take(512).collect::<String>())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn database_path(label: &str) -> PathBuf {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-migrate-{label}-{}-{n}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test]
    async fn status_reports_all_migrations_pending_on_fresh_database() {
        let path = database_path("status");
        cleanup(&path);
        let runtime = SqliteMigrationRuntime::new(&path).unwrap();

        let report = runtime.status().await.unwrap();

        assert_eq!(report.applied_version, 0);
        assert_eq!(report.supported_version, 3);
        assert_eq!(report.pending_versions, vec![1, 2, 3]);
        cleanup(&path);
    }

    #[tokio::test]
    async fn up_applies_all_migrations_and_is_idempotent() {
        let path = database_path("up");
        cleanup(&path);
        let runtime = SqliteMigrationRuntime::new(&path).unwrap();

        let first = runtime.up().await.unwrap();
        assert_eq!(first.applied_version, 3);
        assert!(first.pending_versions.is_empty());

        let second = runtime.up().await.unwrap();
        assert_eq!(second, first);

        let pool = runtime.open().await.unwrap();
        for table in [
            "uploads",
            "upload_consumptions",
            "physical_blobs",
            "resource_bindings",
            "jobs",
            "job_effects",
        ] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(exists, 1, "missing table {table}");
        }
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn changed_applied_checksum_fails_closed() {
        let path = database_path("checksum");
        cleanup(&path);
        let runtime = SqliteMigrationRuntime::new(&path).unwrap();
        runtime.up().await.unwrap();

        let pool = runtime.open().await.unwrap();
        sqlx::query(&format!(
            "UPDATE {LEDGER_TABLE} SET sha256='tampered' WHERE version=2"
        ))
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        let error = runtime.status().await.unwrap_err();
        assert_eq!(error.code, "migration_checksum_mismatch");
        cleanup(&path);
    }
}
