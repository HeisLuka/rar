use std::{sync::Arc, time::Duration};

use crate::{
    authz_runtime::{
        SqliteAuthorizedExportPublicationCommitter, SqliteAuthzAuthority,
        SqliteExportPublishPreflightAuthorizer,
    },
    blob_runtime::BlobStoreRuntime,
    config::ChapteraConfig,
    derived_artifacts::SqliteDerivedArtifactStore,
    export_executor::{ExactRevisionEditableExporter, PublishedExportJobExecutor},
    export_publication::SqliteExportPublicationStore,
    job_executor_registry::JobExecutorRegistry,
    job_queue::{JobKind, SqliteJobQueue},
    job_worker::{WorkerControl, WorkerLoop, WorkerLoopConfig},
    jobs::{WorkerRuntime, WorkerRuntimeFuture},
    quota_admission::{SqliteQuotaAdmissionConfig, SqliteQuotaJobAdmission},
    quota_store::{QuotaConfig, SqliteQuotaAuthority},
    revision_materializer::{
        BlobStoreExactSourceLoader, ExactRevisionMaterializer, PubEditorReplayEngine,
    },
    runtime_error::RuntimeError,
    shutdown,
    source_authority::SqliteDocumentSourceAuthority,
    sqlite_store::SqliteRevisionStore,
};

#[derive(Clone)]
pub struct ConfiguredWorkerRuntime {
    config: ChapteraConfig,
}

impl ConfiguredWorkerRuntime {
    pub fn new(config: ChapteraConfig) -> Self {
        Self { config }
    }

    async fn run_configured(&self) -> Result<(), RuntimeError> {
        let busy_timeout = Duration::from_millis(self.config.sqlite.busy_timeout_ms);
        let pool_max = self.config.sqlite.pool_max;
        let path = &self.config.sqlite.path;

        let blob_runtime = BlobStoreRuntime::open(&self.config)
            .await
            .map_err(|error| runtime_error(error.code, error.message))?;
        let blob_store = blob_runtime.service().clone();

        let source_authority = SqliteDocumentSourceAuthority::open(path, pool_max, busy_timeout)
            .await
            .map_err(|error| runtime_error(error.code, error.message))?;
        let revision_store = SqliteRevisionStore::open(path, pool_max, busy_timeout)
            .await
            .map_err(|error| runtime_error(error.code, error.message))?;
        let materializer = Arc::new(ExactRevisionMaterializer::new(
            Arc::new(source_authority),
            Arc::new(BlobStoreExactSourceLoader::new(blob_store.clone())),
            revision_store,
            Arc::new(PubEditorReplayEngine),
        ));
        let producer = Arc::new(ExactRevisionEditableExporter::new(materializer));

        let artifacts = SqliteDerivedArtifactStore::open(path, pool_max, busy_timeout)
            .await
            .map_err(|error| runtime_error(error.code, error.message))?;
        let publications = SqliteExportPublicationStore::open(path, pool_max, busy_timeout)
            .await
            .map_err(|error| runtime_error(error.code, error.message))?;
        let authz = SqliteAuthzAuthority::open(path, pool_max, busy_timeout)
            .await
            .map_err(|error| runtime_error(error.code, error.message))?;
        let authorizer = Arc::new(SqliteExportPublishPreflightAuthorizer::new(authz.clone()));
        let publication_committer = Arc::new(
            SqliteAuthorizedExportPublicationCommitter::new(authz, publications)
                .map_err(|error| runtime_error(error.code, error.message))?,
        );

        let export_executor = Arc::new(PublishedExportJobExecutor::new(
            producer,
            blob_store,
            artifacts,
            publication_committer,
            authorizer,
        ));
        let registry = Arc::new(JobExecutorRegistry::new(vec![(
            JobKind::Export,
            export_executor,
        )])?);
        registry.require_kinds(&[JobKind::Export])?;

        let quota = SqliteQuotaAuthority::open(
            path,
            pool_max,
            busy_timeout,
            QuotaConfig {
                shared_capacity: self.config.worker.quota_shared_capacity,
                semantic_headroom: self.config.worker.quota_semantic_headroom,
                export_cap: self.config.worker.quota_export_cap,
                background_cap: self.config.worker.quota_background_cap,
            },
        )
        .await
        .map_err(|error| runtime_error(error.code, error.message))?;

        let loop_config = WorkerLoopConfig::conservative_v0(
            format!("worker:{}", std::process::id()),
            vec![JobKind::Export],
        );
        let admission = Arc::new(SqliteQuotaJobAdmission::new(
            quota,
            SqliteQuotaAdmissionConfig::conservative_v0(loop_config.lease_duration),
        )?);
        let queue = SqliteJobQueue::open(path, pool_max, busy_timeout)
            .await
            .map_err(|error| runtime_error(error.code, error.message))?;
        let control = WorkerControl::default();
        let worker_loop =
            WorkerLoop::new(queue, registry, admission, control.clone(), loop_config)?;

        let run = worker_loop.run();
        tokio::pin!(run);
        let _receipt = tokio::select! {
            result = &mut run => result?,
            _ = shutdown::signal() => {
                control.request_drain();
                run.await?
            }
        };
        Ok(())
    }
}

impl WorkerRuntime for ConfiguredWorkerRuntime {
    fn run(&self) -> WorkerRuntimeFuture<'_> {
        Box::pin(async move { self.run_configured().await })
    }
}

fn runtime_error(code: &'static str, message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_policy_is_explicit_not_derived_from_concurrency() {
        let mut config = ChapteraConfig::development_from_env().unwrap();
        config.worker.heavy_concurrency = 4;
        config.worker.light_concurrency = 32;
        config.worker.quota_shared_capacity = 7;
        config.worker.quota_semantic_headroom = 3;
        config.worker.quota_export_cap = 2;
        config.worker.quota_background_cap = 1;

        assert_eq!(config.worker.quota_shared_capacity, 7);
        assert_eq!(config.worker.quota_semantic_headroom, 3);
        assert_eq!(config.worker.quota_export_cap, 2);
        assert_eq!(config.worker.quota_background_cap, 1);
    }
}
