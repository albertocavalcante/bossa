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
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::Context;
use crate::progress;
use crate::ui;

// ============================================================================
// Command projections from the canonical schema; never deserialized as input
// ============================================================================

#[derive(Debug, Clone)]
pub struct CommandConfig {
    pub workspace_root: PathBuf,
    pub collections: Vec<Collection>,

    pub workspaces: Vec<Workspace>,

    pub storage: Vec<Storage>,
}

#[derive(Debug, Clone)]
pub struct Collection {
    pub name: String,
    pub path: String,

    pub description: Option<String>,

    pub repositories: Vec<Repository>,
    pub clone: crate::schema::CloneSettings,
}

type Repository = crate::schema::CollectionRepo;

#[derive(Debug, Clone)]
pub struct Workspace {
    pub name: String,
    pub url: String,

    pub bare_path: PathBuf,

    pub worktrees: Vec<WorktreeConfig>,
}

#[derive(Debug, Clone)]
pub struct WorktreeConfig {
    pub branch: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Storage {
    pub name: String,
    pub mount_point: PathBuf,
    pub storage_type: String,

    pub symlinks: Vec<Symlink>,
}

#[derive(Debug, Clone)]
pub struct Symlink {
    pub source: PathBuf,
    pub target: PathBuf,
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
    pub symlinks: Vec<PathBuf>,
}

#[derive(Debug, Default)]
struct ApplySummary {
    collections_checked: usize,
    workspaces_checked: usize,
    storage_checked: usize,
    repos_cloned: usize,
    repos_failed: usize,
}

impl ApplySummary {
    fn total_checked(&self) -> usize {
        self.collections_checked + self.workspaces_checked + self.storage_checked
    }

    fn has_changes(&self) -> bool {
        self.repos_cloned > 0 || self.repos_failed > 0
    }

