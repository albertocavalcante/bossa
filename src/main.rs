mod cli;
mod commands;
mod config;
mod engine;
mod progress;
mod resource;
mod runner;
mod schema;
mod state;
mod sudo;
mod ui;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use cli::{AddCommand, Cli, Command, RmCommand};
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
        Command::Nova(args) => commands::nova::run(&ctx, args),
        Command::Status(args) => commands::declarative::status(&ctx, args.target.as_deref()),
        Command::Apply(args) => commands::declarative::apply(
            &ctx,
            args.target.as_deref(),
            args.dry_run,
            args.jobs as usize,
        ),
        Command::Diff(args) => commands::declarative::diff(&ctx, args.target.as_deref()),
        Command::Add(cmd) => match cmd {
            AddCommand::Collection {
                name,
                path,
                description,
            } => commands::crud::add_collection(&ctx, &name, &path, description.as_deref()),
            AddCommand::Repo {
                collection,
                url,
                name,
            } => commands::crud::add_repo(&ctx, &collection, &url, name.as_deref()),
            AddCommand::Workspace {
                url,
                name,
                category,
            } => commands::crud::add_workspace(&ctx, &url, name.as_deref(), category.as_deref()),
            AddCommand::Storage {
                name,
                mount,
                storage_type,
            } => commands::crud::add_storage(&ctx, &name, &mount, storage_type.as_deref()),
        },
        Command::Rm(cmd) => match cmd {
            RmCommand::Collection { name } => commands::crud::rm_collection(&ctx, &name),
            RmCommand::Repo { collection, name } => {
                commands::crud::rm_repo(&ctx, &collection, &name)
            }
            RmCommand::Workspace { name } => commands::crud::rm_workspace(&ctx, &name),
            RmCommand::Storage { name } => commands::crud::rm_storage(&ctx, &name),
        },
        Command::List(args) => commands::crud::list(&ctx, args.resource_type),
        Command::Show(args) => commands::crud::show(&ctx, &args.target),
        Command::Doctor => commands::doctor::run(&ctx),
        Command::Migrate { dry_run } => commands::migrate::run(&ctx, dry_run),
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "bossa", &mut io::stdout());
            Ok(())
        }
    }
}
