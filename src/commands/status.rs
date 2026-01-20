use anyhow::Result;
use colored::Colorize;

use crate::Context;
use crate::config::{self, RefsConfig, WorkspacesConfig};
use crate::runner;
use crate::ui;

pub fn run(_ctx: &Context) -> Result<()> {
    ui::banner();
    ui::header("System Status");

    // Refs status
    show_refs_status()?;

    // Workspaces status
    show_workspaces_status()?;

    // Brew status
    show_brew_status()?;

    // T9 status
    show_t9_status()?;

    println!();
    Ok(())
}

fn show_refs_status() -> Result<()> {
    ui::section("📚 Reference Repos");

    match RefsConfig::load() {
        Ok(config) => {
            let root = config.root_path()?;
            let total = config.repositories.len();

            let existing = config
                .repositories
                .iter()
                .filter(|r| root.join(&r.name).exists())
                .count();

            let missing = total - existing;

            ui::kv("Config", &config_path("refs.json"));
            ui::kv("Root", &root.display().to_string());
            ui::kv(
                "Status",
                &format!(
                    "{} total, {} {} cloned, {} {} missing",
                    total.to_string().bold(),
                    existing.to_string().green(),
                    "✓".green(),
                    if missing > 0 {
                        missing.to_string().yellow()
                    } else {
                        missing.to_string().dimmed()
                    },
                    if missing > 0 { "⚠" } else { "✓" }
                ),
            );
        }
        Err(_) => {
            ui::kv("Status", &"Not configured".yellow().to_string());
            ui::dim("Run: bossa refs snapshot");
        }
    }

    Ok(())
}

fn show_workspaces_status() -> Result<()> {
    ui::section("🏗️  Workspaces");

    match WorkspacesConfig::load() {
        Ok(config) => {
            let ws_dir = config::workspaces_dir()?;
            let total = config.workspaces.len();

            ui::kv("Config", &config_path("workspaces.json"));
            ui::kv("Root", &ws_dir.display().to_string());
            ui::kv("Workspaces", &format!("{}", total));

            for ws in &config.workspaces {
                let path = ws_dir.join(&ws.name);
                let exists = path.exists();
                let status = if exists {
                    "✓".green().to_string()
                } else {
                    "✗".yellow().to_string()
                };
                println!("    {} {}", status, ws.name);
            }
        }
        Err(_) => {
            ui::kv("Status", &"Not configured".yellow().to_string());
        }
    }

    Ok(())
}

fn show_brew_status() -> Result<()> {
    ui::section("🍺 Homebrew Packages");

    if runner::command_exists("brew") {
        // Count installed packages
        let formulas = runner::run_capture("brew", &["list", "--formula", "-1"])
            .map(|s| s.lines().count())
            .unwrap_or(0);

        let casks = runner::run_capture("brew", &["list", "--cask", "-1"])
            .map(|s| s.lines().count())
            .unwrap_or(0);

        ui::kv(
            "Installed",
            &format!("{} formulas, {} casks", formulas, casks),
        );

        // Check if Brewfile exists
        let brewfile = dirs::home_dir()
            .map(|h| h.join("dotfiles/scripts/brew/Brewfile"))
            .filter(|p| p.exists());

        if let Some(path) = brewfile {
            ui::kv("Brewfile", &path.display().to_string());
        }
    } else {
        ui::kv("Status", &"Homebrew not installed".yellow().to_string());
    }

    Ok(())
}

fn show_t9_status() -> Result<()> {
    ui::section("💾 T9 External Drive");

    let t9_path = std::path::Path::new("/Volumes/T9");
    let refs_link = dirs::home_dir().map(|h| h.join("dev/refs"));

    if t9_path.exists() {
        ui::kv("Mount", &format!("{} {}", "✓".green(), "/Volumes/T9"));

        // Check if refs is symlinked to T9
        if let Some(refs) = refs_link {
            if refs.is_symlink() {
                if let Ok(target) = std::fs::read_link(&refs) {
                    if target.to_string_lossy().contains("T9") {
                        ui::kv(
                            "Refs symlink",
                            &format!("{} -> {}", "✓".green(), target.display()),
                        );
                    }
                }
            }
        }
    } else {
        ui::kv("Mount", &format!("{} {}", "✗".yellow(), "Not mounted"));
    }

    Ok(())
}

fn config_path(name: &str) -> String {
    if let Ok(dir) = config::config_dir() {
        dir.join(name).display().to_string()
    } else {
        format!("~/.config/workspace-setup/{}", name)
    }
}
