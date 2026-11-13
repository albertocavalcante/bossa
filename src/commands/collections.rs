use anyhow::{Context as AnyhowContext, Result};
use colored::Colorize;
use rayon::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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

impl From<crate::cli::CollectionsCommand> for CollectionsCommand {
    fn from(cmd: crate::cli::CollectionsCommand) -> Self {
        match cmd {
            crate::cli::CollectionsCommand::List => Self::List,
            crate::cli::CollectionsCommand::Status { name } => Self::Status { name },
            crate::cli::CollectionsCommand::Sync {
                name,
                jobs,
                retries,
                dry_run,
            } => Self::Sync {
                name,
                jobs,
                retries,
                dry_run,
            },
            crate::cli::CollectionsCommand::Audit { name, fix } => Self::Audit { name, fix },
            crate::cli::CollectionsCommand::Snapshot { name } => Self::Snapshot { name },
            crate::cli::CollectionsCommand::Add {
                collection,
                url,
                name,
                clone,
            } => Self::Add {
                collection,
                url,
                name,
                clone,
            },
            crate::cli::CollectionsCommand::Rm {
                collection,
                repo,
                delete,
            } => Self::Rm {
                collection,
                repo,
                delete,
            },
            crate::cli::CollectionsCommand::Clean { name, yes, dry_run } => {
                Self::Clean { name, yes, dry_run }
            }
        }
    }
}

impl From<crate::cli::RefsCommand> for CollectionsCommand {
    fn from(cmd: crate::cli::RefsCommand) -> Self {
        match cmd {
            crate::cli::RefsCommand::Sync(args) => {
                let name = args.name.unwrap_or_else(|| "refs".to_string());
                Self::Sync {
                    name,
                    jobs: args.jobs,
                    retries: args.retries,
                    dry_run: args.dry_run,
                }
            }
            crate::cli::RefsCommand::List {
                filter: _,
                missing: _,
            } => Self::Status {
                name: "refs".to_string(),
            },
            crate::cli::RefsCommand::Snapshot => Self::Snapshot {
                name: "refs".to_string(),
            },
            crate::cli::RefsCommand::Audit { fix } => Self::Audit {
                name: "refs".to_string(),
                fix,
            },
            crate::cli::RefsCommand::Add { url, name, clone } => Self::Add {
                collection: "refs".to_string(),
                url,
                name,
                clone,
            },
            crate::cli::RefsCommand::Remove { name, delete } => Self::Rm {
                collection: "refs".to_string(),
                repo: name,
                delete,
            },
        }
    }
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
        let path = collection.expanded_path(crate::paths::environment())?;
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

    let root = collection.expanded_path(crate::paths::environment())?;
    let state_root = crate::paths::state_dir()?;

    ui::kv("Path", &root.display().to_string());
    ui::kv("Description", &collection.description);
    ui::kv("Total repos", &collection.repos.len().to_string());
    println!();

