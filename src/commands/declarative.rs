//! Declarative commands for bossa CLI redesign
//!
//! This module implements the core declarative commands:
//! - `status` - Show current state vs desired state
//! - `apply` - Make current state match desired state
//! - `diff` - Preview what apply would change

#![allow(dead_code)]

use anyhow::{Context as AnyhowContext, Result};
use colored::Colorize;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::Context;
use crate::progress;
use crate::ui;

// ============================================================================
// Temporary stub types (will be replaced by schema.rs and state.rs)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BossaConfig {
    #[serde(default)]
    pub collections: Vec<Collection>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub storage: Vec<Storage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repositories: Vec<Repository>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub name: String,
    pub url: String,
    #[serde(default = "default_branch")]
    pub default_branch: String,
}

fn default_branch() -> String {
    "main".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub bare_dir: Option<String>,
    #[serde(default)]
    pub worktrees: Vec<WorktreeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeConfig {
    pub branch: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Storage {
    pub name: String,
    pub mount_point: String,
    pub storage_type: String,
    #[serde(default)]
    pub symlinks: Vec<Symlink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symlink {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Default)]
pub struct BossaState {
    pub collections: Vec<CollectionState>,
    pub workspaces: Vec<WorkspaceState>,
    pub storage: Vec<StorageState>,
}

#[derive(Debug, Clone)]
pub struct CollectionState {
    pub name: String,
    pub cloned_repos: Vec<String>,
    pub failed_repos: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceState {
    pub name: String,
    pub bare_setup: bool,
    pub worktrees: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StorageState {
    pub name: String,
    pub mounted: bool,
    pub symlinks: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    Collections,
    Workspaces,
    Storage,
}

impl std::fmt::Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceType::Collections => write!(f, "collections"),
            ResourceType::Workspaces => write!(f, "workspaces"),
            ResourceType::Storage => write!(f, "storage"),
        }
    }
}

// ============================================================================
// Config and State Loading (Stubs)
// ============================================================================

fn load_config() -> Result<BossaConfig> {
    // Stub: Will load from ~/.config/bossa/config.{toml,json}
    // This is a temporary implementation that returns empty config
    // until schema.rs is implemented with proper BossaConfig serialization

    let config_dir = crate::config::config_dir()?;

    // Try to load the new unified config format
    if let Ok((config, _)) = crate::config::load_config::<BossaConfig>(&config_dir, "config") {
        return Ok(config);
    }

    // Fallback: Try legacy configs from workspace-setup directory
    let _legacy_dir = crate::config::legacy_config_dir().ok();

    let collections = vec![];
    let workspaces = vec![];

    // Try loading legacy refs.json/toml if it exists
    // Note: These types need to be defined in schema.rs or we load as serde_json::Value
    // For now, just return empty to avoid compilation errors

    Ok(BossaConfig {
        collections,
        workspaces,
        storage: vec![],
    })
}

fn load_state() -> Result<BossaState> {
    // Stub: Will load from ~/.local/state/bossa/state.json
    // For now, return empty state and compute on the fly
    Ok(BossaState::default())
}

fn save_state(_state: &BossaState) -> Result<()> {
    // Stub: Will save to ~/.local/state/bossa/state.json
    Ok(())
}

// ============================================================================
// Target Parsing
// ============================================================================

fn parse_target(target: &str) -> (Option<ResourceType>, Option<String>) {
    let parts: Vec<&str> = target.split('.').collect();

    match parts.len() {
        1 => {
            // Just resource type: "collections", "workspaces", "storage"
            match parts[0] {
                "collections" => (Some(ResourceType::Collections), None),
                "workspaces" => (Some(ResourceType::Workspaces), None),
                "storage" => (Some(ResourceType::Storage), None),
                _ => (None, Some(target.to_string())),
            }
        }
        2 => {
            // Resource type + name: "collections.refs", "storage.t9"
            let resource_type = match parts[0] {
                "collections" => Some(ResourceType::Collections),
                "workspaces" => Some(ResourceType::Workspaces),
                "storage" => Some(ResourceType::Storage),
                _ => None,
            };
            (resource_type, Some(parts[1].to_string()))
        }
        _ => (None, Some(target.to_string())),
    }
}

// ============================================================================
// Status Command
// ============================================================================

pub fn status(ctx: &Context, target: Option<&str>) -> Result<()> {
    ui::header("Bossa Status");

    let config = load_config()?;
    let state = compute_state(&config)?;

    let (resource_filter, name_filter) = target.map(parse_target).unwrap_or((None, None));

    // Show collections
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Collections) {
        show_collections_status(&config, &state, name_filter.as_deref(), ctx)?;
    }

    // Show workspaces
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Workspaces) {
        show_workspaces_status(&config, &state, name_filter.as_deref(), ctx)?;
    }

    // Show storage
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Storage) {
        show_storage_status(&config, &state, name_filter.as_deref(), ctx)?;
    }

    Ok(())
}

