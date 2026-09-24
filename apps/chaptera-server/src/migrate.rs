use crate::{cli::MigrateAction, db::MigrationRuntime, runtime_error::RuntimeError};

pub fn run(action: MigrateAction, runtime: &dyn MigrationRuntime) -> Result<(), RuntimeError> {
    match action {
        MigrateAction::Status => runtime.status(),
        MigrateAction::Up => runtime.up(),
    }
}
