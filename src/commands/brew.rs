use anyhow::Result;
use std::path::PathBuf;

use crate::Context;
use crate::cli::BrewCommand;
use crate::progress;
use crate::runner;
use crate::ui;

pub fn run(_ctx: &Context, cmd: BrewCommand) -> Result<()> {
    match cmd {
        BrewCommand::Apply { essential } => apply(essential),
        BrewCommand::Capture => capture(),
        BrewCommand::Audit => audit(),
    }
}

fn brew_manager_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join("dotfiles/scripts/brew/brew-manager.sh"))
        .unwrap_or_else(|| PathBuf::from("brew-manager.sh"))
}

fn apply(essential: bool) -> Result<()> {
    let cmd = if essential {
        ui::header("Installing Essential Packages");
        "apply-essential"
    } else {
        ui::header("Installing All Packages");
        "apply"
    };

    let script = brew_manager_path();
    if !script.exists() {
        ui::error(&format!(
            "brew-manager.sh not found at {}",
            script.display()
        ));
        ui::info("Make sure dotfiles are properly installed");
        return Ok(());
    }

    runner::run(script.to_str().unwrap(), &[cmd])?;
    Ok(())
}

fn capture() -> Result<()> {
    ui::header("Capturing Brew Packages");

    let script = brew_manager_path();
    if !script.exists() {
        ui::error(&format!(
            "brew-manager.sh not found at {}",
            script.display()
        ));
        return Ok(());
    }

    let pb = progress::spinner("Capturing installed packages...");
    let result = runner::run(script.to_str().unwrap(), &["capture"]);

    match result {
        Ok(_) => {
            progress::finish_success(&pb, "Brewfile updated");
        }
        Err(e) => {
            progress::finish_error(&pb, &format!("Capture failed: {}", e));
        }
    }

    Ok(())
}

fn audit() -> Result<()> {
    ui::header("Brew Audit - Drift Detection");

    let script = brew_manager_path();
    if !script.exists() {
        ui::error(&format!(
            "brew-manager.sh not found at {}",
            script.display()
        ));
        return Ok(());
    }

    runner::run(script.to_str().unwrap(), &["audit"])?;
    Ok(())
}
