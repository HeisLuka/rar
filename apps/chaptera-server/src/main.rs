use std::{error::Error, process::ExitCode, time::Duration};

use chaptera_server::{
    cli::{Cli, Command},
    config::{ChapteraConfig, SecretResolver},
    doctor,
    edge::EdgePolicy,
    jobs::UnconfiguredWorkerRuntime,
    migrate,
    runtime_readiness::ports_with_revision_stream,
    schema_migration::SqliteMigrationRuntime,
    serve,
    sqlite_store::SqliteRevisionStore,
    state::{AppState, RuntimePorts},
    worker,
    worker_runtime::ConfiguredWorkerRuntime,
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

            let state = if explicit_config.is_some() {
                let revision_stream = SqliteRevisionStore::open(
                    &config.sqlite.path,
                    config.sqlite.pool_max,
                    Duration::from_millis(config.sqlite.busy_timeout_ms),
                )
                .await?;
                let assembled = ports_with_revision_stream(revision_stream);
                AppState::new(assembled.ports)
            } else {
                AppState::new(RuntimePorts::unconfigured())
            };

            let edge_policy = EdgePolicy::from_config(&config)?;
            serve::run(config.runtime_config(), edge_policy, state).await?;
        }
        Command::Worker => {
            if let Some(config) = explicit_config.as_ref() {
                let runtime = ConfiguredWorkerRuntime::new(config.clone());
                worker::run(&runtime).await?;
            } else {
                worker::run(&UnconfiguredWorkerRuntime).await?;
            }
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
            doctor::run(&AppState::new(RuntimePorts::unconfigured()))?;
        }
    }

    Ok(())
}
