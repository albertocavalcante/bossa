//! Migrate command - convert old config format to new unified format

#![allow(dead_code)]

use anyhow::Result;
use colored::Colorize;
use serde::Deserialize;
use std::path::Path;

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

    let source = bossa::FileConfigSource::default();
    let read_legacy =
        |path: &Path| bossa::ConfigDocument::read(&source, path, bossa::ConfigFormat::Json);
    let refs = match read_legacy(&bossa_refs_path)? {
        Some(document) => Some(document),
        None => read_legacy(&legacy_refs_path)?,
    };
    let workspaces = read_legacy(&ws_path)?;
    let store = config::main_store()?;
    let snapshot = store.read()?;
    let selected = snapshot.document;
    let (mut config, legacy_main) = match selected.as_ref() {
        Some(document) => config::parse_bossa_document(document)?,
        None => (BossaConfig::default(), false),
    };
    if legacy_main {
        anyhow::bail!(
            "Explicitly convert the vector-shaped main configuration before merging legacy files"
        );
    }
    config.store_revision = Some((store, snapshot.revision));
    if refs.is_none() && workspaces.is_none() {
        ui::info("No legacy configs found to migrate");
        return Ok(());
    }
    let destination = selected.as_ref().map_or_else(
        || new_dir.join("config.toml"),
        |document| document.path.clone(),
    );
    let format = selected
        .as_ref()
        .map_or(config::ConfigFormat::Toml, |document| {
            document.format.into()
        });
    ui::info(&format!("Migration destination: {}", destination.display()));

    // Migrate refs.json
    if let Some(document) = &refs {
        ui::section("Preparing refs.json migration");
        migrate_refs(document, &mut config, ctx)?;
    }

    // Migrate workspaces.json
    if let Some(document) = &workspaces {
        ui::section("Preparing workspaces.json migration");
        migrate_workspaces(document, &mut config, ctx)?;
    }
    config.validate()?;

    if dry_run {
        ui::warn("Dry run - no changes made");
        println!();
        println!("Would write to: {}", destination.display());

        // Show preview
        let toml_str = format.serialize(&config)?;
        println!();
        println!("{}", "Preview:".bold());
        for line in toml_str.lines().take(50) {
            println!("  {}", line.dimmed());
        }
        if toml_str.lines().count() > 50 {
            println!("  ... ({} more lines)", toml_str.lines().count() - 50);
        }
    } else {
        // The canonical saver propagates errors and preserves the selected format.
        let config_path = config.save()?;

        ui::success(&format!("Config written to {}", config_path.display()));

        // Optionally backup and remove old configs
        println!();
        ui::info("Old configs preserved at:");
        if let Some(document) = &refs {
            println!("  {}", document.path.display());
        }
        if let Some(document) = &workspaces {
            println!("  {}", document.path.display());
        }
    }

    Ok(())
}

// Old refs.json format
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRefsConfig {
    root_directory: String,
    #[serde(default)]
    repositories: Vec<LegacyRepo>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRepo {
    name: String,
    url: String,
    #[serde(default)]
    default_branch: Option<String>,
}

fn migrate_refs(
    document: &bossa::ConfigDocument,
    config: &mut BossaConfig,
    ctx: &AppContext,
) -> Result<()> {
    let legacy: LegacyRefsConfig = document.parse()?;

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

    if let Some(existing) = config.collections.get_mut("refs") {
        if existing.path != collection.path {
            anyhow::bail!(
                "Existing refs collection has a different path; resolve the migration conflict explicitly"
            );
        }
        for repo in collection.repos {
            if let Some(current) = existing.find_repo(&repo.name) {
                if current.url != repo.url || current.default_branch != repo.default_branch {
                    anyhow::bail!("Conflicting legacy repository: {}", repo.name);
                }
            } else {
                existing.add_repo(repo);
            }
        }
    } else {
        config.collections.insert("refs".to_string(), collection);
    }

    if !ctx.quiet {
        println!(
            "  {} Prepared {} repositories",
            "✓".green(),
            config.collections["refs"].repos.len()
        );
    }

    Ok(())
}

// Old workspaces.json format
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyWorkspacesConfig {
    root_directory: String,
    #[serde(default)]
    repositories: Vec<LegacyWorkspace>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyWorkspace {
    name: String,
    url: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    worktrees: Vec<LegacyWorktree>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyWorktree {
    branch: String,
    #[serde(default)]
    path: String,
}

fn migrate_workspaces(
    document: &bossa::ConfigDocument,
    config: &mut BossaConfig,
    ctx: &AppContext,
) -> Result<()> {
    let legacy: LegacyWorkspacesConfig = document.parse()?;
    if !config.workspaces.repos.is_empty() && config.workspaces.root != legacy.root_directory {
        anyhow::bail!("Existing workspace root conflicts with migration input");
    }
    config.workspaces.root = legacy.root_directory;
    config.workspaces.structure = "bare-worktree".to_string();
    let root = config.workspaces.expanded_root()?;
    for ws in &legacy.repositories {
        let repo = crate::schema::WorkspaceRepo {
            name: ws.name.clone(),
            url: ws.url.clone(),
            category: ws.category.clone().unwrap_or_else(|| "default".to_string()),
            worktrees: ws
                .worktrees
                .iter()
                .map(|worktree| worktree.branch.clone())
                .collect(),
            description: String::new(),
        };
        repo.validate()?;
        for worktree in &ws.worktrees {
            if !worktree.path.is_empty()
                && crate::paths::expand(&worktree.path)
                    != repo.worktree_path(&root, &worktree.branch)
            {
                anyhow::bail!(
                    "Legacy worktree path requires explicit conversion: {}",
                    worktree.path
                );
            }
        }
        if let Some(current) = config.workspaces.find_repo(&repo.name) {
            if current.url != repo.url
                || current.category != repo.category
                || current.worktrees != repo.worktrees
            {
                anyhow::bail!("Conflicting legacy workspace: {}", repo.name);
            }
        } else {
            config.workspaces.add_repo(repo);
        }
    }

    if !ctx.quiet {
        println!(
            "  {} Prepared {} workspaces",
            "✓".green(),
            legacy.repositories.len()
        );
    }

    Ok(())
}