fn show_collections_status(
    config: &BossaConfig,
    state: &BossaState,
    name_filter: Option<&str>,
    ctx: &Context,
) -> Result<()> {
    let collections: Vec<_> = config
        .collections
        .iter()
        .filter(|c| name_filter.is_none() || Some(c.name.as_str()) == name_filter)
        .collect();

    if collections.is_empty() {
        return Ok(());
    }

    ui::section("Collections");

    for collection in collections {
        let collection_state = state.collections.iter().find(|s| s.name == collection.name);

        let total = collection.repositories.len();
        let cloned = collection_state.map(|s| s.cloned_repos.len()).unwrap_or(0);
        let failed = collection_state.map(|s| s.failed_repos.len()).unwrap_or(0);

        let status_icon = if cloned == total && failed == 0 {
            "✓".green()
        } else if cloned == 0 {
            "✗".red()
        } else {
            "⚠".yellow()
        };

        println!(
            "  {} {} {}",
            status_icon,
            collection.name.bold(),
            format!("({}/{})", cloned, total).dimmed()
        );

        if let Some(desc) = &collection.description {
            ui::dim(&format!("    {}", desc));
        }

        ui::dim(&format!("    Path: {}", collection.path));

        if failed > 0 && !ctx.quiet
            && let Some(state) = collection_state
        {
            for (name, error) in &state.failed_repos {
                println!("      {} {} - {}", "✗".red(), name, error.dimmed());
            }
        }
    }

    Ok(())
}

fn show_workspaces_status(
    config: &BossaConfig,
    state: &BossaState,
    name_filter: Option<&str>,
    ctx: &Context,
) -> Result<()> {
    let workspaces: Vec<_> = config
        .workspaces
        .iter()
        .filter(|w| name_filter.is_none() || Some(w.name.as_str()) == name_filter)
        .collect();

    if workspaces.is_empty() {
        return Ok(());
    }

    ui::section("Workspaces");

    for workspace in workspaces {
        let ws_state = state.workspaces.iter().find(|s| s.name == workspace.name);

        let bare_setup = ws_state.map(|s| s.bare_setup).unwrap_or(false);
        let worktrees_count = ws_state.map(|s| s.worktrees.len()).unwrap_or(0);
        let expected_count = workspace.worktrees.len();

        let status_icon = if bare_setup && worktrees_count == expected_count {
            "✓".green()
        } else if !bare_setup {
            "✗".red()
        } else {
            "⚠".yellow()
        };

        println!(
            "  {} {} {}",
            status_icon,
            workspace.name.bold(),
            format!("({}/{})", worktrees_count, expected_count).dimmed()
        );

        if !ctx.quiet {
            ui::dim(&format!("    Bare: {}", if bare_setup { "✓" } else { "✗" }));
            ui::dim(&format!(
                "    Worktrees: {}/{}",
                worktrees_count, expected_count
            ));
        }
    }

    Ok(())
}

