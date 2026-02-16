mod cli;
mod commands;
mod config;
mod engine;
mod generators;
mod paths;
mod progress;
mod resource;
mod runner;
mod scanner;
mod schema;
mod state;
mod sudo;
mod ui;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use cli::{AddCommand, Cli, Command, RmCommand, StorageCommand};
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
        Command::Dotfiles(cmd) => commands::dotfiles::run(&ctx, cmd),
        Command::Doctor => commands::doctor::run(&ctx),
        Command::Migrate { dry_run } => commands::migrate::run(&ctx, dry_run),
        Command::Caches(cmd) => commands::caches::run(cmd),
        Command::Collections(cmd) => commands::collections::run(&ctx, cmd.into()),
        Command::Manifest(cmd) => commands::manifest::run(cmd.into()),
        Command::ICloud(cmd) => commands::icloud::run(cmd.into()),
        Command::Storage(cmd) => match cmd {
            StorageCommand::Status => commands::storage::status(),
            StorageCommand::Duplicates {
                manifests,
                list,
                min_size,
                limit,
            } => commands::storage::duplicates(&manifests, list, min_size, limit),
        },
        Command::Disk(cmd) => commands::disk::run(cmd.into()),
        Command::Brew(cmd) => commands::brew::run(&ctx, cmd),
        Command::Refs(cmd) => {
            // Show deprecation warning
            ui::warn(
                "'bossa refs' is deprecated. Use 'bossa collections <subcommand> refs' instead.",
            );
            println!();

            commands::collections::run(&ctx, cmd.into())
        }
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "bossa", &mut io::stdout());
            Ok(())
        }
        Command::Tools(cmd) => commands::tools::run(&ctx, cmd),
        Command::Stow(cmd) => commands::stow::run(&ctx, cmd),
        Command::Theme(cmd) => commands::theme::run(&ctx, cmd),
        Command::Defaults(cmd) => commands::defaults::run(&ctx, cmd),
        Command::Locations(cmd) => commands::locations::run(&ctx, cmd),
        Command::Configs(cmd) => commands::configs::run(&ctx, cmd),
        Command::Relocate(cmd) => commands::relocate::run(&ctx, cmd),
    }
}
