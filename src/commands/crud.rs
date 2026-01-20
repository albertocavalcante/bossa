use anyhow::{Context as AnyhowContext, Result, bail};
use colored::Colorize;

use crate::Context;
use crate::cli::ResourceType;
use crate::config;
use crate::schema::{BossaConfig, Collection, CollectionRepo, Storage, StorageType, WorkspaceRepo};
use crate::ui;

// ============================================================================
// Add Commands
// ============================================================================

/// Add a collection to the config
pub fn add_collection(
    _ctx: &Context,
    name: &str,
    path: &str,
    description: Option<&str>,
) -> Result<()> {
    ui::header(&format!("Adding Collection: {}", name));

    // Load or create config
    let mut config = BossaConfig::load()?;

    // Check if already exists
    if config.collections.contains_key(name) {
        ui::warn(&format!("Collection '{}' already exists in config", name));
        return Ok(());
    }

    ui::kv("Name", name);
    ui::kv("Path", path);
    if let Some(desc) = description {
        ui::kv("Description", desc);
    }

    // Add collection
    config.collections.insert(
        name.to_string(),
        Collection {
            path: path.to_string(),
            description: description.unwrap_or("").to_string(),
            clone: Default::default(),
            storage: None,
            repos: Vec::new(),
        },
    );

    // Save config
    config.save()?;
    ui::success(&format!("Added collection '{}'", name));
    ui::dim("Run 'bossa apply' to create the directory and clone repos");

    Ok(())
}

/// Add a repository to a collection
pub fn add_repo(_ctx: &Context, collection: &str, url: &str, name: Option<&str>) -> Result<()> {
    // Auto-detect name from URL if not provided
    let repo_name = name
        .map(|s| s.to_string())
        .or_else(|| config::repo_name_from_url(url))
        .context("Could not determine repo name from URL. Use --name to specify.")?;

    ui::header(&format!("Adding Repo: {}", repo_name));
    ui::kv("Collection", collection);
    ui::kv("URL", url);
    ui::kv("Name", &repo_name);

    // Auto-detect default branch
    ui::info("Detecting default branch...");
    let default_branch = config::detect_default_branch(url);
    ui::kv("Default branch", &default_branch);

    // Load config
    let mut config = BossaConfig::load()?;

    // Get or create collection
    let coll = config
        .collections
        .get_mut(collection)
        .context(format!("Collection '{}' not found", collection))?;

    // Check if already exists
    if coll.find_repo(&repo_name).is_some() {
        ui::warn(&format!(
            "Repo '{}' already exists in collection '{}'",
            repo_name, collection
        ));
        return Ok(());
    }

    // Add repo
    coll.add_repo(CollectionRepo {
        name: repo_name.clone(),
        url: url.to_string(),
        default_branch,
        description: String::new(),
    });

    // Save config
    config.save()?;
    ui::success(&format!(
        "Added repo '{}' to collection '{}'",
        repo_name, collection
    ));
    ui::dim("Run 'bossa apply' to clone the repository");

    Ok(())
}

/// Add a workspace
pub fn add_workspace(
    _ctx: &Context,
    url: &str,
    name: Option<&str>,
    category: Option<&str>,
) -> Result<()> {
    // Auto-detect name from URL if not provided
    let workspace_name = name
        .map(|s| s.to_string())
        .or_else(|| config::repo_name_from_url(url))
        .context("Could not determine workspace name from URL. Use --name to specify.")?;

    ui::header(&format!("Adding Workspace: {}", workspace_name));
    ui::kv("URL", url);
    ui::kv("Name", &workspace_name);
    let category_str = category.unwrap_or("default");
    ui::kv("Category", category_str);

    // Load config
    let mut config = BossaConfig::load()?;

    // Check if already exists
    if config.workspaces.find_repo(&workspace_name).is_some() {
        ui::warn(&format!(
            "Workspace '{}' already exists in config",
            workspace_name
        ));
        return Ok(());
    }

    // Add workspace
    config.workspaces.add_repo(WorkspaceRepo {
        name: workspace_name.clone(),
        url: url.to_string(),
        category: category_str.to_string(),
        worktrees: Vec::new(),
        description: String::new(),
    });

    // Save config
    config.save()?;
    ui::success(&format!("Added workspace '{}'", workspace_name));
    ui::dim("Run 'bossa apply' to initialize the workspace");

    Ok(())
}