fn show_storage_status(
    config: &BossaConfig,
    state: &BossaState,
    name_filter: Option<&str>,
    ctx: &Context,
) -> Result<()> {
    let storage: Vec<_> = config
        .storage
        .iter()
        .filter(|s| name_filter.is_none() || Some(s.name.as_str()) == name_filter)
        .collect();

    if storage.is_empty() {
        return Ok(());
    }

    ui::section("Storage");

    for stor in storage {
        let stor_state = state.storage.iter().find(|s| s.name == stor.name);

        let mounted = stor_state.map(|s| s.mounted).unwrap_or(false);
        let symlinks_count = stor_state.map(|s| s.symlinks.len()).unwrap_or(0);
        let expected_count = stor.symlinks.len();

        let status_icon = if mounted && symlinks_count == expected_count {
            "✓".green()
        } else if !mounted {
            "✗".red()
        } else {
            "⚠".yellow()
        };

        println!(
            "  {} {} {}",
            status_icon,
            stor.name.bold(),
            format!("({}/{})", symlinks_count, expected_count).dimmed()
        );

        if !ctx.quiet {
            ui::dim(&format!("    Mount: {}", stor.mount_point));
            ui::dim(&format!("    Mounted: {}", if mounted { "✓" } else { "✗" }));
            ui::dim(&format!(
                "    Symlinks: {}/{}",
                symlinks_count, expected_count
            ));
        }
    }

    Ok(())
}

// ============================================================================
// Apply Command
// ============================================================================

pub fn apply(ctx: &Context, target: Option<&str>, dry_run: bool, jobs: usize) -> Result<()> {
    ui::header("Applying Configuration");

    if dry_run {
        ui::warn("Dry run - no changes will be made");
        println!();
    }

    let config = load_config()?;
    let mut state = compute_state(&config)?;

    let (resource_filter, name_filter) = target.map(parse_target).unwrap_or((None, None));

    // Apply collections
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Collections) {
        apply_collections(
            &config,
            &mut state,
            name_filter.as_deref(),
            dry_run,
            jobs,
            ctx,
        )?;
    }

    // Apply workspaces
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Workspaces) {
        apply_workspaces(&config, &mut state, name_filter.as_deref(), dry_run, ctx)?;
    }

    // Apply storage
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Storage) {
        apply_storage(&config, &mut state, name_filter.as_deref(), dry_run, ctx)?;
    }

    if !dry_run {
        save_state(&state)?;
    }

    println!();
    ui::success("Apply complete!");

    Ok(())
}

fn apply_collections(
    config: &BossaConfig,
    state: &mut BossaState,
    name_filter: Option<&str>,
    dry_run: bool,
    jobs: usize,
    ctx: &Context,
) -> Result<()> {
    let collections: Vec<_> = config
        .collections
        .iter()
        .filter(|c| name_filter.is_none() || Some(c.name.as_str()) == name_filter)
        .collect();

    if collections.is_empty() {
        return Ok(());
    }

    for collection in collections {
        apply_collection(config, state, collection, dry_run, jobs, ctx)?;
    }

    Ok(())
}

