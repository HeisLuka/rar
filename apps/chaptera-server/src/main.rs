use std::{error::Error, process::ExitCode, time::Duration};

use chaptera_server::{
    auth_runtime::AuthRuntime,
    cli::{Cli, Command},
    config::{ChapteraConfig, SecretResolver},
    doctor,
    edge::EdgePolicy,
    jobs::UnconfiguredWorkerRuntime,
    migrate,
    runtime_readiness::{ports_with_revision_stream, ports_with_revision_stream_and_authn},
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

            let edge_policy = EdgePolicy::from_config(&config)?;
            if explicit_config.is_some() {
                let revision_stream = SqliteRevisionStore::open(
                    &config.sqlite.path,
                    config.sqlite.pool_max,
                    Duration::from_millis(config.sqlite.busy_timeout_ms),
                )
                .await?;
                if config.auth.is_some() {
                    let auth_runtime = AuthRuntime::open(&config, &secrets).await?;
                    drop(secrets);
                    let auth_http = auth_runtime.http_state();
                    let assembled =
                        ports_with_revision_stream_and_authn(revision_stream, auth_runtime);
                    let state = AppState::new(assembled.ports);
                    serve::run_with_auth(
                        config.runtime_config(),
                        edge_policy,
                        state,
                        Some(auth_http),
                    )
                    .await?;
                } else {
                    drop(secrets);
                    let assembled = ports_with_revision_stream(revision_stream);
                    let state = AppState::new(assembled.ports);
                    serve::run(config.runtime_config(), edge_policy, state).await?;
                }
            } else {
                drop(secrets);
                let state = AppState::new(RuntimePorts::unconfigured());
                serve::run(config.runtime_config(), edge_policy, state).await?;
            }
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
            doctor::run(&AppState::new(RuntimePorts::unconfigured()))?;
        }
    }

    Ok(())
}