    fn checked_line(&self) -> String {
        let mut parts = Vec::new();
        if self.collections_checked > 0 {
            parts.push(format!(
                "{} collection{}",
                self.collections_checked,
                if self.collections_checked == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        if self.workspaces_checked > 0 {
            parts.push(format!(
                "{} workspace{}",
                self.workspaces_checked,
                if self.workspaces_checked == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        if self.storage_checked > 0 {
            parts.push(format!("{} storage", self.storage_checked));
        }
        if parts.is_empty() {
            "No resources configured".to_string()
        } else {
            format!("Checked {}", parts.join(", "))
        }
    }
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
            Self::Collections => write!(f, "collections"),
            Self::Workspaces => write!(f, "workspaces"),
            Self::Storage => write!(f, "storage"),
        }
    }
}

// ============================================================================
// Config and State Loading (Stubs)
// ============================================================================

fn load_config() -> Result<CommandConfig> {
    load_command_config(false)
}

fn load_command_config(required: bool) -> Result<CommandConfig> {
    let config = crate::config::load_bossa_config(required)?;
    let working_dir = std::env::current_dir()?;
    let root = if crate::paths::environment()
        .variable("BOSSA_WORKSPACES_DIR")
        .is_some()
    {
        crate::paths::workspaces_dir()?
    } else {
        config.workspaces.expanded_root()?
    };
    let absolute = |path: PathBuf| {
        if path.is_absolute() {
            path
        } else {
            working_dir.join(path)
        }
    };
    let root = absolute(root);
    let mut collections: Vec<_> = config
        .collections
        .into_iter()
        .map(|(name, collection)| Collection {
            name,
            path: collection.path,
            description: (!collection.description.is_empty()).then_some(collection.description),
            repositories: collection.repos,
            clone: collection.clone,
        })
        .collect();
    collections.sort_by(|a, b| a.name.cmp(&b.name));
    let mut workspaces = Vec::new();
    for workspace in config.workspaces.repos {
        let bare = workspace.bare_path(&root);
        let worktrees = workspace
            .worktrees
            .iter()
            .map(|branch| WorktreeConfig {
                branch: branch.clone(),
                path: workspace.worktree_path(&root, branch),
            })
            .collect();
        workspaces.push(Workspace {
            name: workspace.name,
            url: workspace.url,
            bare_path: bare,
            worktrees,
        });
    }
    let mut storage = Vec::new();
    for (name, configured) in config.storage {
        let mount = absolute(configured.expanded_mount()?);
        let mut symlinks = Vec::new();
        for link in configured.symlinks {
            let source = if link.to.contains("{mount}") {
                let mount_text = mount
                    .to_str()
                    .context("A storage placeholder requires a UTF-8 mount path")?;
                crate::paths::expand(&link.to.replace("{mount}", mount_text))
            } else {
                crate::paths::expand(&link.to)
            };
            symlinks.push(Symlink {
                source,
                target: absolute(crate::paths::expand(&link.from)),
            });
        }
        storage.push(Storage {
            name,
            mount_point: mount,
            storage_type: match configured.storage_type {
                crate::schema::StorageType::External => "external",
                crate::schema::StorageType::Network => "network",
                crate::schema::StorageType::Internal => "internal",
            }
            .to_string(),
            symlinks,
        });
    }
    storage.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(CommandConfig {
        workspace_root: root,
        collections,
        workspaces,
        storage,
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

    let (resource_filter, name_filter) = target.map_or((None, None), parse_target);

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

    // Show hint when nothing is configured
    if resource_filter.is_none()
        && config.collections.is_empty()
        && config.workspaces.is_empty()
        && config.storage.is_empty()
    {
        ui::dim("  No resources configured");
        ui::info("  Run 'bossa add' to add collections, workspaces, or storage");
    }

    Ok(())
}

fn show_collections_status(
    config: &CommandConfig,
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
        let cloned = collection_state.map_or(0, |s| s.cloned_repos.len());
        let failed = collection_state.map_or(0, |s| s.failed_repos.len());

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
            format!("({cloned}/{total})").dimmed()
        );

        if let Some(desc) = &collection.description {
            ui::dim(&format!("    {desc}"));
        }

        ui::dim(&format!("    Path: {}", collection.path));

        if failed > 0
            && !ctx.quiet
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
    config: &CommandConfig,
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

        let bare_setup = ws_state.is_some_and(|s| s.bare_setup);
        let worktrees_count = ws_state.map_or(0, |s| s.worktrees.len());
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
            format!("({worktrees_count}/{expected_count})").dimmed()
        );

        if !ctx.quiet {
            ui::dim(&format!("    Bare: {}", if bare_setup { "✓" } else { "✗" }));
            ui::dim(&format!(
                "    Worktrees: {worktrees_count}/{expected_count}"
            ));
        }
    }

    Ok(())
}

fn show_storage_status(
    config: &CommandConfig,
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

        let mounted = stor_state.is_some_and(|s| s.mounted);
        let symlinks_count = stor_state.map_or(0, |s| s.symlinks.len());
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
            format!("({symlinks_count}/{expected_count})").dimmed()
        );

        if !ctx.quiet {
            ui::dim(&format!("    Mount: {}", stor.mount_point.display()));
            ui::dim(&format!("    Mounted: {}", if mounted { "✓" } else { "✗" }));
            ui::dim(&format!("    Symlinks: {symlinks_count}/{expected_count}"));
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

    let config = load_command_config(true)?;
    let mut state = compute_state(&config)?;
    let mut summary = ApplySummary::default();

    let (resource_filter, name_filter) = target.map_or((None, None), parse_target);

    if resource_filter.is_none() || resource_filter == Some(ResourceType::Collections) {
        for collection in config.collections.iter().filter(|collection| {
            name_filter
                .as_deref()
                .is_none_or(|name| name == collection.name)
        }) {
            collection.clone.validate()?;
        }
    }

    // Validate every selected workspace destination before any collection or
    // storage mutation, including escapes through existing directory symlinks.
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Workspaces) {
        let selected: Vec<_> = config
            .workspaces
            .iter()
            .filter(|workspace| {
                name_filter
                    .as_deref()
                    .is_none_or(|name| name == workspace.name)
            })
            .collect();
        if !selected.is_empty() {
            let root = resolve_workspace_location(&config.workspace_root)?;
            for workspace in selected {
                ensure_workspace_scope(&root, &workspace.bare_path)?;
                for worktree in &workspace.worktrees {
                    ensure_workspace_scope(&root, &worktree.path)?;
                }
            }
        }
    }

    // Apply collections
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Collections) {
        apply_collections(
            &config,
            &mut state,
            name_filter.as_deref(),
            dry_run,
            jobs,
            ctx,
            &mut summary,
        )?;
    }

    // Apply workspaces
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Workspaces) {
        apply_workspaces(
            &config,
            &mut state,
            name_filter.as_deref(),
            dry_run,
            ctx,
            &mut summary,
        )?;
    }

