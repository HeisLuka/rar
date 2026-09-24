use std::{error::Error, process::ExitCode};

use chaptera_server::{
    cli::{Cli, Command},
    config::RuntimeConfig,
    db::UnconfiguredMigrationRuntime,
    doctor,
    jobs::UnconfiguredWorkerRuntime,
    migrate, serve,
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
    let state = AppState::new(RuntimePorts::unconfigured());

    match cli.command {
        Command::Serve => {
            let config = RuntimeConfig::from_env()?;
            serve::run(config, state).await?;
        }
        Command::Worker => {
            worker::run(&UnconfiguredWorkerRuntime)?;
        }
        Command::Migrate { action } => {
            migrate::run(action, &UnconfiguredMigrationRuntime)?;
        }
        Command::Doctor => {
            doctor::run(&state)?;
        }
    }

    Ok(())
}
