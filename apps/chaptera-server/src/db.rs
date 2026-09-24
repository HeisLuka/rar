use serde::Serialize;

use crate::runtime_error::RuntimeError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationReport {
    pub applied_version: i64,
    pub supported_version: i64,
    pub pending_versions: Vec<i64>,
}

#[async_trait::async_trait]
pub trait MigrationRuntime: Send + Sync {
    async fn status(&self) -> Result<MigrationReport, RuntimeError>;
    async fn up(&self) -> Result<MigrationReport, RuntimeError>;
}

pub struct UnconfiguredMigrationRuntime;

#[async_trait::async_trait]
impl MigrationRuntime for UnconfiguredMigrationRuntime {
    async fn status(&self) -> Result<MigrationReport, RuntimeError> {
        Err(RuntimeError::new(
            "migration_runtime_not_configured",
            "CLOUD-MIGRATION-01 is not connected to the runtime shell",
        ))
    }

    async fn up(&self) -> Result<MigrationReport, RuntimeError> {
        Err(RuntimeError::new(
            "migration_runtime_not_configured",
            "CLOUD-MIGRATION-01 is not connected to the runtime shell",
        ))
    }
}
