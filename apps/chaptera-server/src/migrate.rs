use crate::{cli::MigrateAction, db::MigrationRuntime, runtime_error::RuntimeError};

pub async fn run(
    action: MigrateAction,
    runtime: &dyn MigrationRuntime,
) -> Result<(), RuntimeError> {
    let report = match action {
        MigrateAction::Status => runtime.status().await?,
        MigrateAction::Up => runtime.up().await?,
    };

    let rendered = serde_json::to_string_pretty(&report).map_err(|error| {
        RuntimeError::new(
            "migration_report_serialization_failed",
            format!("could not serialize migration report: {error}"),
        )
    })?;
    println!("{rendered}");
    Ok(())
}
