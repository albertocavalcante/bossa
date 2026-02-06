use anyhow::{Context as AnyhowContext, Result};
use colored::Colorize;
use rayon::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use crate::Context;
use crate::config;
use crate::progress;
use crate::runner;
use crate::schema::{BossaConfig, CollectionRepo};
use crate::ui;

// ============================================================================
// Command Enum
// ============================================================================

#[derive(Debug)]
pub enum CollectionsCommand {
    List,
    Status {
        name: String,
    },
    Sync {
        name: String,
        jobs: usize,
        retries: usize,
        dry_run: bool,
    },
    Audit {
        name: String,
        fix: bool,
    },
    Snapshot {
        name: String,
    },
    Add {
        collection: String,
        url: String,
        name: Option<String>,
        clone: bool,
    },
    Rm {
        collection: String,
        repo: String,
        delete: bool,
    },
    Clean {
        name: String,
        yes: bool,
        dry_run: bool,
    },
}

pub fn run(ctx: &Context, cmd: CollectionsCommand) -> Result<()> {
    match cmd {
        CollectionsCommand::List => list(ctx),
        CollectionsCommand::Status { name } => status(ctx, &name),
        CollectionsCommand::Sync {
            name,
            jobs,
            retries,
            dry_run,
        } => sync(ctx, &name, jobs, retries, dry_run),
        CollectionsCommand::Audit { name, fix } => audit(ctx, &name, fix),
        CollectionsCommand::Snapshot { name } => snapshot(ctx, &name),
        CollectionsCommand::Add {
            collection,
            url,
            name,
            clone,
        } => add(ctx, &collection, &url, name, clone),
        CollectionsCommand::Rm {
            collection,
            repo,
            delete,
        } => rm(ctx, &collection, &repo, delete),
        CollectionsCommand::Clean { name, yes, dry_run } => clean(ctx, &name, yes, dry_run),
    }
}

// ============================================================================
// List Collections
// ============================================================================

fn list(_ctx: &Context) -> Result<()> {
    ui::header("Collections");

    let config = BossaConfig::load()?;

    if config.collections.is_empty() {
        ui::info("No collections defined in config");
        return Ok(());
    }

    ui::kv("Total collections", &config.collections.len().to_string());
    println!();

    for (name, collection) in &config.collections {
        let path = collection.expanded_path()?;
        let exists = path.exists();
        let status = if exists {
            "✓".green()
        } else {
            "✗".yellow()
        };

        println!(
            "  {} {} ({})",
            status,
            name.bold(),
            collection.description.dimmed()
        );
        println!("    Path: {}", path.display());
        println!("    Repos: {}", collection.repos.len());
        println!();
    }

    Ok(())
}

// ============================================================================
// Status - Show collection details
// ============================================================================

fn status(_ctx: &Context, name: &str) -> Result<()> {
    ui::header(&format!("Collection: {name}"));

    let config = BossaConfig::load()?;
    let collection = config
        .find_collection(name)
        .with_context(|| format!("Collection '{name}' not found"))?;

    let root = collection.expanded_path()?;

    ui::kv("Path", &root.display().to_string());
    ui::kv("Description", &collection.description);
    ui::kv("Total repos", &collection.repos.len().to_string());
    println!();

    for repo in &collection.repos {
        let path = root.join(&repo.name);
        let exists = path.exists();
        let status = if exists {
            "✓".green()
        } else {
            "✗".yellow()
        };

        println!(
            "  {} {} {}",
            status,
            repo.name,
            format!("({})", repo.default_branch).dimmed()
        );
        println!("    {}", repo.url.dimmed());
    }

    Ok(())
}

// ============================================================================
// Sync - Clone missing repos
// ============================================================================

