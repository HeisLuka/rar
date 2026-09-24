use clap::{Parser, Subcommand};

use crate::build_info::BUILD_IDENTITY;

#[derive(Debug, Parser)]
#[command(
    name = "chaptera",
    about = "Chaptera Cloud server runtime shell",
    version = BUILD_IDENTITY,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the private HTTP runtime.
    Serve,
    /// Start the durable background worker runtime.
    Worker,
    /// Inspect or advance the durable schema.
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
    },
    /// Run read-only startup/operator diagnostics.
    Doctor,
}

#[derive(Debug, Clone, Copy, Subcommand)]
pub enum MigrateAction {
    Status,
    Up,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command, MigrateAction};

    #[test]
    fn parses_all_operator_commands() {
        assert!(matches!(
            Cli::try_parse_from(["chaptera", "serve"]).unwrap().command,
            Command::Serve
        ));
        assert!(matches!(
            Cli::try_parse_from(["chaptera", "worker"]).unwrap().command,
            Command::Worker
        ));
        assert!(matches!(
            Cli::try_parse_from(["chaptera", "doctor"]).unwrap().command,
            Command::Doctor
        ));
        assert!(matches!(
            Cli::try_parse_from(["chaptera", "migrate", "status"])
                .unwrap()
                .command,
            Command::Migrate {
                action: MigrateAction::Status
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["chaptera", "migrate", "up"])
                .unwrap()
                .command,
            Command::Migrate {
                action: MigrateAction::Up
            }
        ));
    }
}