fn apply_collection(
    _config: &BossaConfig,
    state: &mut BossaState,
    collection: &Collection,
    dry_run: bool,
    jobs: usize,
    ctx: &Context,
) -> Result<()> {
    ui::section(&format!("Collection: {}", collection.name));

    let root = shellexpand::tilde(&collection.path);
    let root = PathBuf::from(root.as_ref());

    // Find repos to clone
    let cloned_repos = get_cloned_repos(&root)?;
    let repos_to_clone: Vec<_> = collection
        .repositories
        .iter()
        .filter(|r| !cloned_repos.contains(&r.name))
        .collect();

    if repos_to_clone.is_empty() {
        ui::success("All repositories already cloned");
        return Ok(());
    }

    ui::kv("To clone", &repos_to_clone.len().to_string());

    if dry_run {
        for repo in &repos_to_clone {
            println!("  {} {}", "→".cyan(), repo.name);
        }
        return Ok(());
    }

    // Ensure root exists
    fs::create_dir_all(&root)?;

    // Clone in parallel
    let pb = progress::clone_bar(repos_to_clone.len() as u64, "Cloning");
    let cloned = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let failed_repos: Arc<std::sync::Mutex<Vec<(String, String)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .unwrap();

    pool.install(|| {
        repos_to_clone.par_iter().for_each(|repo| {
            let result = clone_repo(&root, repo);

            match result {
                Ok(()) => {
                    cloned.fetch_add(1, Ordering::Relaxed);
                    pb.set_message(format!("{} ✓", repo.name));
                }
                Err(e) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    failed_repos
                        .lock()
                        .unwrap()
                        .push((repo.name.clone(), e.to_string()));
                    pb.set_message(format!("{} ✗", repo.name));
                }
            }

            pb.inc(1);
        });
    });

    pb.finish_and_clear();

    // Update state
    let cloned_count = cloned.load(Ordering::Relaxed);
    let failed_count = failed.load(Ordering::Relaxed);

    let mut collection_state = state
        .collections
        .iter()
        .find(|s| s.name == collection.name)
        .cloned()
        .unwrap_or_else(|| CollectionState {
            name: collection.name.clone(),
            cloned_repos: vec![],
            failed_repos: vec![],
        });

    collection_state.cloned_repos.extend(
        repos_to_clone
            .iter()
            .filter(|r| {
                !failed_repos
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(name, _)| name == &r.name)
            })
            .map(|r| r.name.clone()),
    );
    collection_state.failed_repos = failed_repos.lock().unwrap().clone();

    state.collections.retain(|s| s.name != collection.name);
    state.collections.push(collection_state);

    // Summary
    println!();
    if failed_count == 0 {
        ui::success(&format!("Cloned {} repositories", cloned_count));
    } else {
        ui::warn(&format!("Cloned {}, {} failed", cloned_count, failed_count));

        if !ctx.quiet {
            for (name, error) in failed_repos.lock().unwrap().iter() {
                println!("  {} {} - {}", "✗".red(), name, error.dimmed());
            }
        }
    }

    Ok(())
}

fn apply_workspaces(
    config: &BossaConfig,
    state: &mut BossaState,
    name_filter: Option<&str>,
    dry_run: bool,
    ctx: &Context,
) -> Result<()> {
    let workspaces: Vec<_> = config
        .workspaces
        .iter()
        .filter(|w| name_filter.is_none() || Some(w.name.as_str()) == name_filter)
        .collect();

    if workspaces.is_empty() {
        return Ok(());
    }

    for workspace in workspaces {
        apply_workspace(state, workspace, dry_run, ctx)?;
    }

    Ok(())
}

fn apply_workspace(
    state: &mut BossaState,
    workspace: &Workspace,
    dry_run: bool,
    ctx: &Context,
) -> Result<()> {
    ui::section(&format!("Workspace: {}", workspace.name));

    // Check if bare repo exists
    let ws_dir = crate::config::workspaces_dir()?;
    let bare_dir = workspace.bare_dir.as_deref().unwrap_or(&workspace.name);
    let bare_path = ws_dir.join(bare_dir);

    let bare_exists = bare_path.exists() && bare_path.join("config").exists();

    if !bare_exists {
        ui::info("Bare repository needs setup");

        if dry_run {
            println!("  {} Clone bare repository", "→".cyan());
        } else {
            let pb = progress::spinner("Cloning bare repository...");

            fs::create_dir_all(&ws_dir)?;

            let result = std::process::Command::new("git")
                .args([
                    "clone",
                    "--bare",
                    &workspace.url,
                    bare_path.to_str().unwrap(),
                ])
                .output()?;

            if result.status.success() {
                progress::finish_success(&pb, "Bare repository cloned");
            } else {
                let stderr = String::from_utf8_lossy(&result.stderr);
                progress::finish_error(&pb, &format!("Failed: {}", stderr.trim()));

                // Update state with failure
                let ws_state = WorkspaceState {
                    name: workspace.name.clone(),
                    bare_setup: false,
                    worktrees: vec![],
                };
                state.workspaces.retain(|s| s.name != workspace.name);
                state.workspaces.push(ws_state);

                return Ok(());
            }
        }
    } else {
        ui::success("Bare repository already setup");
    }

    // Setup worktrees
    for worktree in &workspace.worktrees {
        let wt_path = shellexpand::tilde(&worktree.path);
        let wt_path = PathBuf::from(wt_path.as_ref());

        if !wt_path.exists() {
            ui::info(&format!("Creating worktree: {}", worktree.branch));

            if dry_run {
                println!(
                    "  {} git worktree add {} {}",
                    "→".cyan(),
                    wt_path.display(),
                    worktree.branch
                );
            } else if !ctx.quiet {
                ui::dim(&format!("  {}", wt_path.display()));
            }
        }
    }

    // Update state
    let ws_state = WorkspaceState {
        name: workspace.name.clone(),
        bare_setup: !dry_run, // Mark as setup if we cloned it
        worktrees: workspace
            .worktrees
            .iter()
            .map(|wt| wt.branch.clone())
            .collect(),
    };
    state.workspaces.retain(|s| s.name != workspace.name);
    state.workspaces.push(ws_state);

    Ok(())
}

