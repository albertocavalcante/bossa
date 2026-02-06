//! Configs command - manage generated configuration files

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;

use crate::Context as AppContext;
use crate::cli::ConfigsCommand;
use crate::generators;
use crate::schema::BossaConfig;
use crate::ui;

pub fn run(ctx: &AppContext, cmd: ConfigsCommand) -> Result<()> {
    match cmd {
        ConfigsCommand::Apply { config, dry_run } => apply(ctx, config.as_deref(), dry_run),
        ConfigsCommand::Diff { config } => diff(ctx, config.as_deref()),
        ConfigsCommand::Status => status(ctx),
        ConfigsCommand::Show { config } => show(ctx, &config),
    }
}

fn apply(_ctx: &AppContext, config_name: Option<&str>, dry_run: bool) -> Result<()> {
    let config = BossaConfig::load()?;

    match config_name {
        Some("git") | None => {
            if let Some(ref git_config) = config.configs.git {
                apply_git(&config, git_config, dry_run)?;
            } else if config_name.is_some() {
                anyhow::bail!("No git config defined in config.toml. Add [configs.git] section.");
            }
        }
        Some(name) => {
            anyhow::bail!("Unknown config '{}'. Available: git", name);
        }
    }

    Ok(())
}

fn apply_git(
    config: &BossaConfig,
    git_config: &crate::schema::GitConfig,
    dry_run: bool,
) -> Result<()> {
    ui::header("Git Config");

    let target = generators::git::target_path(git_config, &config.locations);
    let content = generators::git::generate(git_config, &config.locations)?;

    println!("  Target: {}", target.display());

    if dry_run {
        println!();
        println!("{}", "Would generate:".yellow());
        println!("{}", "─".repeat(60).dimmed());
        println!("{}", content);
        println!("{}", "─".repeat(60).dimmed());
        println!();
        println!("{}", "Dry run - no changes made.".dimmed());
    } else {
        // Create parent directories if needed
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Backup existing file
        if target.exists() {
            let backup = target.with_extension("gitconfig.bak");
            fs::copy(&target, &backup)
                .with_context(|| format!("Failed to backup to {}", backup.display()))?;
            println!("  {} Backed up to {}", "→".dimmed(), backup.display());
        }

        fs::write(&target, &content)
            .with_context(|| format!("Failed to write {}", target.display()))?;

        println!("  {} Generated {}", "✓".green(), target.display());
    }

    Ok(())
}

fn diff(_ctx: &AppContext, config_name: Option<&str>) -> Result<()> {
    let config = BossaConfig::load()?;

    match config_name {
        Some("git") | None => {
            if let Some(ref git_config) = config.configs.git {
                diff_git(&config, git_config)?;
            } else if config_name.is_some() {
                anyhow::bail!("No git config defined in config.toml");
            }
        }
        Some(name) => {
            anyhow::bail!("Unknown config '{}'. Available: git", name);
        }
    }

    Ok(())
}

fn diff_git(config: &BossaConfig, git_config: &crate::schema::GitConfig) -> Result<()> {
    ui::header("Git Config Diff");

    match generators::git::diff(git_config, &config.locations)? {
        Some(diff_output) => {
            println!("{}", diff_output);
        }
        None => {
            println!("{}", "No changes - config is up to date.".green());
        }
    }

    Ok(())
}

fn status(_ctx: &AppContext) -> Result<()> {
    let config = BossaConfig::load()?;

    ui::header("Config Status");

    if let Some(ref git_config) = config.configs.git {
        let target = generators::git::target_path(git_config, &config.locations);
        let exists = target.exists();
        let icon = if exists {
            "✓".green()
        } else {
            "○".yellow()
        };
        let status = if exists { "exists" } else { "not created" };

        println!("  {} git: {} ({})", icon, target.display(), status);

        if exists {
            // Check if it matches
            if let Ok(Some(_)) = generators::git::diff(git_config, &config.locations) {
                println!(
                    "      {} out of sync - run 'bossa configs apply git'",
                    "⚠".yellow()
                );
            } else {
                println!("      {} in sync", "✓".green());
            }
        }
    } else {
        println!("  {} git: not configured", "○".dimmed());
    }

    Ok(())
}

fn show(_ctx: &AppContext, config_name: &str) -> Result<()> {
    let config = BossaConfig::load()?;

    match config_name {
        "git" => {
            if let Some(ref git_config) = config.configs.git {
                let content = generators::git::generate(git_config, &config.locations)?;
                println!("{}", content);
            } else {
                anyhow::bail!("No git config defined in config.toml. Add [configs.git] section.");
            }
        }
        name => {
            anyhow::bail!("Unknown config '{}'. Available: git", name);
        }
    }

    Ok(())
}