    // Apply storage
    if resource_filter.is_none() || resource_filter == Some(ResourceType::Storage) {
        apply_storage(
            &config,
            &mut state,
            name_filter.as_deref(),
            dry_run,
            ctx,
            &mut summary,
        )?;
    }

    if !dry_run {
        save_state(&state)?;
    }

    println!();
    if summary.total_checked() == 0 {
        ui::dim("  No resources configured");
    } else if !summary.has_changes() {
        ui::dim(&format!("  {}", summary.checked_line()));
        ui::dim("  Everything is up to date");
    }
    ui::success("Apply complete!");

    Ok(())
}

fn apply_collections(
    config: &CommandConfig,
    state: &mut BossaState,
    name_filter: Option<&str>,
    dry_run: bool,
    jobs: usize,
    ctx: &Context,
    summary: &mut ApplySummary,
) -> Result<()> {
    let collections: Vec<_> = config
        .collections
        .iter()
        .filter(|c| name_filter.is_none() || Some(c.name.as_str()) == name_filter)
        .collect();

    if collections.is_empty() {
        return Ok(());
    }

    for collection in &collections {
        apply_collection(config, state, collection, dry_run, jobs, ctx, summary)?;
    }

    summary.collections_checked += collections.len();

    Ok(())
}

fn apply_collection(
    _config: &CommandConfig,
    state: &mut BossaState,
    collection: &Collection,
    dry_run: bool,
    jobs: usize,
    ctx: &Context,
    summary: &mut ApplySummary,
) -> Result<()> {
    ui::section(&format!("Collection: {}", collection.name));

    let root = crate::paths::expand(&collection.path);

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
        .context("Failed to create clone thread pool")?;

    pool.install(|| {
        repos_to_clone.par_iter().for_each(|repo| {
            let result = clone_repo(&root, repo, &collection.clone);

            match result {
                Ok(()) => {
                    cloned.fetch_add(1, Ordering::Relaxed);
                    pb.set_message(format!("{} ✓", repo.name));
                }
                Err(e) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    push_failed_repo(&failed_repos, repo.name.clone(), e.to_string());
                    pb.set_message(format!("{} ✗", repo.name));
                }
            }

            pb.inc(1);
        });
    });

    pb.finish_and_clear();

    // Update state and summary
    let cloned_count = cloned.load(Ordering::Relaxed);
    let failed_count = failed.load(Ordering::Relaxed);
    summary.repos_cloned += cloned_count;
    summary.repos_failed += failed_count;

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

    let failed_repos_snapshot = snapshot_failed_repos(&failed_repos);

    collection_state.cloned_repos.extend(
        repos_to_clone
            .iter()
            .filter(|r| {
                !failed_repos_snapshot
                    .iter()
                    .any(|(failed_name, _)| failed_name == &r.name)
            })
            .map(|r| r.name.clone()),
    );
    collection_state.failed_repos = failed_repos_snapshot.clone();

    state.collections.retain(|s| s.name != collection.name);
    state.collections.push(collection_state);

    // Summary
    println!();
    if failed_count == 0 {
        ui::success(&format!("Cloned {cloned_count} repositories"));
    } else {
        ui::warn(&format!("Cloned {cloned_count}, {failed_count} failed"));

        if !ctx.quiet {
            for (name, error) in &failed_repos_snapshot {
                println!("  {} {} - {}", "✗".red(), name, error.dimmed());
            }
        }
    }

    Ok(())
}

