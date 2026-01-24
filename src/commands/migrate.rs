//! Migrate command - convert old config format to new unified format

#![allow(dead_code)]

use anyhow::{Context, Result};
use colored::Colorize;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

use crate::Context as AppContext;
use crate::config;
use crate::schema::{BossaConfig, Collection, CollectionRepo};
use crate::ui;

pub fn run(ctx: &AppContext, dry_run: bool) -> Result<()> {
    ui::header("Config Migration");
    println!();

    let legacy_dir = config::legacy_config_dir()?;
    let new_dir = config::config_dir()?;

    // Check for legacy configs in both legacy dir and bossa config dir
    let legacy_refs_path = legacy_dir.join("refs.json");
    let bossa_refs_path = new_dir.join("refs.json");
    let ws_path = legacy_dir.join("workspaces.json");

    // Prefer refs.json in bossa config dir, fall back to legacy dir
    let refs_path = if bossa_refs_path.exists() {
        bossa_refs_path
    } else {
        legacy_refs_path
    };

    let has_refs = refs_path.exists();
    let has_ws = ws_path.exists();

    if !has_refs && !has_ws {
        ui::info("No legacy configs found to migrate");
        return Ok(());
    }

    // Load existing new config or create default
    let mut config = match config::load_config::<BossaConfig>(&new_dir, "config") {
        Ok((c, _)) => c,
        Err(_) => BossaConfig::default(),
    };

    // Migrate refs.json
    if has_refs {
        ui::section("Migrating refs.json");
        migrate_refs(&refs_path, &mut config, ctx)?;
    }

    // Migrate workspaces.json
    if has_ws {
        ui::section("Migrating workspaces.json");
        migrate_workspaces(&ws_path, &mut config, ctx)?;
    }

    if dry_run {
        ui::warn("Dry run - no changes made");
        println!();
        println!("Would write to: {}", new_dir.join("config.toml").display());

        // Show preview
        let toml_str = toml::to_string_pretty(&config)?;
        println!();
        println!("{}", "Preview:".bold());
        for line in toml_str.lines().take(50) {
            println!("  {}", line.dimmed());
        }
        if toml_str.lines().count() > 50 {
            println!("  ... ({} more lines)", toml_str.lines().count() - 50);
        }
    } else {
        // Save new config
        fs::create_dir_all(&new_dir)?;
        let config_path = new_dir.join("config.toml");
        let toml_str = toml::to_string_pretty(&config)?;
        fs::write(&config_path, toml_str)?;

        ui::success(&format!("Config written to {}", config_path.display()));

        // Optionally backup and remove old configs
        println!();
        ui::info("Old configs preserved at:");
        if has_refs {
            println!("  {}", refs_path.display());
        }
        if has_ws {
            println!("  {}", ws_path.display());
        }
    }

    Ok(())
}

// Old refs.json format
#[derive(Debug, Deserialize)]
struct LegacyRefsConfig {
    root_directory: String,
    #[serde(default)]
    repositories: Vec<LegacyRepo>,
}

#[derive(Debug, Deserialize)]
struct LegacyRepo {
    name: String,
    url: String,
    #[serde(default)]
    default_branch: Option<String>,
}

fn migrate_refs(path: &PathBuf, config: &mut BossaConfig, ctx: &AppContext) -> Result<()> {
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    let legacy: LegacyRefsConfig =
        serde_json::from_str(&content).with_context(|| "Failed to parse refs.json")?;

    // Create collection from legacy refs
    let mut repos = Vec::new();
    for repo in legacy.repositories {
        repos.push(CollectionRepo {
            name: repo.name,
            url: repo.url,
            default_branch: repo.default_branch.unwrap_or_else(|| "main".to_string()),
            description: String::new(),
        });
    }

    let collection = Collection {
        path: legacy.root_directory,
        description: "Reference repositories for code exploration".to_string(),
        clone: crate::schema::CloneSettings {
            depth: 1,
            single_branch: true,
            options: vec![],
        },
        storage: None,
        repos,
    };

    config.collections.insert("refs".to_string(), collection);

    if !ctx.quiet {
        println!(
            "  {} Migrated {} repositories",
            "✓".green(),
            config.collections["refs"].repos.len()
        );
    }

    Ok(())
}

// Old workspaces.json format
#[derive(Debug, Deserialize)]
struct LegacyWorkspacesConfig {
    root_directory: String,
    #[serde(default)]
    repositories: Vec<LegacyWorkspace>,
}

#[derive(Debug, Deserialize)]
struct LegacyWorkspace {
    name: String,
    url: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    worktrees: Vec<LegacyWorktree>,
}

#[derive(Debug, Deserialize)]
struct LegacyWorktree {
    branch: String,
    #[serde(default)]
    path: String,
}

fn migrate_workspaces(path: &PathBuf, config: &mut BossaConfig, ctx: &AppContext) -> Result<()> {
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    let legacy: LegacyWorkspacesConfig =
        serde_json::from_str(&content).with_context(|| "Failed to parse workspaces.json")?;

    config.workspaces.root = legacy.root_directory;
    config.workspaces.structure = "category".to_string();

    for ws in &legacy.repositories {
        // Extract branch names from worktrees
        let worktree_branches: Vec<String> =
            ws.worktrees.iter().map(|wt| wt.branch.clone()).collect();

        config.workspaces.repos.push(crate::schema::WorkspaceRepo {
            name: ws.name.clone(),
            url: ws.url.clone(),
            category: ws.category.clone().unwrap_or_else(|| "default".to_string()),
            worktrees: worktree_branches,
            description: String::new(),
        });
    }

    if !ctx.quiet {
        println!(
            "  {} Migrated {} workspaces",
            "✓".green(),
            legacy.repositories.len()
        );
    }

    Ok(())
}
