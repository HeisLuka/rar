use crate::runtime_error::RuntimeError;

pub trait MigrationRuntime: Send + Sync {
    fn status(&self) -> Result<(), RuntimeError>;
    fn up(&self) -> Result<(), RuntimeError>;
}

pub struct UnconfiguredMigrationRuntime;

impl MigrationRuntime for UnconfiguredMigrationRuntime {
    fn status(&self) -> Result<(), RuntimeError> {
        Err(RuntimeError::new(
            "migration_runtime_not_configured",
            "CLOUD-MIGRATION-01 is not connected to the runtime shell",
        ))
    }

    fn up(&self) -> Result<(), RuntimeError> {
        Err(RuntimeError::new(
            "migration_runtime_not_configured",
            "CLOUD-MIGRATION-01 is not connected to the runtime shell",
        ))
    }
}
