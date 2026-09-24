use async_trait::async_trait;

use crate::runtime_error::RuntimeError;

#[async_trait(?Send)]
pub trait MigrationRuntime: Send + Sync {
    async fn status(&self) -> Result<(), RuntimeError>;
    async fn up(&self) -> Result<(), RuntimeError>;
}

pub struct UnconfiguredMigrationRuntime;

#[async_trait(?Send)]
impl MigrationRuntime for UnconfiguredMigrationRuntime {
    async fn status(&self) -> Result<(), RuntimeError> {
        Err(RuntimeError::new(
            "migration_runtime_not_configured",
            "CLOUD-MIGRATION-01 is not connected to the runtime shell",
        ))
    }

    async fn up(&self) -> Result<(), RuntimeError> {
        Err(RuntimeError::new(
            "migration_runtime_not_configured",
            "CLOUD-MIGRATION-01 is not connected to the runtime shell",
        ))
    }
}
