use anyhow::Result;

use crate::cli::{RefsCommand, RefsSyncArgs, SyncArgs};
use crate::commands::refs;
use crate::runner;
use crate::ui;
use crate::Context;

pub fn run(ctx: &Context, args: SyncArgs) -> Result<()> {
    ui::header("Syncing Environment");

    let targets: Vec<&str> = match &args.only {
        Some(only) => only.split(',').collect(),
        None => vec!["workspace", "refs"],
    };

    let total = targets.len();

    for (i, target) in targets.iter().enumerate() {
        let step = i + 1;

        match *target {
            "workspace" | "workspaces" | "ws" => {
                ui::step(step, total, "Syncing workspaces...");
                if args.dry_run {
                    ui::dim("Would run: workspace-setup sync");
                } else {
                    runner::run_script("workspace-setup", &["sync"])?;
                }
            }
            "refs" | "ref" => {
                ui::step(step, total, "Syncing reference repos...");
                if args.dry_run {
                    ui::dim("Would run: bossa refs sync");
                } else {
                    // Use native refs sync with parallel cloning
                    let refs_args = RefsSyncArgs {
                        name: None,
                        jobs: args.jobs,
                        retries: 3,
                        dry_run: false,
                    };
                    refs::run(ctx, RefsCommand::Sync(refs_args))?;
                }
            }
            "brew" | "packages" => {
                ui::step(step, total, "Syncing brew packages...");
                if args.dry_run {
                    ui::dim("Would run: brew-manager.sh apply");
                } else {
                    let home = dirs::home_dir().unwrap();
                    let script = home.join("dotfiles/scripts/brew/brew-manager.sh");
                    runner::run(script.to_str().unwrap(), &["apply"])?;
                }
            }
            other => {
                ui::warn(&format!("Unknown sync target: {}", other));
            }
        }
    }

    println!();
    ui::success("Sync complete!");
    Ok(())
}