/// Add storage
pub fn add_storage(
    _ctx: &Context,
    name: &str,
    mount: &str,
    storage_type: Option<&str>,
) -> Result<()> {
    ui::header(&format!("Adding Storage: {}", name));
    ui::kv("Name", name);
    ui::kv("Mount point", mount);

    let st = match storage_type {
        Some("external") | None => StorageType::External,
        Some("network") => StorageType::Network,
        Some("internal") => StorageType::Internal,
        Some(t) => bail!(
            "Unknown storage type: {}. Valid types: external, network, internal",
            t
        ),
    };

    ui::kv("Type", &format!("{:?}", st).to_lowercase());

    // Load config
    let mut config = BossaConfig::load()?;

    // Check if already exists
    if config.storage.contains_key(name) {
        ui::warn(&format!("Storage '{}' already exists in config", name));
        return Ok(());
    }

    // Add storage
    config.storage.insert(
        name.to_string(),
        Storage {
            mount: mount.to_string(),
            storage_type: st,
            symlinks: Vec::new(),
            description: String::new(),
        },
    );

    // Save config
    config.save()?;
    ui::success(&format!("Added storage '{}'", name));
    ui::dim("Run 'bossa apply' to set up symlinks and mounts");

    Ok(())
}

// ============================================================================
// Remove Commands
// ============================================================================

/// Remove a collection from config
pub fn rm_collection(_ctx: &Context, name: &str) -> Result<()> {
    ui::header(&format!("Removing Collection: {}", name));

    // Load config
    let mut config = BossaConfig::load()?;

    // Remove collection
    if config.collections.remove(name).is_none() {
        ui::warn(&format!("Collection '{}' not found in config", name));
        return Ok(());
    }

    // Save config
    config.save()?;
    ui::success(&format!("Removed collection '{}'", name));
    ui::dim("Note: This only modifies config - directories and repos are NOT deleted");
    ui::dim("You must manually delete the directory if needed");

    Ok(())
}

/// Remove a repository from a collection
pub fn rm_repo(_ctx: &Context, collection: &str, name: &str) -> Result<()> {
    ui::header(&format!("Removing Repo: {}", name));
    ui::kv("Collection", collection);
    ui::kv("Repo", name);

    // Load config
    let mut config = BossaConfig::load()?;

    // Get collection
    let coll = config
        .collections
        .get_mut(collection)
        .context(format!("Collection '{}' not found", collection))?;

    // Remove repo
    if !coll.remove_repo(name) {
        ui::warn(&format!(
            "Repo '{}' not found in collection '{}'",
            name, collection
        ));
        return Ok(());
    }

    // Save config
    config.save()?;
    ui::success(&format!(
        "Removed repo '{}' from collection '{}'",
        name, collection
    ));
    ui::dim("Note: This only modifies config - the local clone was NOT deleted");
    ui::dim("You must manually delete the directory if needed");

    Ok(())
}

/// Remove a workspace from config
pub fn rm_workspace(_ctx: &Context, name: &str) -> Result<()> {
    ui::header(&format!("Removing Workspace: {}", name));

    // Load config
    let mut config = BossaConfig::load()?;

    // Remove workspace
    if !config.workspaces.remove_repo(name) {
        ui::warn(&format!("Workspace '{}' not found in config", name));
        return Ok(());
    }

    // Save config
    config.save()?;
    ui::success(&format!("Removed workspace '{}'", name));
    ui::dim("Note: This only modifies config - worktrees and bare repo are NOT deleted");
    ui::dim("You must manually delete the directories if needed");

    Ok(())
}

/// Remove storage from config
pub fn rm_storage(_ctx: &Context, name: &str) -> Result<()> {
    ui::header(&format!("Removing Storage: {}", name));

    // Load config
    let mut config = BossaConfig::load()?;

    // Remove storage
    if config.storage.remove(name).is_none() {
        ui::warn(&format!("Storage '{}' not found in config", name));
        return Ok(());
    }

    // Save config
    config.save()?;
    ui::success(&format!("Removed storage '{}'", name));
    ui::dim("Note: This only modifies config - symlinks and mounts are NOT removed");
    ui::dim("You must manually clean up symlinks if needed");

    Ok(())
}

// ============================================================================
// List Command
// ============================================================================

/// List resources of a given type
pub fn list(ctx: &Context, resource_type: ResourceType) -> Result<()> {
    match resource_type {
        ResourceType::Collections => list_collections(ctx),
        ResourceType::Repos => list_repos(ctx, None),
        ResourceType::Workspaces => list_workspaces(ctx),
        ResourceType::Storage => list_storage(ctx),
    }
}

fn list_collections(_ctx: &Context) -> Result<()> {
    ui::header("Collections");

    let config = BossaConfig::load()?;

    ui::kv("Total", &config.collections.len().to_string());
    println!();

    for (name, collection) in &config.collections {
        let path = collection.expanded_path()?;
        let exists = path.exists();
        let status = if exists {
            "✓".green()
        } else {
            "✗".yellow()
        };

        let storage_info = if let Some(ref storage_name) = collection.storage {
            format!(" [→ {}]", storage_name).dimmed().to_string()
        } else {
            String::new()
        };

        println!(
            "  {} {} {}{}",
            status,
            name.bold(),
            format!("({} repos)", collection.repos.len()).dimmed(),
            storage_info
        );
        println!("    {}", path.display().to_string().dimmed());
    }

    if config.collections.is_empty() {
        ui::dim("No collections configured");
    }

    Ok(())
}

