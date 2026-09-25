use std::time::Duration;

use crate::{
    config::ChapteraConfig,
    job_queue::{JobKind, SqliteJobQueue},
    job_worker::WorkerLoopConfig,
    quota_admission::{SqliteQuotaAdmissionConfig, SqliteQuotaJobAdmission},
    quota_store::{QuotaConfig, SqliteQuotaAuthority},
    runtime_error::RuntimeError,
};

#[derive(Clone)]
pub struct JobsAdmissionRuntime {
    queue: SqliteJobQueue,
    admission: SqliteQuotaJobAdmission,
}

impl JobsAdmissionRuntime {
    pub async fn open(config: &ChapteraConfig) -> Result<Self, RuntimeError> {
        let busy_timeout = Duration::from_millis(config.sqlite.busy_timeout_ms);
        let path = &config.sqlite.path;
        let pool_max = config.sqlite.pool_max;

        let queue = SqliteJobQueue::open(path, pool_max, busy_timeout)
            .await
            .map_err(|error| RuntimeError::new(error.code, error.message))?;

        let quota = SqliteQuotaAuthority::open(
            path,
            pool_max,
            busy_timeout,
            QuotaConfig {
                shared_capacity: config.worker.quota_shared_capacity,
                semantic_headroom: config.worker.quota_semantic_headroom,
                export_cap: config.worker.quota_export_cap,
                background_cap: config.worker.quota_background_cap,
            },
        )
        .await
        .map_err(|error| RuntimeError::new(error.code, error.message))?;

        let worker_config =
            WorkerLoopConfig::conservative_v0("serve-readiness", vec![JobKind::Export]);
        let admission = SqliteQuotaJobAdmission::new(
            quota,
            SqliteQuotaAdmissionConfig::conservative_v0(worker_config.lease_duration),
        )?;

        Ok(Self { queue, admission })
    }

    pub fn queue(&self) -> &SqliteJobQueue {
        &self.queue
    }

    pub fn admission(&self) -> &SqliteQuotaJobAdmission {
        &self.admission
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use crate::{config::ChapteraConfig, schema_migration::SqliteMigrationRuntime};

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-jobs-runtime-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test]
    async fn open_requires_migrated_queue_and_quota_authority() {
        let path = temp_db("open");
        let mut config = ChapteraConfig::development_from_env().unwrap();
        config.sqlite.path = path.clone();

        let missing = JobsAdmissionRuntime::open(&config).await.unwrap_err();
        assert!(matches!(
            missing.code,
            "job_queue_database_missing" | "sqlite_database_missing"
        ));
        assert!(!path.exists());

        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();

        let runtime = JobsAdmissionRuntime::open(&config).await.unwrap();
        let _ = runtime.queue();
        let _ = runtime.admission();

        drop(runtime);
        cleanup(&path);
    }
}
