use crate::{cli::MigrateAction, db::MigrationRuntime, runtime_error::RuntimeError};

pub async fn run(
    action: MigrateAction,
    runtime: &dyn MigrationRuntime,
) -> Result<(), RuntimeError> {
    match action {
        MigrateAction::Status => runtime.status().await,
        MigrateAction::Up => runtime.up().await,
    }
}
