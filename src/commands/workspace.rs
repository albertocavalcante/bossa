use anyhow::Result;

use crate::cli::WorkspaceCommand;
use crate::config::WorkspacesConfig;
use crate::runner;
use crate::ui;
use crate::Context;

pub fn run(_ctx: &Context, cmd: WorkspaceCommand) -> Result<()> {
    match cmd {
        WorkspaceCommand::Sync { target } => sync(target),
        WorkspaceCommand::List => list(),
    }
}

fn sync(target: Option<String>) -> Result<()> {
    ui::header("Syncing Workspaces");

    let args: Vec<&str> = match &target {
        Some(t) => vec!["sync", t.as_str()],
        None => vec!["sync"],
    };

    runner::run_script("workspace-setup", &args)?;
    Ok(())
}

fn list() -> Result<()> {
    ui::header("Configured Workspaces");

    let config = WorkspacesConfig::load()?;

    for ws in &config.workspaces {
        println!();
        ui::info(&ws.name);
        ui::dim(&format!("  URL: {}", ws.url));

        if let Some(bare) = &ws.bare_dir {
            ui::dim(&format!("  Bare: {}", bare));
        }

        if !ws.worktrees.is_empty() {
            ui::dim("  Worktrees:");
            for wt in &ws.worktrees {
                ui::dim(&format!("    - {} -> {}", wt.branch, wt.path));
            }
        }
    }

    println!();
    ui::kv("Total", &config.workspaces.len().to_string());

    Ok(())
}