fn list_repos(_ctx: &Context, _collection: Option<&str>) -> Result<()> {
    ui::header("Repositories");

    let config = BossaConfig::load()?;

    let mut total_repos = 0;
    let mut cloned_count = 0;

    for (coll_name, collection) in &config.collections {
        let path = collection.expanded_path()?;

        if !collection.repos.is_empty() {
            ui::section(&format!("Collection: {}", coll_name));

            for repo in &collection.repos {
                total_repos += 1;
                let repo_path = path.join(&repo.name);
                let exists = repo_path.exists();
                if exists {
                    cloned_count += 1;
                }
                let status = if exists {
                    "✓".green()
                } else {
                    "✗".yellow()
                };
                let status_text = if exists { "cloned" } else { "not cloned" };

                println!(
                    "  {} {} {}",
                    status,
                    repo.name.bold(),
                    format!("({})", status_text).dimmed()
                );
                println!("    {}", repo.url.dimmed());
            }
            println!();
        }
    }

    if total_repos == 0 {
        ui::dim("No repositories configured");
    } else {
        ui::kv("Total", &total_repos.to_string());
        ui::kv("Cloned", &format!("{}/{}", cloned_count, total_repos));
    }

    Ok(())
}

fn list_workspaces(_ctx: &Context) -> Result<()> {
    ui::header("Workspaces");

    let config = BossaConfig::load()?;

    ui::kv("Total", &config.workspaces.repos.len().to_string());

    // Group by category
    let categories = config.workspaces.categories();

    for category in &categories {
        println!();
        ui::section(&format!("Category: {}", category));

        let repos = config.workspaces.repos_by_category(category);
        for repo in repos {
            let worktree_count = repo.worktrees.len();

            println!(
                "  {} {} {}",
                "○".cyan(),
                repo.name.bold(),
                format!("({} worktrees)", worktree_count).dimmed()
            );
            println!("    {}", repo.url.dimmed());
        }
    }

    if config.workspaces.repos.is_empty() {
        println!();
        ui::dim("No workspaces configured");
    }

    Ok(())
}

fn list_storage(_ctx: &Context) -> Result<()> {
    ui::header("Storage");

    let config = BossaConfig::load()?;

    ui::kv("Total", &config.storage.len().to_string());
    println!();

    for (name, storage) in &config.storage {
        let mount = storage.expanded_mount()?;
        let is_mounted = mount.exists();
        let status = if is_mounted {
            "✓".green()
        } else {
            "✗".yellow()
        };
        let status_text = if is_mounted { "mounted" } else { "not mounted" };

        println!(
            "  {} {} {}",
            status,
            name.bold(),
            format!("({}, {} symlinks)", status_text, storage.symlinks.len()).dimmed()
        );
        println!("    {}", mount.display().to_string().dimmed());
    }

    if config.storage.is_empty() {
        ui::dim("No storage configured");
    }

    Ok(())
}

// ============================================================================
// Show Command
// ============================================================================

/// Show detailed information about a specific resource
pub fn show(_ctx: &Context, target: &str) -> Result<()> {
    // Parse target like "collections.refs", "workspaces.dotfiles", "storage.t9"
    let parts: Vec<&str> = target.split('.').collect();

    if parts.len() != 2 {
        bail!(
            "Invalid target format. Expected: <type>.<name> (e.g., collections.refs, workspaces.dotfiles)"
        );
    }

    let resource_type = parts[0];
    let resource_name = parts[1];

    match resource_type {
        "collections" => show_collection(resource_name),
        "collection" => show_collection(resource_name),
        "workspaces" => show_workspace(resource_name),
        "workspace" => show_workspace(resource_name),
        "storage" => show_storage(resource_name),
        _ => bail!(
            "Unknown resource type: {}. Valid types: collections, workspaces, storage",
            resource_type
        ),
    }
}