fn sync(
    ctx: &Context,
    collection_name: &str,
    jobs: usize,
    retries: usize,
    dry_run: bool,
) -> Result<()> {
    ui::header(&format!("Syncing Collection: {collection_name}"));

    let config = BossaConfig::load()?;
    let collection = config
        .find_collection(collection_name)
        .with_context(|| format!("Collection '{collection_name}' not found"))?;

    let root = collection.expanded_path()?;

    // Ensure root directory exists
    fs::create_dir_all(&root)?;

    // Filter repos to clone (only those not on disk)
    let repos_to_clone: Vec<_> = collection
        .repos
        .iter()
        .filter(|r| !root.join(&r.name).exists())
        .cloned()
        .collect();

    if repos_to_clone.is_empty() {
        ui::success("All repositories already cloned!");
        return Ok(());
    }

    ui::kv("Root", &root.display().to_string());
    ui::kv("To clone", &repos_to_clone.len().to_string());
    ui::kv("Parallel jobs", &jobs.to_string());
    ui::kv("Retries", &retries.to_string());
    println!();

    if dry_run {
        ui::warn("Dry run - no changes will be made");
        for repo in &repos_to_clone {
            println!("  {} {}", "→".cyan(), repo.name);
        }
        return Ok(());
    }

    // Clone in parallel with progress bar
    let pb = progress::clone_bar(repos_to_clone.len() as u64, "Cloning");
    let cloned = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let failed_repos: Arc<std::sync::Mutex<Vec<(String, String)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    // Configure rayon thread pool
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .unwrap();

    let clone_settings = &collection.clone;

    pool.install(|| {
        repos_to_clone.par_iter().for_each(|repo| {
            let result = clone_repo_with_retry(&root, repo, clone_settings, retries);

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

    // Summary
    let cloned_count = cloned.load(Ordering::Relaxed);
    let failed_count = failed.load(Ordering::Relaxed);

    println!();
    if failed_count == 0 {
        ui::success(&format!("Cloned {cloned_count} repositories successfully!"));
    } else {
        ui::warn(&format!("Cloned {cloned_count}, {failed_count} failed"));

        if !ctx.quiet {
            println!();
            ui::error("Failed repositories:");
            for (name, error) in failed_repos.lock().unwrap().iter() {
                println!("  {} {} - {}", "✗".red(), name, error.dimmed());
            }
        }
    }

    Ok(())
}

/// Clone a single repo with retry logic
fn clone_repo_with_retry(
    root: &std::path::Path,
    repo: &CollectionRepo,
    clone_settings: &crate::schema::CloneSettings,
    max_retries: usize,
) -> Result<()> {
    let repo_path = root.join(&repo.name);

    for attempt in 1..=max_retries {
        let result = clone_repo(root, repo, clone_settings);

        match result {
            Ok(()) => return Ok(()),
            Err(e) => {
                let error_str = e.to_string();
                let is_retryable = is_network_error(&error_str);

                if attempt < max_retries && is_retryable {
                    // Exponential backoff: 2^attempt seconds
                    let delay = Duration::from_secs(2_u64.pow(attempt as u32));
                    thread::sleep(delay);

                    // Clean up partial clone
                    let _ = fs::remove_dir_all(&repo_path);
                } else {
                    // Clean up and return error
                    let _ = fs::remove_dir_all(&repo_path);
                    return Err(e);
                }
            }
        }
    }

    anyhow::bail!("Max retries exceeded")
}

/// Clone a single repository
fn clone_repo(
    root: &std::path::Path,
    repo: &CollectionRepo,
    clone_settings: &crate::schema::CloneSettings,
) -> Result<()> {
    let repo_path = root.join(&repo.name);

    let mut args = vec!["clone".to_string()];

    // Apply clone settings
    if clone_settings.depth > 0 {
        args.push("--depth".to_string());
        args.push(clone_settings.depth.to_string());
    }

    if clone_settings.single_branch {
        args.push("--single-branch".to_string());
    }

    // Add custom options
    args.extend(clone_settings.options.iter().cloned());

    // Add URL and path
    args.push(repo.url.clone());
    args.push(repo_path.to_str().unwrap().to_string());

    // Run git clone
    let output = std::process::Command::new("git")
        .args(&args)
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

/// Check if an error is likely a network error (retryable)
fn is_network_error(error: &str) -> bool {
    let network_patterns = [
        "Could not resolve",
        "Connection refused",
        "Connection timed out",
        "SSL",
        "unable to access",
        "Temporary failure",
        "Network is unreachable",
        "curl",
        "fetch",
    ];

    network_patterns
        .iter()
        .any(|p| error.to_lowercase().contains(&p.to_lowercase()))
}

// ============================================================================
// Audit - Detect drift
// ============================================================================

fn audit(_ctx: &Context, collection_name: &str, fix: bool) -> Result<()> {
    ui::header(&format!("Collection Audit: {collection_name}"));

    let mut config = BossaConfig::load()?;
    let collection = config
        .find_collection(collection_name)
        .with_context(|| format!("Collection '{collection_name}' not found"))?
        .clone();

    let root = collection.expanded_path()?;

    if !root.exists() {
        ui::warn(&format!(
            "Collection directory does not exist: {}",
            root.display()
        ));
        return Ok(());
    }

    let pb = progress::spinner("Scanning for untracked repos...");

    // Get all directories in collection
    let entries: Vec<_> = fs::read_dir(&root)?
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_dir())
        .filter(|e| e.path().join(".git").exists())
        .collect();

    // Find repos not in config
    let tracked_names: std::collections::HashSet<_> =
        collection.repos.iter().map(|r| r.name.clone()).collect();

    let mut untracked: Vec<(String, PathBuf)> = Vec::new();
    let mut tracked_count = 0;

    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if tracked_names.contains(&name) {
            tracked_count += 1;
        } else {
            untracked.push((name, entry.path()));
        }
    }

    progress::finish_success(&pb, "Scan complete");

    ui::kv("Tracked", &format!("{tracked_count} repos"));
    ui::kv("Untracked", &format!("{} repos", untracked.len()));

    if untracked.is_empty() {
        println!();
        ui::success("No drift detected - all repos are tracked in config");
        return Ok(());
    }

    println!();
    ui::warn("Untracked repos (exist locally but not in config):");

    for (name, _) in &untracked {
        println!("  {} {}", "?".yellow(), name);
    }

    if fix {
        println!();
        ui::info("Adding untracked repos to config...");

        let collection_mut = config.find_collection_mut(collection_name).unwrap();

        for (name, path) in &untracked {
            // Get remote URL
            let url = runner::run_capture(
                "git",
                &[
                    "-C",
                    path.to_str().unwrap(),
                    "config",
                    "--get",
                    "remote.origin.url",
                ],
            )?;

            // Get default branch
            let default_branch = runner::run_capture(
                "git",
                &[
                    "-C",
                    path.to_str().unwrap(),
                    "symbolic-ref",
                    "refs/remotes/origin/HEAD",
                ],
            )
            .map_or_else(
                |_| "main".to_string(),
                |s| s.split('/').next_back().unwrap_or("main").to_string(),
            );

            collection_mut.add_repo(CollectionRepo {
                name: name.clone(),
                url,
                default_branch,
                description: String::new(),
            });

            println!("  {} {}", "✓".green(), name);
        }

        config.save()?;
        ui::success(&format!("Added {} repos to config.toml", untracked.len()));
    } else {
        println!();
        ui::info(&format!(
            "Run 'bossa collections audit {collection_name} --fix' to add these repos to config"
        ));
        ui::info(&format!(
            "Or 'bossa collections snapshot {collection_name}' to regenerate the entire collection"
        ));
    }

    Ok(())
}

// ============================================================================
// Snapshot - Regenerate from disk
// ============================================================================

fn snapshot(_ctx: &Context, collection_name: &str) -> Result<()> {
    ui::header(&format!("Capturing Snapshot: {collection_name}"));

    let mut config = BossaConfig::load()?;
    let collection = config
        .find_collection(collection_name)
        .with_context(|| format!("Collection '{collection_name}' not found"))?
        .clone();

    let root = collection.expanded_path()?;

    if !root.exists() {
        ui::warn(&format!(
            "Collection directory does not exist: {}",
            root.display()
        ));
        return Ok(());
    }

    let pb = progress::spinner(&format!("Scanning {}...", root.display()));

    let mut repositories = Vec::new();

    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        if !path.join(".git").exists() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();

        // Get remote URL
        let url = match runner::run_capture(
            "git",
            &[
                "-C",
                path.to_str().unwrap(),
                "config",
                "--get",
                "remote.origin.url",
            ],
        ) {
            Ok(u) => u,
            Err(_) => continue,
        };

        // Get default branch
        let default_branch = runner::run_capture(
            "git",
            &[
                "-C",
                path.to_str().unwrap(),
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
            ],
        )
        .map_or_else(
            |_| "main".to_string(),
            |s| s.split('/').next_back().unwrap_or("main").to_string(),
        );

        repositories.push(CollectionRepo {
            name,
            url,
            default_branch,
            description: String::new(),
        });
    }

    repositories.sort_by(|a, b| a.name.cmp(&b.name));

    let collection_mut = config.find_collection_mut(collection_name).unwrap();
    collection_mut.repos = repositories;

    let count = collection_mut.repos.len();
    config.save()?;

    progress::finish_success(&pb, &format!("Captured {count} repositories"));

    let config_path = config::config_dir()?.join("config.toml");
    ui::dim(&format!("Saved to: {}", config_path.display()));

    Ok(())
}

// ============================================================================
// Add - Add repo to collection
// ============================================================================

fn add(
    ctx: &Context,
    collection_name: &str,
    url: &str,
    name: Option<String>,
    clone: bool,
) -> Result<()> {
    let repo_name = name
        .or_else(|| config::repo_name_from_url(url))
        .context("Could not determine repo name from URL. Use --name to specify.")?;

    ui::header(&format!("Adding Repo to {collection_name}: {repo_name}"));

    // Detect default branch with spinner
    let pb = progress::spinner("Detecting default branch...");
    let default_branch = config::detect_default_branch(url);
    progress::finish_success(&pb, &format!("Default branch: {default_branch}"));

    // Load config
    let mut config = BossaConfig::load()?;

    // Check if collection exists, create if not
    if !config.collections.contains_key(collection_name) {
        anyhow::bail!(
            "Collection '{collection_name}' not found. Create it first with 'bossa add collection {collection_name}'"
        );
    }

    // Check if already exists
    {
        let collection = config.find_collection(collection_name).unwrap();
        if collection.find_repo(&repo_name).is_some() {
            ui::warn(&format!("Repo '{repo_name}' already exists in collection"));
        }
    }

    // Add repo
    {
        let collection = config.find_collection_mut(collection_name).unwrap();
        collection.add_repo(CollectionRepo {
            name: repo_name.clone(),
            url: url.to_string(),
            default_branch,
            description: String::new(),
        });
    }

    // Save config
    config.save()?;
    ui::success(&format!(
        "Added '{repo_name}' to collection '{collection_name}'"
    ));

    // Clone if requested
    if clone {
        println!();
        let pb = progress::spinner(&format!("Cloning {repo_name}..."));

        // Get collection data for cloning
        let collection = config.find_collection(collection_name).unwrap();
        let root = collection.expanded_path()?;
        fs::create_dir_all(&root)?;

        let repo = collection.find_repo(&repo_name).unwrap().clone();
        let clone_settings = collection.clone.clone();

        match clone_repo_with_retry(&root, &repo, &clone_settings, 3) {
            Ok(()) => {
                progress::finish_success(&pb, &format!("Cloned '{repo_name}'"));
            }
            Err(e) => {
                progress::finish_error(&pb, &format!("Failed to clone: {e}"));
                if !ctx.quiet {
                    ui::info(&format!(
                        "You can retry later with: bossa collections sync {collection_name}"
                    ));
                }
            }
        }
    }

    Ok(())
}

// ============================================================================
// Remove - Remove repo from collection
// ============================================================================

fn rm(_ctx: &Context, collection_name: &str, repo_name: &str, delete: bool) -> Result<()> {
    ui::header(&format!(
        "Removing Repo from {collection_name}: {repo_name}"
    ));

    let mut config = BossaConfig::load()?;

    let collection = config
        .find_collection_mut(collection_name)
        .with_context(|| format!("Collection '{collection_name}' not found"))?;

    if !collection.remove_repo(repo_name) {
        ui::warn(&format!("Repo '{repo_name}' not found in collection"));
        return Ok(());
    }

    let root = collection.expanded_path()?;

    config.save()?;
    ui::success(&format!(
        "Removed '{repo_name}' from collection '{collection_name}'"
    ));

    if delete {
        let repo_path = root.join(repo_name);

        if repo_path.exists() {
            let pb = progress::spinner(&format!("Deleting {}...", repo_path.display()));
            fs::remove_dir_all(&repo_path)?;
            progress::finish_success(&pb, "Local clone deleted");
        }
    }

    Ok(())
}

// ============================================================================
// Clean - Delete all clones from disk, preserve config
// ============================================================================

fn clean(_ctx: &Context, collection_name: &str, skip_confirm: bool, dry_run: bool) -> Result<()> {
    ui::header(&format!("Clean Collection: {collection_name}"));

    let config = BossaConfig::load()?;
    let collection = config
        .find_collection(collection_name)
        .with_context(|| format!("Collection '{collection_name}' not found"))?;

    let root = collection.expanded_path()?;

    if !root.exists() {
        ui::info("Collection directory does not exist. Nothing to clean.");
        return Ok(());
    }

    // Scan for cloned repos (directories with .git)
    let pb = progress::spinner("Scanning for cloned repositories...");

    let mut cloned_repos: Vec<(String, PathBuf, u64)> = Vec::new();

    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        if !path.join(".git").exists() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        let size = dir_size(&path).unwrap_or(0);
        cloned_repos.push((name, path, size));
    }

    progress::finish_success(&pb, "Scan complete");

    if cloned_repos.is_empty() {
        ui::info("No cloned repositories found. Nothing to clean.");
        return Ok(());
    }

    // Calculate total size
    let total_size: u64 = cloned_repos.iter().map(|(_, _, s)| s).sum();
    let total_size_str = format_size(total_size);

    println!();
    ui::kv("Path", &root.display().to_string());
    ui::kv("Cloned repos", &cloned_repos.len().to_string());
    ui::kv("Total size", &total_size_str);
    println!();

    if dry_run {
        ui::info("Dry run - repos that would be deleted:");
        for (name, _, size) in &cloned_repos {
            println!("  {} {} ({})", "−".red(), name, format_size(*size).dimmed());
        }
        println!();
        ui::dim(&format!("Would free {total_size_str} of disk space"));
        ui::dim("Run without --dry-run to actually delete");
        return Ok(());
    }

    // Show warning
    println!(
        "  {} This will DELETE {} cloned repositories from disk.",
        "⚠".yellow(),
        cloned_repos.len()
    );
    println!(
        "  {} Config will be PRESERVED (re-clone with 'bossa collections sync {}')",
        "✓".green(),
        collection_name
    );
    println!();

    // Confirmation
    if !skip_confirm {
        print!(
            "  Type '{}' to confirm: ",
            format!("clean {collection_name}").bold()
        );
        std::io::Write::flush(&mut std::io::stdout())?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        let expected = format!("clean {collection_name}");
        if input.trim() != expected {
            println!();
            ui::warn("Aborted. No changes made.");
            return Ok(());
        }
    }

    println!();

    // Delete repos
    let mut deleted = 0;
    let mut freed: u64 = 0;

    for (name, path, size) in &cloned_repos {
        let pb = progress::spinner(&format!("Deleting {name}..."));

        match fs::remove_dir_all(path) {
            Ok(()) => {
                progress::finish_success(&pb, &format!("Deleted {name}"));
                deleted += 1;
                freed += size;
            }
            Err(e) => {
                progress::finish_error(&pb, &format!("Failed to delete {name}: {e}"));
            }
        }
    }

    println!();
    ui::success(&format!(
        "Cleaned {} repositories (freed {})",
        deleted,
        format_size(freed)
    ));
    ui::dim(&format!(
        "Config preserved. Run 'bossa collections sync {collection_name}' to re-clone."
    ));

    Ok(())
}

/// Calculate directory size recursively
fn dir_size(path: &PathBuf) -> Result<u64> {
    let mut size = 0;

    if path.is_file() {
        return Ok(fs::metadata(path)?.len());
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            size += fs::metadata(&path)?.len();
        } else if path.is_dir() {
            size += dir_size(&path)?;
        }
    }

    Ok(size)
}

/// Format bytes as human-readable size
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
