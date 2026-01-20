use anyhow::Result;

use crate::Context;
use crate::cli::WorktreeCommand;
use crate::runner;
use crate::ui;

pub fn run(_ctx: &Context, cmd: WorktreeCommand) -> Result<()> {
    match cmd {
        WorktreeCommand::Status => status(),
        WorktreeCommand::New { branch, slot, push } => new(&branch, slot, push),
        WorktreeCommand::Release { slot, force } => release(&slot, force),
        WorktreeCommand::Cleanup { force, dry_run } => cleanup(force, dry_run),
    }
}

fn status() -> Result<()> {
    // wt status is the default command
    runner::run_script("wt", &["status"])?;
    Ok(())
}

fn new(branch: &str, slot: Option<String>, push: bool) -> Result<()> {
    ui::header(&format!("Creating Worktree: {}", branch));

    let mut args = vec!["new", branch];

    if let Some(s) = &slot {
        args.push("--slot");
        args.push(s);
    }

    if push {
        args.push("--push");
    }

    runner::run_script("wt", &args)?;
    Ok(())
}

fn release(slot: &str, force: bool) -> Result<()> {
    ui::header(&format!("Releasing Slot: {}", slot));

    let mut args = vec!["release", slot];

    if force {
        args.push("--force");
    }

    runner::run_script("wt", &args)?;
    Ok(())
}

fn cleanup(force: bool, dry_run: bool) -> Result<()> {
    ui::header("Cleaning Up Worktrees");

    let mut args = vec!["cleanup"];

    if force {
        args.push("--force");
    }

    if dry_run {
        args.push("--dry-run");
    }

    runner::run_script("wt", &args)?;
    Ok(())
}