fn apply_storage(
    config: &BossaConfig,
    state: &mut BossaState,
    name_filter: Option<&str>,
    dry_run: bool,
    ctx: &Context,
) -> Result<()> {
    let storage: Vec<_> = config
        .storage
        .iter()
        .filter(|s| name_filter.is_none() || Some(s.name.as_str()) == name_filter)
        .collect();

    if storage.is_empty() {
        return Ok(());
    }

    for stor in storage {
        apply_storage_item(state, stor, dry_run, ctx)?;
    }

    Ok(())
}

fn apply_storage_item(
    state: &mut BossaState,
    storage: &Storage,
    dry_run: bool,
    ctx: &Context,
) -> Result<()> {
    ui::section(&format!("Storage: {}", storage.name));

    let mount_path = PathBuf::from(&storage.mount_point);
    let mounted = mount_path.exists();

    if !mounted {
        ui::warn("Storage not mounted");
        ui::dim(&format!("  Mount point: {}", storage.mount_point));
        return Ok(());
    }

    ui::success("Storage mounted");

    // Create symlinks
    for symlink in &storage.symlinks {
        let target = shellexpand::tilde(&symlink.target);
        let target = PathBuf::from(target.as_ref());

        let source = shellexpand::tilde(&symlink.source);
        let source = PathBuf::from(source.as_ref());

        if target.exists() {
            if !ctx.quiet {
                ui::dim(&format!("  ✓ {} -> {}", target.display(), source.display()));
            }
        } else {
            ui::info(&format!("Creating symlink: {}", target.display()));

            if dry_run {
                println!(
                    "  {} ln -s {} {}",
                    "→".cyan(),
                    source.display(),
                    target.display()
                );
            } else {
                // Ensure parent directory exists
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }

                #[cfg(unix)]
                std::os::unix::fs::symlink(&source, &target)?;

                #[cfg(not(unix))]
                ui::warn("Symlink creation not supported on this platform");
            }
        }
    }

    // Update state
    let stor_state = StorageState {
        name: storage.name.clone(),
        mounted,
        symlinks: storage.symlinks.iter().map(|s| s.target.clone()).collect(),
    };
    state.storage.retain(|s| s.name != storage.name);
    state.storage.push(stor_state);

    Ok(())
}

// ============================================================================
// Diff Command
// ============================================================================

