use std::sync::{Arc, RwLock};

use crate::{
    sqlite_store::SqliteRevisionStore,
    state::{DependencyFailure, RuntimeDependency, RuntimePorts},
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

/// Concrete readiness binding for the physical RevisionStream producer.
///
/// Unlike lower-level storage helpers, SqliteRevisionStore is itself the
/// required RevisionStream producer represented by RuntimePorts. AuthN and jobs
/// intentionally do not get equivalent constructors here yet: their SQLite
/// stores alone do not prove that the full AuthN or jobs/admission service is
/// assembled.
pub fn revision_stream_dependency(
    revision_stream: SqliteRevisionStore,
) -> RuntimeDependencyBinding {
    bind(revision_stream)
}

pub struct RevisionStreamPorts {
    pub ports: RuntimePorts,
    pub readiness: ReadinessHandle,
}

/// Assemble the first real RuntimePorts component without weakening any other
/// required dependency. Overall readiness must therefore remain false until the
/// remaining producers are independently connected.
pub fn ports_with_revision_stream(
    revision_stream: SqliteRevisionStore,
) -> RevisionStreamPorts {
    let binding = revision_stream_dependency(revision_stream);
    let mut ports = RuntimePorts::unconfigured();
    ports.revision_stream = binding.dependency;
    RevisionStreamPorts {
        ports,
        readiness: binding.readiness,
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
    async fn opened_revision_stream_starts_ready_and_can_fail_closed() {
        let path = temp_db("revision");
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();

        let revision_stream = SqliteRevisionStore::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();

        let binding = revision_stream_dependency(revision_stream);
        binding.dependency.check().unwrap();

        binding
            .readiness
            .fail(
                "revision_stream_unavailable",
                "revision stream lifecycle degraded",
            )
            .unwrap();
        let error = binding.dependency.check().unwrap_err();
        assert_eq!(error.code, "revision_stream_unavailable");
        assert_eq!(error.message, "revision stream lifecycle degraded");

        binding.readiness.restore().unwrap();
        binding.dependency.check().unwrap();

        drop(binding);
        cleanup(&path);
    }

    #[tokio::test]
    async fn partial_ports_expose_revision_stream_without_claiming_server_ready() {
        let path = temp_db("ports");
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();

        let revision_stream = SqliteRevisionStore::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();
        let assembled = ports_with_revision_stream(revision_stream);

        let report = assembled.ports.readiness_report();
        assert!(!report.ready);
        assert_eq!(report.status, "not_ready");
        assert!(report.components["revision_stream"].ready);
        for component in ["authn", "authz", "jobs", "blob_store"] {
            assert!(!report.components[component].ready);
            assert_eq!(
                report.components[component].code.as_deref(),
                Some("not_configured")
            );
        }

        assembled
            .readiness
            .fail(
                "revision_stream_unavailable",
                "revision stream lifecycle degraded",
            )
            .unwrap();
        let degraded = assembled.ports.readiness_report();
        assert!(!degraded.components["revision_stream"].ready);
        assert_eq!(
            degraded.components["revision_stream"].code.as_deref(),
            Some("revision_stream_unavailable")
        );

        drop(assembled);
        cleanup(&path);
    }

    #[tokio::test]
    async fn revision_binding_requires_a_real_opened_producer() {
        let path = temp_db("missing");
        let revision = SqliteRevisionStore::open(&path, 1, Duration::from_secs(1)).await;
        assert!(revision.is_err());
        assert!(!path.exists());
        cleanup(&path);
    }
}