fn apply_workspaces(
    config: &CommandConfig,
    state: &mut BossaState,
    name_filter: Option<&str>,
    dry_run: bool,
    ctx: &Context,
    summary: &mut ApplySummary,
) -> Result<()> {
    let workspaces: Vec<_> = config
        .workspaces
        .iter()
        .filter(|w| name_filter.is_none() || Some(w.name.as_str()) == name_filter)
        .collect();

    if workspaces.is_empty() {
        return Ok(());
    }

    summary.workspaces_checked += workspaces.len();

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
    let bare_path = &workspace.bare_path;

    let bare_exists = bare_path.exists() && bare_path.join("config").exists();

    if !bare_exists {
        ui::info("Bare repository needs setup");

        if dry_run {
            println!("  {} Clone bare repository", "→".cyan());
        } else {
            let pb = progress::spinner("Cloning bare repository...");

            fs::create_dir_all(
                bare_path
                    .parent()
                    .context("Workspace destination has no parent")?,
            )?;
            let result = std::process::Command::new("git")
                .args(["clone", "--bare", "--", &workspace.url])
                .arg(bare_path)
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
        let wt_path = &worktree.path;

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
    config: &CommandConfig,
    state: &mut BossaState,
    name_filter: Option<&str>,
    dry_run: bool,
    ctx: &Context,
    summary: &mut ApplySummary,
) -> Result<()> {
    let storage: Vec<_> = config
        .storage
        .iter()
        .filter(|s| name_filter.is_none() || Some(s.name.as_str()) == name_filter)
        .collect();

    if storage.is_empty() {
        return Ok(());
    }

    summary.storage_checked += storage.len();

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

    let mount_path = &storage.mount_point;
    let mounted = mount_path.exists();

    if !mounted {
        ui::warn("Storage not mounted");
        ui::dim(&format!("  Mount point: {}", storage.mount_point.display()));
        return Ok(());
    }

    ui::success("Storage mounted");

    // Create symlinks
    for symlink in &storage.symlinks {
        let target = &symlink.target;
        let source = &symlink.source;

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
                std::os::unix::fs::symlink(source, target)?;

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

    let (resource_filter, name_filter) = target.map_or((None, None), parse_target);

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
        let mut checked_parts = Vec::new();
        if !config.collections.is_empty() {
            checked_parts.push(format!(
                "{} collection{}",
                config.collections.len(),
                if config.collections.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        if !config.workspaces.is_empty() {
            checked_parts.push(format!(
                "{} workspace{}",
                config.workspaces.len(),
                if config.workspaces.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        if !config.storage.is_empty() {
            checked_parts.push(format!("{} storage", config.storage.len()));
        }
        if !checked_parts.is_empty() {
            ui::dim(&format!("  Checked {}", checked_parts.join(", ")));
        }
        println!();
        ui::success("No changes - current state matches desired state");
    }

    Ok(())
}

fn diff_collections(
    config: &CommandConfig,
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
        let root = crate::paths::expand(&collection.path);

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
    config: &CommandConfig,
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

        let bare_setup = ws_state.is_some_and(|s| s.bare_setup);
        let has_worktrees = ws_state.is_some_and(|s| !s.worktrees.is_empty());

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
                    ui::dim(&format!("      {} -> {}", wt.branch, wt.path.display()));
                }
            }
        }
    }

    Ok(has_changes)
}

fn diff_storage(
    config: &CommandConfig,
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

        let mounted = stor_state.is_some_and(|s| s.mounted);

        if !mounted {
            if !has_changes {
                ui::section("Storage");
                has_changes = true;
            }

            println!();
            println!("  {}", stor.name.bold());
            println!(
                "    {} Not mounted: {}",
                "!".yellow(),
                stor.mount_point.display()
            );
            continue;
        }

        // Check symlinks
        let missing_symlinks: Vec<_> = stor
            .symlinks
            .iter()
            .filter(|s| !s.target.exists())
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
                symlink.target.display(),
                symlink.source.display()
            );
        }
    }

    Ok(has_changes)
}

// ============================================================================
// Helper Functions
// ============================================================================

fn compute_state(config: &CommandConfig) -> Result<BossaState> {
    let mut state = BossaState::default();

    // Compute collections state
    for collection in &config.collections {
        let root = crate::paths::expand(&collection.path);

        let cloned_repos = get_cloned_repos(&root)?;

        state.collections.push(CollectionState {
            name: collection.name.clone(),
            cloned_repos,
            failed_repos: vec![],
        });
    }

    // Compute workspaces state
    for workspace in &config.workspaces {
        let bare_path = &workspace.bare_path;

        let bare_setup = bare_path.exists() && bare_path.join("config").exists();

        state.workspaces.push(WorkspaceState {
            name: workspace.name.clone(),
            bare_setup,
            worktrees: vec![], // TODO: detect actual worktrees
        });
    }

    // Compute storage state
    for storage in &config.storage {
        let mount_path = &storage.mount_point;
        let mounted = mount_path.exists();

        let symlinks: Vec<_> = storage
            .symlinks
            .iter()
            .filter(|s| s.target.exists())
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

        if path.is_dir()
            && path.join(".git").exists()
            && let Some(name) = entry.file_name().to_str()
        {
            repos.push(name.to_string());
        }
    }

    Ok(repos)
}

fn resolve_workspace_location(path: &Path) -> Result<PathBuf> {
    let mut existing = if path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        path.to_path_buf()
    };
    let mut suffix = Vec::new();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(_) => {
                let mut resolved = existing
                    .canonicalize()
                    .context("Could not resolve workspace location")?;
                if !suffix.is_empty() && !fs::metadata(&existing)?.is_dir() {
                    anyhow::bail!("Workspace ancestor is not a directory");
                }
                for component in suffix.into_iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(
                    existing
                        .file_name()
                        .context("Could not resolve workspace ancestor")?
                        .to_os_string(),
                );
                existing = existing
                    .parent()
                    .context("No existing workspace ancestor")?
                    .to_path_buf();
                if existing.as_os_str().is_empty() {
                    existing = PathBuf::from(".");
                }
            }
            Err(error) => return Err(error).context("Could not inspect workspace location"),
        }
    }
}