pub fn diff(ctx: &Context, target: Option<&str>) -> Result<()> {
    ui::header("Configuration Diff");

    let config = load_config()?;
    let state = compute_state(&config)?;

    let (resource_filter, name_filter) = target.map(parse_target).unwrap_or((None, None));

    let mut has_changes = false;

    // Diff collections
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Collections) {
        has_changes |= diff_collections(&config, &state, name_filter.as_deref(), ctx)?;
    }

    // Diff workspaces
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Workspaces) {
        has_changes |= diff_workspaces(&config, &state, name_filter.as_deref(), ctx)?;
    }

    // Diff storage
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Storage) {
        has_changes |= diff_storage(&config, &state, name_filter.as_deref(), ctx)?;
    }

    if !has_changes {
        println!();
        ui::success("No changes - current state matches desired state");
    }

    Ok(())
}

fn diff_collections(
    config: &BossaConfig,
    _state: &BossaState,
    name_filter: Option<&str>,
    ctx: &Context,
) -> Result<bool> {
    let collections: Vec<_> = config
        .collections
        .iter()
        .filter(|c| name_filter.is_none() || Some(c.name.as_str()) == name_filter)
        .collect();

    if collections.is_empty() {
        return Ok(false);
    }

    let mut has_changes = false;

    for collection in collections {
        let root = shellexpand::tilde(&collection.path);
        let root = PathBuf::from(root.as_ref());

        let cloned_repos = get_cloned_repos(&root)?;
        let configured_repos: HashSet<_> = collection
            .repositories
            .iter()
            .map(|r| r.name.as_str())
            .collect();

        // Repos in config but not cloned
        let missing: Vec<_> = collection
            .repositories
            .iter()
            .filter(|r| !cloned_repos.contains(&r.name))
            .collect();

        // Repos cloned but not in config (drift)
        let extra: Vec<_> = cloned_repos
            .iter()
            .filter(|name| !configured_repos.contains(name.as_str()))
            .collect();

        if missing.is_empty() && extra.is_empty() {
            continue;
        }

        if !has_changes {
            ui::section("Collections");
            has_changes = true;
        }

        println!();
        println!("  {}", collection.name.bold());

        if !missing.is_empty() {
            println!("    {} Missing (will be cloned):", "+".green());
            for repo in missing {
                println!("      {} {}", "+".green(), repo.name);
                if !ctx.quiet {
                    ui::dim(&format!("        {}", repo.url));
                }
            }
        }

        if !extra.is_empty() {
            println!("    {} Drift (cloned but not in config):", "!".yellow());
            for name in extra {
                println!("      {} {}", "!".yellow(), name);
            }
        }
    }

    Ok(has_changes)
}

fn diff_workspaces(
    config: &BossaConfig,
    state: &BossaState,
    name_filter: Option<&str>,
    ctx: &Context,
) -> Result<bool> {
    let workspaces: Vec<_> = config
        .workspaces
        .iter()
        .filter(|w| name_filter.is_none() || Some(w.name.as_str()) == name_filter)
        .collect();

    if workspaces.is_empty() {
        return Ok(false);
    }

    let mut has_changes = false;

    for workspace in workspaces {
        let ws_state = state.workspaces.iter().find(|s| s.name == workspace.name);

        let bare_setup = ws_state.map(|s| s.bare_setup).unwrap_or(false);
        let has_worktrees = ws_state.map(|s| !s.worktrees.is_empty()).unwrap_or(false);

        if bare_setup && has_worktrees {
            continue;
        }

        if !has_changes {
            ui::section("Workspaces");
            has_changes = true;
        }

        println!();
        println!("  {}", workspace.name.bold());

        if !bare_setup {
            println!("    {} Bare repository not setup", "+".green());
            if !ctx.quiet {
                ui::dim(&format!("      {}", workspace.url));
            }
        }

        if !has_worktrees {
            println!(
                "    {} Worktrees not created: {}",
                "+".green(),
                workspace.worktrees.len()
            );
            if !ctx.quiet {
                for wt in &workspace.worktrees {
                    ui::dim(&format!("      {} -> {}", wt.branch, wt.path));
                }
            }
        }
    }

    Ok(has_changes)
}