    let mut failures = false;
    for repo in &collection.repos {
        let observation = observe_checkout(&root, &state_root, repo);
        let status = match &observation {
            Ok(true) => "✓".green(),
            Ok(false) => "✗".yellow(),
            Err(_) => "!".red(),
        };

        println!(
            "  {} {} {}",
            status,
            repo.name,
            format!("({})", repo.default_branch).dimmed()
        );
        println!("    {}", repo.url.dimmed());
        if let Err(error) = observation {
            println!("    {error:#}");
            failures = true;
        }
    }
    if failures {
        anyhow::bail!("Some collection destinations could not be verified");
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

    let root = collection.expanded_path(crate::paths::environment())?;
    let state_root = crate::paths::state_dir()?;

    // Ensure root directory exists
    fs::create_dir_all(&root)?;

    let mut repos_to_clone = Vec::new();
    for repo in &collection.repos {
        if !observe_checkout(&root, &state_root, repo)? {
            repos_to_clone.push(repo.clone());
        }
    }

    if repos_to_clone.is_empty() {
        ui::success("All repositories already cloned!");
        return Ok(());
    }

    ui::kv("Root", &root.display().to_string());
    ui::kv("To clone", &repos_to_clone.len().to_string());
    ui::kv("Parallel jobs", &jobs.to_string());
    ui::kv("Retries", "disabled; failed destinations are preserved");
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
        .context("Failed to create clone thread pool")?;

    let clone_settings = &collection.clone;

    pool.install(|| {
        repos_to_clone.par_iter().for_each(|repo| {
            let result = clone_repo_with_retry(&root, &state_root, repo, clone_settings, retries);

            match result {
                Ok(()) => {
                    cloned.fetch_add(1, Ordering::Relaxed);
                    pb.set_message(format!("{} ✓", repo.name));
                }
                Err(e) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    match failed_repos.lock() {
                        Ok(mut failures) => failures.push((repo.name.clone(), e.to_string())),
                        Err(poisoned) => poisoned
                            .into_inner()
                            .push((repo.name.clone(), e.to_string())),
                    }
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
            let failures = match failed_repos.lock() {
                Ok(failures) => failures,
                Err(poisoned) => poisoned.into_inner(),
            };
            for (name, error) in failures.iter() {
                println!("  {} {} - {}", "✗".red(), name, error.dimmed());
            }
        }
        anyhow::bail!(
            "{failed_count} collection clone(s) failed; inspect preserved destinations before retrying"
        );
    }

    Ok(())
}

fn observe_checkout(
    root: &std::path::Path,
    state_root: &std::path::Path,
    repo: &CollectionRepo,
) -> Result<bool> {
    validate_clone_name(repo)?;
    let path = root.join(&repo.name);
    let marker = pending_marker(root, state_root, repo)?;
    match fs::symlink_metadata(&marker) {
        Ok(_) => anyhow::bail!(
            "Pending clone record exists at {}; inspect it and {} before retrying",
            marker.display(),
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Cannot inspect pending clone record"),
    }
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot inspect {}", path.display()));
        }
        Ok(metadata) if !metadata.is_dir() => {
            anyhow::bail!("Destination is not a directory: {}", path.display())
        }
        Ok(_) => {}
    }
    verify_checkout(&path)?;
    Ok(true)
}

fn pending_marker(
    root: &std::path::Path,
    state_root: &std::path::Path,
    repo: &CollectionRepo,
) -> Result<PathBuf> {
    let destination = bossa::setup::resolve_location(&std::path::absolute(root.join(&repo.name))?)?;
    let key = blake3::hash(destination.as_os_str().as_encoded_bytes());
    Ok(state_root
        .join("collection-clones")
        .join(format!("{key}.pending")))
}

fn verify_checkout(path: &std::path::Path) -> Result<()> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .output()
        .with_context(|| format!("Cannot inspect Git checkout {}", path.display()))?;
    if !output.status.success() {
        anyhow::bail!(
            "Destination is not a verified Git checkout: {} ({})",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let git_root = std::str::from_utf8(&output.stdout)
        .context("Git checkout path is not UTF-8")?
        .trim_end_matches(['\r', '\n']);
    if fs::canonicalize(git_root)? != fs::canonicalize(path)? {
        anyhow::bail!(
            "Destination is not the root of its Git checkout: {}",
            path.display()
        );
    }
    Ok(())
}

/// Clone a single repo with retry logic
fn clone_repo_with_retry(
    root: &std::path::Path,
    state_root: &std::path::Path,
    repo: &CollectionRepo,
    clone_settings: &crate::schema::CloneSettings,
    max_retries: usize,
) -> Result<()> {
    clone_repo_with_program(
        root,
        state_root,
        repo,
        clone_settings,
        max_retries,
        std::ffi::OsStr::new("git"),
    )
}

fn clone_repo_with_program(
    root: &std::path::Path,
    state_root: &std::path::Path,
    repo: &CollectionRepo,
    clone_settings: &crate::schema::CloneSettings,
    _max_retries: usize,
    git: &std::ffi::OsStr,
) -> Result<()> {
    validate_clone_name(repo)?;
    let repo_path = root.join(&repo.name);

    // Reserve the destination atomically. Never remove it after a failed clone:
    // another process may have put data there while Git was running.
    fs::create_dir(&repo_path)
        .with_context(|| format!("Cannot reserve clone destination {}", repo_path.display()))?;
    let marker = pending_marker(root, state_root, repo)?;
    fs::create_dir_all(marker.parent().context("Pending record has no parent")?)
        .context("Cannot create pending clone record directory; Git was not started")?;
    let _record = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
        .with_context(|| {
            format!(
                "Cannot create pending clone record {}; Git was not started",
                marker.display()
            )
        })?;
    clone_repo(root, repo, clone_settings, git).map_err(|error| anyhow::anyhow!(
        "Clone failed: {error:#}; destination and pending record preserved at {} and {}. Inspect them before moving or removing them, then retry.",
        repo_path.display(), marker.display()
    ))?;
    verify_checkout(&repo_path).context(
        "Clone returned success but checkout could not be verified; pending record preserved",
    )?;
    fs::remove_file(&marker)
        .context("Checkout verified but pending clone record could not be cleared")?;
    Ok(())
}

fn validate_destination_name(name: &str) -> Result<()> {
    if name.trim().is_empty()
        || name.starts_with('-')
        || (name.contains("..") || name.contains(['/', '\\', ':']))
        || name.chars().any(char::is_control)
    {
        anyhow::bail!("Repository name must be a safe single directory name");
    }
    let mut components = std::path::Path::new(name).components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
    {
        anyhow::bail!("Repository name must be a single directory name");
    }
    Ok(())
}

fn validate_clone_name(repo: &CollectionRepo) -> Result<()> {
    validate_destination_name(&repo.name)?;
    repo.validate()
}

/// Clone a single repository
fn clone_repo(
    root: &std::path::Path,
    repo: &CollectionRepo,
    clone_settings: &crate::schema::CloneSettings,
    git: &std::ffi::OsStr,
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
    let repo_path_str = repo_path.to_string_lossy().into_owned();
    args.push(repo.url.clone());
    args.push(repo_path_str.clone());

    // Run git clone
    let output = std::process::Command::new(git)
        .args(&args)
        .output()
        .context("Failed to execute git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    // Configure for exFAT (T9 drive)
    let _ = std::process::Command::new(git)
        .args([
            "-C",
            repo_path_str.as_str(),
            "config",
            "--local",
            "core.fileMode",
            "false",
        ])
        .output();

    Ok(())
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

    let root = collection.expanded_path(crate::paths::environment())?;

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

        let collection_mut = config
            .find_collection_mut(collection_name)
            .with_context(|| format!("Collection '{collection_name}' not found"))?;

        for (name, path) in &untracked {
            let path_str = path.to_string_lossy().into_owned();

            // Get remote URL
            let url = runner::run_capture(
                "git",
                &[
                    "-C",
                    path_str.as_str(),
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
                    path_str.as_str(),
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

    let root = collection.expanded_path(crate::paths::environment())?;

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
        let path_str = path.to_string_lossy().into_owned();

        // Get remote URL
        let url = match runner::run_capture(
            "git",
            &[
                "-C",
                path_str.as_str(),
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
                path_str.as_str(),
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

    let collection_mut = config
        .find_collection_mut(collection_name)
        .with_context(|| format!("Collection '{collection_name}' not found"))?;
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
    _ctx: &Context,
    collection_name: &str,
    url: &str,
    name: Option<String>,
    clone: bool,
) -> Result<()> {
    let repo_name = name
        .or_else(|| config::repo_name_from_url(url))
        .context("Could not determine repo name from URL. Use --name to specify.")?;
    validate_destination_name(&repo_name)?;

    ui::header(&format!("Adding Repo to {collection_name}: {repo_name}"));

    // Detect default branch with spinner
    let pb = progress::spinner("Detecting default branch...");
    let default_branch = config::detect_default_branch(url);
    progress::finish_success(&pb, &format!("Default branch: {default_branch}"));
    let repo = CollectionRepo {
        name: repo_name.clone(),
        url: url.to_string(),
        default_branch,
        description: String::new(),
    };
    validate_clone_name(&repo)?;

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
        let collection = config
            .find_collection(collection_name)
            .with_context(|| format!("Collection '{collection_name}' not found"))?;
        if collection.find_repo(&repo_name).is_some() {
            ui::warn(&format!("Repo '{repo_name}' already exists in collection"));
        }
    }

    // Add repo
    {
        let collection = config
            .find_collection_mut(collection_name)
            .with_context(|| format!("Collection '{collection_name}' not found"))?;
        collection.add_repo(repo);
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
        let collection = config
            .find_collection(collection_name)
            .with_context(|| format!("Collection '{collection_name}' not found"))?;
        let root = collection.expanded_path(crate::paths::environment())?;
        let state_root = crate::paths::state_dir()?;
        fs::create_dir_all(&root)?;

        let repo = collection
            .find_repo(&repo_name)
            .with_context(|| {
                format!("Repo '{repo_name}' not found in collection '{collection_name}'")
            })?
            .clone();
        let clone_settings = collection.clone.clone();

        match clone_repo_with_retry(&root, &state_root, &repo, &clone_settings, 3) {
            Ok(()) => {
                progress::finish_success(&pb, &format!("Cloned '{repo_name}'"));
            }
            Err(e) => {
                progress::finish_error(&pb, &format!("Failed to clone: {e}"));
                return Err(e);
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

    let root = collection.expanded_path(crate::paths::environment())?;

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

    let root = collection.expanded_path(crate::paths::environment())?;

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
    let total_size_str = ui::format_size(total_size);

    println!();
    ui::kv("Path", &root.display().to_string());
    ui::kv("Cloned repos", &cloned_repos.len().to_string());
    ui::kv("Total size", &total_size_str);
    println!();

    if dry_run {
        ui::info("Dry run - repos that would be deleted:");
        for (name, _, size) in &cloned_repos {
            println!(
                "  {} {} ({})",
                "−".red(),
                name,
                ui::format_size(*size).dimmed()
            );
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
        ui::format_size(freed)
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

#[cfg(test)]
mod tests {
    use super::{CollectionsCommand, clone_repo_with_retry, observe_checkout};
    use crate::cli::{
        CollectionsCommand as CliCollectionsCommand, RefsCommand as CliRefsCommand, RefsSyncArgs,
    };
    use crate::schema::{CloneSettings, CollectionRepo};
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn clone_fixture(
        root: &std::path::Path,
        name: &str,
        source: &std::path::Path,
    ) -> anyhow::Result<()> {
        clone_repo_with_retry(
            root,
            &root.join("state"),
            &CollectionRepo {
                name: name.into(),
                url: source.to_string_lossy().into_owned(),
                default_branch: "main".into(),
                description: String::new(),
            },
            &CloneSettings::default(),
            0,
        )
    }

    #[test]
    fn existing_destinations_are_preserved() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let source = root.join("missing-source");
        let checkout = root.join("checkout");
        fs::create_dir(&checkout).unwrap();
        fs::write(checkout.join("work"), "unsaved").unwrap();
        fs::write(root.join("file"), "file content").unwrap();
        fs::create_dir(root.join("empty")).unwrap();
        for name in ["checkout", "file", "empty", "", ".", "..", "../escape"] {
            assert!(clone_fixture(root, name, &source).is_err(), "{name:?}");
        }
        assert_eq!(
            fs::read_to_string(checkout.join("work")).unwrap(),
            "unsaved"
        );
        assert_eq!(
            fs::read_to_string(root.join("file")).unwrap(),
            "file content"
        );
        assert!(root.join("empty").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn existing_symlinks_are_preserved() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(root.join("target"), "data").unwrap();
        std::os::unix::fs::symlink(root.join("target"), root.join("link")).unwrap();
        std::os::unix::fs::symlink(root.join("missing"), root.join("dangling")).unwrap();
        for name in ["link", "dangling"] {
            assert!(clone_fixture(root, name, &root.join("missing-source")).is_err());
            assert!(
                fs::symlink_metadata(root.join(name))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
        assert_eq!(fs::read_to_string(root.join("target")).unwrap(), "data");
    }

    #[test]
    fn clone_success_and_failure_keep_reserved_destination() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let source = root.join("source.git");
        assert!(
            Command::new("git")
                .args(["init", "--bare", "--quiet"])
                .arg(&source)
                .status()
                .unwrap()
                .success()
        );
        clone_fixture(root, "clone", &source).unwrap();
        assert!(root.join("clone/.git").is_dir());
        let repo = |name: &str| CollectionRepo {
            name: name.into(),
            url: source.to_string_lossy().into_owned(),
            default_branch: "main".into(),
            description: String::new(),
        };
        assert!(observe_checkout(root, &root.join("state"), &repo("clone")).unwrap());
        assert!(!observe_checkout(root, &root.join("state"), &repo("absent")).unwrap());
        assert!(clone_fixture(root, "failed", &root.join("missing-source")).is_err());
        assert!(root.join("failed").is_dir());
        assert!(observe_checkout(root, &root.join("state"), &repo("failed")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn network_failure_preserves_partial_and_original_error_without_retry() {
        use std::os::unix::fs::PermissionsExt;
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let git = root.join("fake-git");
        fs::write(&git, "#!/bin/sh\nfor last do :; done\nprintf data > \"$last/partial\"\nprintf 'Connection refused\\n' >&2\nexit 1\n").unwrap();
        fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).unwrap();
        let repo = CollectionRepo {
            name: "partial".into(),
            url: "local-source".into(),
            default_branch: "main".into(),
            description: String::new(),
        };
        let error = super::clone_repo_with_program(
            root,
            &root.join("state"),
            &repo,
            &CloneSettings::default(),
            3,
            git.as_os_str(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("Connection refused"));
        assert!(error.contains("Inspect"));
        assert_eq!(
            fs::read_to_string(root.join("partial/partial")).unwrap(),
            "data"
        );
        assert!(observe_checkout(root, &root.join("state"), &repo).is_err());
        assert!(
            clone_repo_with_retry(
                root,
                &root.join("state"),
                &repo,
                &CloneSettings::default(),
                3
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(root.join("partial/partial")).unwrap(),
            "data"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pending_record_blocks_even_a_valid_partial_and_prevents_launch_on_conflict() {
        use std::os::unix::fs::PermissionsExt;
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let git = root.join("fake-git");
        fs::write(&git, "#!/bin/sh\nfor last do :; done\ngit init --quiet \"$last\"\nprintf 'Connection refused\\n' >&2\nexit 1\n").unwrap();
        fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).unwrap();
        let repo = |name: &str| CollectionRepo {
            name: name.into(),
            url: "local-source".into(),
            default_branch: "main".into(),
            description: String::new(),
        };
        let partial = repo("partial-valid");
        assert!(
            super::clone_repo_with_program(
                root,
                &root.join("state"),
                &partial,
                &CloneSettings::default(),
                3,
                git.as_os_str()
            )
            .is_err()
        );
        assert!(root.join("partial-valid/.git").is_dir());
        assert!(observe_checkout(root, &root.join("state"), &partial).is_err());
        let interrupted = repo("interrupted");
        fs::write(
            super::pending_marker(root, &root.join("state"), &interrupted).unwrap(),
            "pending",
        )
        .unwrap();
        assert!(observe_checkout(root, &root.join("state"), &interrupted).is_err());
        let conflict = repo("conflict");
        fs::write(
            super::pending_marker(root, &root.join("state"), &conflict).unwrap(),
            "original",
        )
        .unwrap();
        assert!(
            super::clone_repo_with_program(
                root,
                &root.join("state"),
                &conflict,
                &CloneSettings::default(),
                3,
                git.as_os_str()
            )
            .is_err()
        );
        assert!(!root.join("conflict/.git").exists());
        assert_eq!(
            fs::read_to_string(
                super::pending_marker(root, &root.join("state"), &conflict).unwrap()
            )
            .unwrap(),
            "original"
        );
    }

    #[test]
    fn tracked_file_named_like_old_marker_does_not_block_checkout() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let source = root.join("source");
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(&source)
                .status()
                .unwrap()
                .success()
        );
        fs::write(source.join(".bossa-clone-incomplete"), "legitimate").unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&source)
                .args(["add", ".bossa-clone-incomplete"])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&source)
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "--quiet",
                    "-m",
                    "fixture"
                ])
                .status()
                .unwrap()
                .success()
        );
        clone_fixture(root, "checkout", &source).unwrap();
        let repo = CollectionRepo {
            name: "checkout".into(),
            url: source.to_string_lossy().into_owned(),
            default_branch: "main".into(),
            description: String::new(),
        };
        assert!(observe_checkout(root, &root.join("state"), &repo).unwrap());
        assert_eq!(
            fs::read_to_string(root.join("checkout/.bossa-clone-incomplete")).unwrap(),
            "legitimate"
        );
    }

    #[test]
    fn pending_records_do_not_collide_with_repository_names_or_long_names() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let source = root.join("source.git");
        assert!(
            Command::new("git")
                .args(["init", "--bare", "--quiet"])
                .arg(&source)
                .status()
                .unwrap()
                .success()
        );
        fs::create_dir(root.join(".bossa-clone-foo.pending")).unwrap();
        fs::write(root.join(".bossa-clone-foo.pending/user-data"), "keep").unwrap();
        clone_fixture(root, "foo", &source).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".bossa-clone-foo.pending/user-data")).unwrap(),
            "keep"
        );
        let long_name = "x".repeat(240);
        clone_fixture(root, &long_name, &source).unwrap();
        assert!(root.join(long_name).join(".git").is_dir());
    }

    #[test]
    fn refs_sync_without_name_maps_to_refs_collection() {
        let cli_cmd = CliRefsCommand::Sync(RefsSyncArgs {
            name: None,
            jobs: 8,
            retries: 5,
            dry_run: true,
        });

        let mapped: CollectionsCommand = cli_cmd.into();
        match mapped {
            CollectionsCommand::Sync {
                name,
                jobs,
                retries,
                dry_run,
            } => {
                assert_eq!(name, "refs");
                assert_eq!(jobs, 8);
                assert_eq!(retries, 5);
                assert!(dry_run);
            }
            _ => panic!("expected sync mapping"),
        }
    }

    #[test]
    fn refs_add_maps_into_refs_collection() {
        let cli_cmd = CliRefsCommand::Add {
            url: "https://github.com/example/repo.git".to_string(),
            name: Some("repo".to_string()),
            clone: true,
        };

        let mapped: CollectionsCommand = cli_cmd.into();
        match mapped {
            CollectionsCommand::Add {
                collection,
                url,
                name,
                clone,
            } => {
                assert_eq!(collection, "refs");
                assert_eq!(url, "https://github.com/example/repo.git");
                assert_eq!(name.as_deref(), Some("repo"));
                assert!(clone);
            }
            _ => panic!("expected add mapping"),
        }
    }

    #[test]
    fn collections_clean_mapping_preserves_fields() {
        let cli_cmd = CliCollectionsCommand::Clean {
            name: "my-collection".to_string(),
            yes: true,
            dry_run: false,
        };

        let mapped: CollectionsCommand = cli_cmd.into();
        match mapped {
            CollectionsCommand::Clean { name, yes, dry_run } => {
                assert_eq!(name, "my-collection");
                assert!(yes);
                assert!(!dry_run);
            }
            _ => panic!("expected clean mapping"),
        }
    }
}
