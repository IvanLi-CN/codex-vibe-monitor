mod checks;
mod config;
mod model;
mod parsers;
mod ratchet;
mod repository;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::model::Report;

#[derive(Debug, Parser)]
#[command(
    name = "source-structure-check",
    version,
    about = "Enforce the source structure contract"
)]
struct Cli {
    #[arg(long, global = true, value_name = "DIR")]
    repo_root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Baseline {
        #[arg(long, help = "Use staged index contents instead of the worktree")]
        staged: bool,
    },
    Check {
        #[arg(long, conflicts_with = "all", help = "Check staged index contents")]
        staged: bool,
        #[arg(
            long,
            conflicts_with = "staged",
            help = "Check every tracked source file"
        )]
        all: bool,
    },
    RequireZero {
        #[arg(long, help = "Check every tracked source file")]
        all: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            let report = Report::error(error.to_string());
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_else(|_| {
                    "{\"status\":\"error\",\"errors\":[\"report serialization failed\"]}".to_owned()
                })
            );
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let repo_root = repository::resolve_repo_root(cli.repo_root.as_deref())?;
    let config = config::load(&repo_root)?;

    match cli.command {
        Command::Baseline { staged } => ratchet::baseline(&repo_root, &config, staged),
        Command::Check { staged, all: _ } => ratchet::check(&repo_root, &config, staged),
        Command::RequireZero { all: _ } => ratchet::require_zero(&repo_root, &config, false),
    }
}