fn show_collection(name: &str) -> Result<()> {
    ui::header(&format!("Collection: {}", name));

    let config = BossaConfig::load()?;

    let collection = config
        .find_collection(name)
        .context(format!("Collection '{}' not found", name))?;

    let path = collection.expanded_path()?;

    ui::kv("Name", name);
    ui::kv("Path", &path.display().to_string());
    ui::kv("Repos", &collection.repos.len().to_string());

    if !collection.description.is_empty() {
        ui::kv("Description", &collection.description);
    }

    let exists = path.exists();
    let status = if exists {
        "exists".green()
    } else {
        "not created".yellow()
    };
    ui::kv("Status", &status.to_string());

    // Show clone settings
    if collection.clone.depth > 0 || collection.clone.single_branch {
        ui::section("Clone Settings");
        if collection.clone.depth > 0 {
            ui::kv("Depth", &collection.clone.depth.to_string());
        }
        if collection.clone.single_branch {
            ui::kv("Single branch", "true");
        }
    }

    // Show storage reference
    if let Some(ref storage_name) = collection.storage {
        ui::section("Storage");
        ui::kv("Linked to", storage_name);
    }

    // Show all repos
    if !collection.repos.is_empty() {
        ui::section("Repositories");

        for repo in &collection.repos {
            let repo_path = path.join(&repo.name);
            let exists = repo_path.exists();
            let status = if exists {
                "✓ cloned".green()
            } else {
                "✗ not cloned".yellow()
            };

            println!("  {} {}", repo.name.bold(), status);
            println!("    URL: {}", repo.url.dimmed());
            println!("    Branch: {}", repo.default_branch.dimmed());
            if exists {
                println!("    Path: {}", repo_path.display().to_string().dimmed());
            }
            if !repo.description.is_empty() {
                println!("    Description: {}", repo.description.dimmed());
            }
            println!();
        }
    } else {
        println!();
        ui::dim("No repositories configured");
    }

    Ok(())
}

fn show_workspace(name: &str) -> Result<()> {
    ui::header(&format!("Workspace: {}", name));

    let config = BossaConfig::load()?;

    let workspace = config
        .find_workspace_repo(name)
        .context(format!("Workspace '{}' not found", name))?;

    let root = config.workspaces.expanded_root()?;

    ui::kv("Name", &workspace.name);
    ui::kv("URL", &workspace.url);
    ui::kv("Category", &workspace.category);
    ui::kv("Worktrees", &workspace.worktrees.len().to_string());

    if !workspace.description.is_empty() {
        ui::kv("Description", &workspace.description);
    }

    // Show bare repo status
    let bare_path = workspace.bare_path(&root);
    let bare_exists = bare_path.exists();
    let status = if bare_exists {
        "✓ initialized".green()
    } else {
        "✗ not initialized".yellow()
    };
    ui::kv("Bare repo", &status.to_string());
    if bare_exists {
        ui::kv("Bare path", &bare_path.display().to_string());
    }

    // Show all worktrees
    if !workspace.worktrees.is_empty() {
        ui::section("Worktrees");

        for branch in &workspace.worktrees {
            let worktree_path = workspace.worktree_path(&root, branch);
            let exists = worktree_path.exists();
            let status = if exists {
                "✓".green()
            } else {
                "✗".yellow()
            };

            println!("  {} {}", status, branch.bold());
            println!("    Path: {}", worktree_path.display().to_string().dimmed());
        }
    } else {
        println!();
        ui::dim("No worktrees configured");
    }

    Ok(())
}

fn show_storage(name: &str) -> Result<()> {
    ui::header(&format!("Storage: {}", name));

    let config = BossaConfig::load()?;

    let storage = config
        .find_storage(name)
        .context(format!("Storage '{}' not found", name))?;

    let mount = storage.expanded_mount()?;
    let is_mounted = mount.exists();
    let status = if is_mounted {
        "✓ mounted".green()
    } else {
        "✗ not mounted".yellow()
    };

    ui::kv("Name", name);
    ui::kv("Mount point", &mount.display().to_string());
    ui::kv("Status", &status.to_string());
    ui::kv(
        "Type",
        &format!("{:?}", storage.storage_type).to_lowercase(),
    );

    if !storage.description.is_empty() {
        ui::kv("Description", &storage.description);
    }

    // Show symlinks
    if !storage.symlinks.is_empty() {
        ui::section("Symlinks");

        for symlink in &storage.symlinks {
            let from = symlink.expanded_from()?;
            let to = symlink.expanded_to(&mount.display().to_string())?;

            let from_exists = from.exists();
            let is_valid_symlink = from.is_symlink() && from.read_link().ok() == Some(to.clone());

            let status = if is_valid_symlink {
                "✓".green()
            } else if from_exists {
                "⚠".yellow()
            } else {
                "✗".yellow()
            };

            let status_text = if is_valid_symlink {
                "linked".green()
            } else if from_exists {
                "exists but not linked".yellow()
            } else {
                "not created".dimmed()
            };

            println!("  {} {} {}", status, from.display(), status_text);
            println!("    → {}", to.display().to_string().dimmed());
        }
    } else {
        println!();
        ui::dim("No symlinks configured");
    }

    Ok(())
}