fn diff_storage(
    config: &BossaConfig,
    state: &BossaState,
    name_filter: Option<&str>,
    _ctx: &Context,
) -> Result<bool> {
    let storage: Vec<_> = config
        .storage
        .iter()
        .filter(|s| name_filter.is_none() || Some(s.name.as_str()) == name_filter)
        .collect();

    if storage.is_empty() {
        return Ok(false);
    }

    let mut has_changes = false;

    for stor in storage {
        let stor_state = state.storage.iter().find(|s| s.name == stor.name);

        let mounted = stor_state.map(|s| s.mounted).unwrap_or(false);

        if !mounted {
            if !has_changes {
                ui::section("Storage");
                has_changes = true;
            }

            println!();
            println!("  {}", stor.name.bold());
            println!("    {} Not mounted: {}", "!".yellow(), stor.mount_point);
            continue;
        }

        // Check symlinks
        let missing_symlinks: Vec<_> = stor
            .symlinks
            .iter()
            .filter(|s| {
                let target = shellexpand::tilde(&s.target);
                !PathBuf::from(target.as_ref()).exists()
            })
            .collect();

        if missing_symlinks.is_empty() {
            continue;
        }

        if !has_changes {
            ui::section("Storage");
            has_changes = true;
        }

        println!();
        println!("  {}", stor.name.bold());
        println!("    {} Missing symlinks:", "+".green());
        for symlink in missing_symlinks {
            println!(
                "      {} {} -> {}",
                "+".green(),
                symlink.target,
                symlink.source
            );
        }
    }

    Ok(has_changes)
}

// ============================================================================
// Helper Functions
// ============================================================================

fn compute_state(config: &BossaConfig) -> Result<BossaState> {
    let mut state = BossaState::default();

    // Compute collections state
    for collection in &config.collections {
        let root = shellexpand::tilde(&collection.path);
        let root = PathBuf::from(root.as_ref());

        let cloned_repos = get_cloned_repos(&root)?;

        state.collections.push(CollectionState {
            name: collection.name.clone(),
            cloned_repos,
            failed_repos: vec![],
        });
    }

    // Compute workspaces state
    for workspace in &config.workspaces {
        let ws_dir = crate::config::workspaces_dir()?;
        let bare_dir = workspace.bare_dir.as_deref().unwrap_or(&workspace.name);
        let bare_path = ws_dir.join(bare_dir);

        let bare_setup = bare_path.exists() && bare_path.join("config").exists();

        state.workspaces.push(WorkspaceState {
            name: workspace.name.clone(),
            bare_setup,
            worktrees: vec![], // TODO: detect actual worktrees
        });
    }

    // Compute storage state
    for storage in &config.storage {
        let mount_path = PathBuf::from(&storage.mount_point);
        let mounted = mount_path.exists();

        let symlinks: Vec<_> = storage
            .symlinks
            .iter()
            .filter(|s| {
                let target = shellexpand::tilde(&s.target);
                PathBuf::from(target.as_ref()).exists()
            })
            .map(|s| s.target.clone())
            .collect();

        state.storage.push(StorageState {
            name: storage.name.clone(),
            mounted,
            symlinks,
        });
    }

    Ok(state)
}

fn get_cloned_repos(root: &Path) -> Result<Vec<String>> {
    if !root.exists() {
        return Ok(vec![]);
    }

    let entries = fs::read_dir(root)?;
    let mut repos = Vec::new();

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() && path.join(".git").exists()
            && let Some(name) = entry.file_name().to_str()
        {
            repos.push(name.to_string());
        }
    }

    Ok(repos)
}

fn clone_repo(root: &Path, repo: &Repository) -> Result<()> {
    let repo_path = root.join(&repo.name);

    let output = std::process::Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            &repo.url,
            repo_path.to_str().unwrap(),
        ])
        .output()
        .context("Failed to execute git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    // Configure for exFAT (T9 drive)
    let _ = std::process::Command::new("git")
        .args([
            "-C",
            repo_path.to_str().unwrap(),
            "config",
            "--local",
            "core.fileMode",
            "false",
        ])
        .output();

    Ok(())
}
