use std::sync::{Arc, RwLock};

use crate::{
    authn::SqliteAuthnStore,
    job_queue::SqliteJobQueue,
    sqlite_store::SqliteRevisionStore,
    state::{DependencyFailure, RuntimeDependency},
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReadinessState {
    Ready,
    Failed(DependencyFailure),
}

/// Mutable readiness state for one already-opened producer.
///
/// The initial Ready state is not publicly constructible on its own: callers
/// obtain a handle only by binding a concrete producer that has already passed
/// its async open/validation path. Runtime code may later fail or restore the
/// snapshot when it observes a concrete producer lifecycle event.
#[derive(Clone)]
pub struct ReadinessHandle {
    state: Arc<RwLock<ReadinessState>>,
}

impl ReadinessHandle {
    fn ready() -> Self {
        Self {
            state: Arc::new(RwLock::new(ReadinessState::Ready)),
        }
    }

    pub fn fail(
        &self,
        code: &'static str,
        message: impl Into<String>,
    ) -> Result<(), DependencyFailure> {
        let mut state = self.state.write().map_err(|_| poisoned_state())?;
        *state = ReadinessState::Failed(DependencyFailure::new(code, message));
        Ok(())
    }

    pub fn restore(&self) -> Result<(), DependencyFailure> {
        let mut state = self.state.write().map_err(|_| poisoned_state())?;
        *state = ReadinessState::Ready;
        Ok(())
    }

    fn check(&self) -> Result<(), DependencyFailure> {
        let state = self.state.read().map_err(|_| poisoned_state())?;
        match &*state {
            ReadinessState::Ready => Ok(()),
            ReadinessState::Failed(error) => Err(error.clone()),
        }
    }
}

pub struct RuntimeDependencyBinding {
    pub dependency: Arc<dyn RuntimeDependency>,
    pub readiness: ReadinessHandle,
}

pub struct LocalSqliteReadiness {
    pub authn: RuntimeDependencyBinding,
    pub revision_stream: RuntimeDependencyBinding,
    pub jobs: RuntimeDependencyBinding,
}

impl LocalSqliteReadiness {
    /// Bind readiness to producers that have already completed their real async
    /// open/schema/profile validation. Holding the producer inside the runtime
    /// dependency also keeps its underlying pool alive for the binding lifetime.
    pub fn from_opened(
        authn: SqliteAuthnStore,
        revision_stream: SqliteRevisionStore,
        jobs: SqliteJobQueue,
    ) -> Self {
        Self {
            authn: bind(authn),
            revision_stream: bind(revision_stream),
            jobs: bind(jobs),
        }
    }
}

struct OpenedProducerDependency<P> {
    _producer: P,
    readiness: ReadinessHandle,
}

impl<P> RuntimeDependency for OpenedProducerDependency<P>
where
    P: Send + Sync,
{
    fn check(&self) -> Result<(), DependencyFailure> {
        self.readiness.check()
    }
}

fn bind<P>(producer: P) -> RuntimeDependencyBinding
where
    P: Send + Sync + 'static,
{
    let readiness = ReadinessHandle::ready();
    RuntimeDependencyBinding {
        dependency: Arc::new(OpenedProducerDependency {
            _producer: producer,
            readiness: readiness.clone(),
        }),
        readiness,
    }
}

fn poisoned_state() -> DependencyFailure {
    DependencyFailure::new(
        "readiness_state_poisoned",
        "runtime readiness state lock is poisoned",
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use crate::schema_migration::SqliteMigrationRuntime;

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-runtime-readiness-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test]
    async fn opened_local_producers_start_ready_and_can_fail_closed() {
        let path = temp_db("local");
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();

        let authn = SqliteAuthnStore::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();
        let revision_stream = SqliteRevisionStore::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();
        let jobs = SqliteJobQueue::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();

        let bindings = LocalSqliteReadiness::from_opened(authn, revision_stream, jobs);
        bindings.authn.dependency.check().unwrap();
        bindings.revision_stream.dependency.check().unwrap();
        bindings.jobs.dependency.check().unwrap();

        bindings
            .jobs
            .readiness
            .fail("job_queue_unavailable", "job queue lifecycle degraded")
            .unwrap();
        let error = bindings.jobs.dependency.check().unwrap_err();
        assert_eq!(error.code, "job_queue_unavailable");
        assert_eq!(error.message, "job queue lifecycle degraded");

        bindings.jobs.readiness.restore().unwrap();
        bindings.jobs.dependency.check().unwrap();

        drop(bindings);
        cleanup(&path);
    }

    #[tokio::test]
    async fn binding_requires_real_opened_producers_not_an_abstract_ready_flag() {
        let path = temp_db("missing");
        let authn = SqliteAuthnStore::open(&path, 1, Duration::from_secs(1)).await;
        assert!(authn.is_err());
        assert!(!path.exists());
        cleanup(&path);
    }
}
