use std::{error::Error, process::ExitCode, time::Duration};

use chaptera_server::{
    cli::{Cli, Command},
    config::{ChapteraConfig, SecretResolver},
    doctor,
    jobs::UnconfiguredWorkerRuntime,
    migrate,
    runtime_readiness::ports_with_revision_stream,
    schema_migration::SqliteMigrationRuntime,
    serve,
    sqlite_store::SqliteRevisionStore,
    state::{AppState, RuntimePorts},
    worker,
};
use clap::Parser;

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("chaptera: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    let explicit_config = cli
        .config
        .as_deref()
        .map(ChapteraConfig::load)
        .transpose()?;

    match cli.command {
        Command::Serve => {
            let config = match explicit_config.as_ref() {
                Some(config) => config.clone(),
                None => ChapteraConfig::development_from_env()?,
            };
            let secrets = config.resolve_required_secrets(&SecretResolver::from_process())?;
            drop(secrets);

            let revision_stream = SqliteRevisionStore::open(
                &config.sqlite.path,
                config.sqlite.pool_max,
                Duration::from_millis(config.sqlite.busy_timeout_ms),
            )
            .await?;
            let assembled = ports_with_revision_stream(revision_stream);
            let _revision_readiness = assembled.readiness;
            let state = AppState::new(assembled.ports);

            serve::run(config.runtime_config(), state).await?;
        }
        Command::Worker => {
            worker::run(&UnconfiguredWorkerRuntime)?;
        }
        Command::Migrate { action } => {
            if let Some(config) = explicit_config.as_ref() {
                let runtime = SqliteMigrationRuntime::new(
                    &config.sqlite.path,
                    Duration::from_millis(config.sqlite.busy_timeout_ms),
                )?;
                migrate::run(action, &runtime).await?;
            } else {
                let runtime = SqliteMigrationRuntime::from_env()?;
                migrate::run(action, &runtime).await?;
            }
        }
        Command::Doctor => {
            if let Some(config) = explicit_config.as_ref() {
                let secrets = config.resolve_required_secrets(&SecretResolver::from_process())?;
                drop(secrets);
            }
            let state = AppState::new(RuntimePorts::unconfigured());
            doctor::run(&state)?;
        }
    }

    Ok(())
}
