use std::{collections::BTreeMap, sync::Arc};

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyFailure {
    pub code: &'static str,
    pub message: String,
}

impl DependencyFailure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub trait RuntimeDependency: Send + Sync {
    fn check(&self) -> Result<(), DependencyFailure>;
}

#[derive(Clone)]
pub struct RuntimePorts {
    pub authn: Arc<dyn RuntimeDependency>,
    pub authz: Arc<dyn RuntimeDependency>,
    pub revision_stream: Arc<dyn RuntimeDependency>,
    pub jobs: Arc<dyn RuntimeDependency>,
    pub blob_store: Arc<dyn RuntimeDependency>,
    pub observability: Arc<dyn RuntimeDependency>,
}

impl RuntimePorts {
    pub fn new(
        authn: Arc<dyn RuntimeDependency>,
        authz: Arc<dyn RuntimeDependency>,
        revision_stream: Arc<dyn RuntimeDependency>,
        jobs: Arc<dyn RuntimeDependency>,
        blob_store: Arc<dyn RuntimeDependency>,
        observability: Arc<dyn RuntimeDependency>,
    ) -> Self {
        Self {
            authn,
            authz,
            revision_stream,
            jobs,
            blob_store,
            observability,
        }
    }

    pub fn unconfigured() -> Self {
        Self::new(
            unconfigured("CLOUD-AUTHN-01"),
            unconfigured("CLOUD-AUTHZ-01"),
            unconfigured("WEB-DURABILITY-WAL-01 / CLOUD-SQLITE-V0-01"),
            unconfigured("CLOUD-ASYNC-RUNTIME-01"),
            unconfigured("CLOUD-BLOB-STORE-V0-01"),
            unconfigured("CLOUD-OBSERVABILITY-01"),
        )
    }

    pub fn readiness_report(&self) -> ReadinessReport {
        let mut components = BTreeMap::new();
        components.insert("authn".to_owned(), snapshot(&self.authn, true));
        components.insert("authz".to_owned(), snapshot(&self.authz, true));
        components.insert(
            "revision_stream".to_owned(),
            snapshot(&self.revision_stream, true),
        );
        components.insert("jobs".to_owned(), snapshot(&self.jobs, true));
        components.insert("blob_store".to_owned(), snapshot(&self.blob_store, true));
        components.insert(
            "observability".to_owned(),
            snapshot(&self.observability, false),
        );

        let ready = components
            .values()
            .filter(|component| component.required)
            .all(|component| component.ready);

        ReadinessReport {
            status: if ready { "ready" } else { "not_ready" }.to_owned(),
            ready,
            components,
        }
    }
}

fn unconfigured(gate: &'static str) -> Arc<dyn RuntimeDependency> {
    Arc::new(UnconfiguredDependency { gate })
}

fn snapshot(port: &Arc<dyn RuntimeDependency>, required: bool) -> ComponentStatus {
    match port.check() {
        Ok(()) => ComponentStatus {
            required,
            ready: true,
            code: None,
            message: None,
        },
        Err(error) => ComponentStatus {
            required,
            ready: false,
            code: Some(error.code.to_owned()),
            message: Some(error.message),
        },
    }
}

struct UnconfiguredDependency {
    gate: &'static str,
}

impl RuntimeDependency for UnconfiguredDependency {
    fn check(&self) -> Result<(), DependencyFailure> {
        Err(DependencyFailure::new(
            "not_configured",
            format!(
                "producer {} is not connected to the runtime shell",
                self.gate
            ),
        ))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ComponentStatus {
    pub required: bool,
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReadinessReport {
    pub status: String,
    pub ready: bool,
    pub components: BTreeMap<String, ComponentStatus>,
}

#[derive(Clone)]
pub struct AppState {
    ports: RuntimePorts,
}

impl AppState {
    pub fn new(ports: RuntimePorts) -> Self {
        Self { ports }
    }

    pub fn readiness_report(&self) -> ReadinessReport {
        self.ports.readiness_report()
    }
}