fn ensure_workspace_scope(root: &Path, destination: &Path) -> Result<()> {
    if !resolve_workspace_location(destination)?.starts_with(root) {
        anyhow::bail!(
            "Workspace destination resolves outside the configured root: {}",
            destination.display()
        );
    }
    Ok(())
}

fn clone_repo(
    root: &Path,
    repo: &Repository,
    settings: &crate::schema::CloneSettings,
) -> Result<()> {
    let runner = bossa::SystemRunner::new(crate::paths::environment().variables().clone());
    clone_repo_using(root, repo, settings, &runner)
}

fn clone_repo_using(
    root: &Path,
    repo: &Repository,
    settings: &crate::schema::CloneSettings,
    runner: &dyn bossa::ProcessRunner,
) -> Result<()> {
    settings.validate()?;
    repo.validate()?;
    let repo_path = root.join(&repo.name);
    let mut args: Vec<std::ffi::OsString> = vec!["clone".into()];
    if settings.depth > 0 {
        args.extend(["--depth".into(), settings.depth.to_string().into()]);
    }
    args.push(
        if settings.single_branch {
            "--single-branch"
        } else {
            "--no-single-branch"
        }
        .into(),
    );
    args.extend(settings.options.iter().map(Into::into));
    args.extend([
        "--branch".into(),
        repo.default_branch.clone().into(),
        "--".into(),
        repo.url.clone().into(),
        repo_path.as_os_str().to_os_string(),
    ]);
    let output = runner
        .run(
            &bossa::CommandRequest {
                program: "git".into(),
                args,
                working_dir: std::env::current_dir()?,
            },
            bossa::IoMode::Capture,
        )
        .context("Failed to execute git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    // Preserve the existing collection policy; full outcome verification follows.
    let _ = runner.run(
        &bossa::CommandRequest {
            program: "git".into(),
            args: vec![
                "-C".into(),
                repo_path.into_os_string(),
                "config".into(),
                "--local".into(),
                "core.fileMode".into(),
                "false".into(),
            ],
            working_dir: std::env::current_dir()?,
        },
        bossa::IoMode::Capture,
    );

    Ok(())
}

#[cfg(all(test, unix))]
mod config_application_tests {
    use super::*;
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::os::unix::process::ExitStatusExt;

    #[derive(Default)]
    struct RecordingRunner {
        calls: RefCell<Vec<Vec<OsString>>>,
        fail: bool,
    }

    impl bossa::ProcessRunner for RecordingRunner {
        fn run(
            &self,
            request: &bossa::CommandRequest,
            _: bossa::IoMode,
        ) -> std::io::Result<std::process::Output> {
            assert_eq!(request.program, "git");
            self.calls.borrow_mut().push(request.args.clone());
            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(if self.fail { 256 } else { 0 }),
                stdout: Vec::new(),
                stderr: b"fixture failure".to_vec(),
            })
        }
    }

    fn repository() -> Repository {
        Repository {
            name: "repo".into(),
            url: "fixture-source".into(),
            default_branch: "other".into(),
            description: String::new(),
        }
    }

    #[test]
    fn clone_honors_canonical_depth_branch_and_additional_options() {
        let root = tempfile::TempDir::new().unwrap();
        let runner = RecordingRunner::default();
        let settings = crate::schema::CloneSettings {
            depth: 0,
            single_branch: true,
            options: vec!["--no-checkout".into()],
        };
        clone_repo_using(root.path(), &repository(), &settings, &runner).unwrap();
        let calls = runner.calls.borrow();
        assert_eq!(
            calls[0],
            vec![
                OsString::from("clone"),
                "--single-branch".into(),
                "--no-checkout".into(),
                "--branch".into(),
                "other".into(),
                "--".into(),
                "fixture-source".into(),
                root.path().join("repo").into_os_string()
            ]
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn shallow_depth_is_explicit_and_conflicting_options_never_execute() {
        let root = tempfile::TempDir::new().unwrap();
        let runner = RecordingRunner::default();
        clone_repo_using(
            root.path(),
            &repository(),
            &crate::schema::CloneSettings {
                depth: 7,
                ..Default::default()
            },
            &runner,
        )
        .unwrap();
        assert_eq!(
            &runner.calls.borrow()[0][..3],
            &[OsString::from("clone"), "--depth".into(), "7".into()]
        );
        assert!(runner.calls.borrow()[0].contains(&OsString::from("--no-single-branch")));
        runner.calls.borrow_mut().clear();
        for option in [
            "--branch=ignored",
            "--dep=1",
            "--no-depth",
            "--no-branch",
            "--bran=ignored",
            "--single-br",
            "--no-single-br",
        ] {
            assert!(
                clone_repo_using(
                    root.path(),
                    &repository(),
                    &crate::schema::CloneSettings {
                        options: vec![option.into()],
                        ..Default::default()
                    },
                    &runner
                )
                .is_err()
            );
            assert!(runner.calls.borrow().is_empty());
        }
    }

    #[test]
    fn unsuccessful_clone_does_not_run_follow_up_commands() {
        let root = tempfile::TempDir::new().unwrap();
        let runner = RecordingRunner {
            fail: true,
            ..Default::default()
        };
        assert!(
            clone_repo_using(
                root.path(),
                &repository(),
                &crate::schema::CloneSettings::default(),
                &runner
            )
            .is_err()
        );
        assert_eq!(runner.calls.borrow().len(), 1);
    }
}

fn push_failed_repo(
    failed_repos: &Arc<std::sync::Mutex<Vec<(String, String)>>>,
    name: String,
    error: String,
) {
    match failed_repos.lock() {
        Ok(mut locked) => locked.push((name, error)),
        Err(poisoned) => poisoned.into_inner().push((name, error)),
    }
}

fn snapshot_failed_repos(
    failed_repos: &Arc<std::sync::Mutex<Vec<(String, String)>>>,
) -> Vec<(String, String)> {
    match failed_repos.lock() {
        Ok(locked) => locked.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{push_failed_repo, snapshot_failed_repos};
    use std::sync::{Arc, Mutex};

    #[test]
    fn push_failed_repo_handles_poisoned_mutex() {
        let failed: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let poisoned = Arc::clone(&failed);

        let _ = std::thread::spawn(move || {
            let _guard = poisoned
                .lock()
                .expect("lock should succeed before poisoning");
            panic!("intentional poison");
        })
        .join();

        push_failed_repo(&failed, "repo-a".to_string(), "network".to_string());
        let snapshot = snapshot_failed_repos(&failed);

        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, "repo-a");
    }

    #[test]
    fn snapshot_failed_repos_recovers_poisoned_mutex() {
        let failed: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let poisoned = Arc::clone(&failed);

        let _ = std::thread::spawn(move || {
            let mut guard = poisoned
                .lock()
                .expect("lock should succeed before poisoning");
            guard.push(("repo-b".to_string(), "timeout".to_string()));
            panic!("intentional poison");
        })
        .join();

        let snapshot = snapshot_failed_repos(&failed);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, "repo-b");
    }
}
