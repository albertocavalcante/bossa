mod cli;
mod commands;
mod config;
mod progress;
mod runner;
mod ui;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use cli::{Cli, Commands};
use std::io;

/// Global context for the application
pub struct Context {
    pub verbose: u8,
    pub quiet: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging based on verbosity
    let log_level = match cli.verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        2 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };

    env_logger::Builder::new()
        .filter_level(if cli.quiet {
            log::LevelFilter::Error
        } else {
            log_level
        })
        .format_timestamp(None)
        .init();

    let ctx = Context {
        verbose: cli.verbose,
        quiet: cli.quiet,
    };

    match cli.command {
        Commands::Status => commands::status::run(&ctx),
        Commands::Sync(args) => commands::sync::run(&ctx, args),
        Commands::Refs(cmd) => commands::refs::run(&ctx, cmd),
        Commands::Brew(cmd) => commands::brew::run(&ctx, cmd),
        Commands::Workspace(cmd) => commands::workspace::run(&ctx, cmd),
        Commands::Worktree(cmd) => commands::worktree::run(&ctx, cmd),
        Commands::T9(cmd) => commands::t9::run(&ctx, cmd),
        Commands::Doctor => commands::doctor::run(&ctx),
        Commands::Nova(args) => commands::nova::run(&ctx, args),
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "bossa", &mut io::stdout());
            Ok(())
        }
        Commands::Config(cmd) => commands::config::run(&ctx, cmd),
    }
}
