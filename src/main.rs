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
use cli::{AddCommand, Cli, Command, RmCommand, CollectionsCommand, ManifestCommand, RefsCommand};
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
        Command::Caches(cmd) => commands::caches::run(cmd),
        Command::Collections(cmd) => {
            let collections_cmd = match cmd {
                CollectionsCommand::List => commands::collections::CollectionsCommand::List,
                CollectionsCommand::Status { name } => commands::collections::CollectionsCommand::Status { name },
                CollectionsCommand::Sync { name, jobs, retries, dry_run } => {
                    commands::collections::CollectionsCommand::Sync { name, jobs, retries, dry_run }
                }
                CollectionsCommand::Audit { name, fix } => {
                    commands::collections::CollectionsCommand::Audit { name, fix }
                }
                CollectionsCommand::Snapshot { name } => {
                    commands::collections::CollectionsCommand::Snapshot { name }
                }
                CollectionsCommand::Add { collection, url, name, clone } => {
                    commands::collections::CollectionsCommand::Add { collection, url, name, clone }
                }
                CollectionsCommand::Rm { collection, repo, delete } => {
                    commands::collections::CollectionsCommand::Rm { collection, repo, delete }
                }
                CollectionsCommand::Clean { name, yes, dry_run } => {
                    commands::collections::CollectionsCommand::Clean { name, yes, dry_run }
                }
            };
            commands::collections::run(&ctx, collections_cmd)
        }
        Command::Manifest(cmd) => {
            let manifest_cmd = match cmd {
                ManifestCommand::Scan { path, force } => {
                    commands::manifest::ManifestCommand::Scan { path, force }
                }
                ManifestCommand::Stats { path } => {
                    commands::manifest::ManifestCommand::Stats { path }
                }
                ManifestCommand::Duplicates { path, min_size, delete } => {
                    commands::manifest::ManifestCommand::Duplicates { path, min_size, delete }
                }
            };
            commands::manifest::run(manifest_cmd)
        }
        Command::Refs(cmd) => {
            // Show deprecation warning
            ui::warn("'bossa refs' is deprecated. Use 'bossa collections <subcommand> refs' instead.");
            println!();

            // Forward to collections command with "refs" collection name
            let collections_cmd = match cmd {
                RefsCommand::Sync(args) => {
                    let name = args.name.unwrap_or_else(|| "refs".to_string());
                    commands::collections::CollectionsCommand::Sync {
                        name,
                        jobs: args.jobs,
                        retries: args.retries,
                        dry_run: args.dry_run,
                    }
                }
                RefsCommand::List { filter: _, missing: _ } => {
                    // For list, just show status of refs collection
                    commands::collections::CollectionsCommand::Status {
                        name: "refs".to_string(),
                    }
                }
                RefsCommand::Snapshot => {
                    commands::collections::CollectionsCommand::Snapshot {
                        name: "refs".to_string(),
                    }
                }
                RefsCommand::Audit { fix } => {
                    commands::collections::CollectionsCommand::Audit {
                        name: "refs".to_string(),
                        fix,
                    }
                }
                RefsCommand::Add { url, name, clone } => {
                    commands::collections::CollectionsCommand::Add {
                        collection: "refs".to_string(),
                        url,
                        name,
                        clone,
                    }
                }
                RefsCommand::Remove { name, delete } => {
                    commands::collections::CollectionsCommand::Rm {
                        collection: "refs".to_string(),
                        repo: name,
                        delete,
                    }
                }
            };
            commands::collections::run(&ctx, collections_cmd)
        }
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "bossa", &mut io::stdout());
            Ok(())
        }
    }
}
