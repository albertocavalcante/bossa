use anyhow::{Context as AnyhowContext, Result};
use colored::Colorize;
use rayon::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::cli::{RefsCommand, RefsSyncArgs};
use crate::config::{self, RefsConfig, RefsRepo};
use crate::progress;
use crate::runner;
use crate::ui;
use crate::Context;

pub fn run(ctx: &Context, cmd: RefsCommand) -> Result<()> {
    match cmd {
        RefsCommand::Sync(args) => sync(ctx, args),
        RefsCommand::List { filter, missing } => list(ctx, filter, missing),
        RefsCommand::Snapshot => snapshot(ctx),
        RefsCommand::Audit { fix } => audit(ctx, fix),
        RefsCommand::Add { url, name, clone } => add(ctx, &url, name, clone),
        RefsCommand::Remove { name, delete } => remove(ctx, &name, delete),
    }
}

// ============================================================================
// Native Sync with Parallelism and Retry
// ============================================================================

fn sync(ctx: &Context, args: RefsSyncArgs) -> Result<()> {
    ui::header("Syncing Reference Repos");

    let config = RefsConfig::load()?;
    let root = config.root_path()?;

    // Ensure root directory exists
    fs::create_dir_all(&root)?;

    // Filter repos to clone
    let repos_to_clone: Vec<_> = if let Some(name) = &args.name {
        config
            .repositories
            .iter()
            .filter(|r| r.name == *name)
            .cloned()
            .collect()
    } else {
        config
            .repositories
            .iter()
            .filter(|r| !root.join(&r.name).exists())
            .cloned()
            .collect()
    };

    if repos_to_clone.is_empty() {
        if args.name.is_some() {
            ui::warn("Repository already exists or not found in config");
        } else {
            ui::success("All repositories already cloned!");
        }
        return Ok(());
    }

    ui::kv("Root", &root.display().to_string());
    ui::kv("To clone", &repos_to_clone.len().to_string());
    ui::kv("Parallel jobs", &args.jobs.to_string());
    ui::kv("Retries", &args.retries.to_string());
    println!();

    if args.dry_run {
        ui::warn("Dry run - no changes will be made");
        for repo in &repos_to_clone {
            println!("  {} {}", "→".cyan(), repo.name);
        }
        return Ok(());
    }

    // Clone in parallel with progress bar
    let pb = progress::clone_bar(repos_to_clone.len() as u64);
    let cloned = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let failed_repos: Arc<std::sync::Mutex<Vec<(String, String)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    // Configure rayon thread pool
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(args.jobs)
        .build()
        .unwrap();

    pool.install(|| {
        repos_to_clone.par_iter().for_each(|repo| {
            let result = clone_with_retry(&root, repo, args.retries);

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
        ui::success(&format!("Cloned {} repositories successfully!", cloned_count));
    } else {
        ui::warn(&format!(
            "Cloned {}, {} failed",
            cloned_count, failed_count
        ));

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
fn clone_with_retry(root: &PathBuf, repo: &RefsRepo, max_retries: usize) -> Result<()> {
    let repo_path = root.join(&repo.name);

    for attempt in 1..=max_retries {
        let result = clone_repo(root, repo);

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
fn clone_repo(root: &PathBuf, repo: &RefsRepo) -> Result<()> {
    let repo_path = root.join(&repo.name);

    // Run git clone
    let output = std::process::Command::new("git")
        .args(["clone", "--depth", "1", &repo.url, repo_path.to_str().unwrap()])
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
// List
// ============================================================================

fn list(_ctx: &Context, filter: Option<String>, only_missing: bool) -> Result<()> {
    ui::header("Reference Repositories");

    let config = RefsConfig::load()?;
    let root = config.root_path()?;

    let repos: Vec<_> = config
        .repositories
        .iter()
        .filter(|r| {
            // Filter by name pattern if specified
            if let Some(ref f) = filter {
                if !r.name.to_lowercase().contains(&f.to_lowercase()) {
                    return false;
                }
            }
            // Filter to only missing if specified
            if only_missing && root.join(&r.name).exists() {
                return false;
            }
            true
        })
        .collect();

    ui::kv("Root", &root.display().to_string());
    ui::kv("Showing", &repos.len().to_string());
    if filter.is_some() || only_missing {
        ui::kv(
            "Total",
            &config.repositories.len().to_string(),
        );
    }
    println!();

    for repo in &repos {
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
// Snapshot
// ============================================================================

fn snapshot(_ctx: &Context) -> Result<()> {
    ui::header("Capturing Refs Snapshot");

    let pb = progress::spinner("Scanning ~/dev/refs...");

    let refs_dir = dirs::home_dir()
        .context("Could not determine home directory")?
        .join("dev")
        .join("refs");

    // Handle symlink
    let refs_dir = if refs_dir.is_symlink() {
        refs_dir.read_link()?
    } else {
        refs_dir
    };

    if !refs_dir.exists() {
        progress::finish_error(&pb, "Refs directory does not exist");
        return Ok(());
    }

    let mut repositories = Vec::new();

    for entry in fs::read_dir(&refs_dir)? {
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
            &["-C", path.to_str().unwrap(), "config", "--get", "remote.origin.url"],
        ) {
            Ok(u) => u,
            Err(_) => continue,
        };

        // Get default branch
        let default_branch = runner::run_capture(
            "git",
            &["-C", path.to_str().unwrap(), "symbolic-ref", "refs/remotes/origin/HEAD"],
        )
        .map(|s| s.split('/').last().unwrap_or("main").to_string())
        .unwrap_or_else(|_| "main".to_string());

        repositories.push(RefsRepo {
            name,
            url,
            default_branch,
        });
    }

    repositories.sort_by(|a, b| a.name.cmp(&b.name));

    let config = RefsConfig {
        root_directory: "~/dev/refs".to_string(),
        repositories,
    };

    let count = config.repositories.len();
    config.save()?;

    progress::finish_success(&pb, &format!("Captured {} repositories", count));

    let config_path = config::config_dir()?.join("refs.json");
    ui::dim(&format!("Saved to: {}", config_path.display()));

    Ok(())
}

// ============================================================================
// Audit
// ============================================================================

fn audit(_ctx: &Context, fix: bool) -> Result<()> {
    ui::header("Refs Audit - Drift Detection");

    let config = RefsConfig::load()?;
    let root = config.root_path()?;

    if !root.exists() {
        ui::warn(&format!("Refs directory does not exist: {}", root.display()));
        return Ok(());
    }

    let pb = progress::spinner("Scanning for untracked repos...");

    // Get all directories in refs
    let entries: Vec<_> = fs::read_dir(&root)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| e.path().join(".git").exists())
        .collect();

    // Find repos not in config
    let tracked_names: std::collections::HashSet<_> =
        config.repositories.iter().map(|r| r.name.clone()).collect();

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

    ui::kv("Tracked", &format!("{} repos", tracked_count));
    ui::kv("Untracked", &format!("{} repos", untracked.len()));

    if untracked.is_empty() {
        println!();
        ui::success("No drift detected - all repos are tracked in refs.json");
        return Ok(());
    }

    println!();
    ui::warn("Untracked repos (exist locally but not in refs.json):");

    for (name, _) in &untracked {
        println!("  {} {}", "?".yellow(), name);
    }

    if fix {
        println!();
        ui::info("Adding untracked repos to config...");

        let mut config = config;

        for (name, path) in &untracked {
            // Get remote URL
            let url = runner::run_capture(
                "git",
                &["-C", path.to_str().unwrap(), "config", "--get", "remote.origin.url"],
            )?;

            // Get default branch
            let default_branch = runner::run_capture(
                "git",
                &["-C", path.to_str().unwrap(), "symbolic-ref", "refs/remotes/origin/HEAD"],
            )
            .map(|s| s.split('/').last().unwrap_or("main").to_string())
            .unwrap_or_else(|_| "main".to_string());

            config.add_repo(RefsRepo {
                name: name.clone(),
                url,
                default_branch,
            });

            println!("  {} {}", "✓".green(), name);
        }

        config.save()?;
        ui::success(&format!("Added {} repos to refs.json", untracked.len()));
    } else {
        println!();
        ui::info("Run 'bossa refs audit --fix' to add these repos to config");
        ui::info("Or 'bossa refs snapshot' to regenerate the entire config");
    }

    Ok(())
}

// ============================================================================
// Add
// ============================================================================

fn add(ctx: &Context, url: &str, name: Option<String>, clone: bool) -> Result<()> {
    let repo_name = name
        .or_else(|| config::repo_name_from_url(url))
        .context("Could not determine repo name from URL. Use --name to specify.")?;

    ui::header(&format!("Adding Repo: {}", repo_name));

    // Detect default branch with spinner
    let pb = progress::spinner("Detecting default branch...");
    let default_branch = config::detect_default_branch(url);
    progress::finish_success(&pb, &format!("Default branch: {}", default_branch));

    // Load or create config
    let mut config = RefsConfig::load().unwrap_or_else(|_| RefsConfig {
        root_directory: "~/dev/refs".to_string(),
        repositories: Vec::new(),
    });

    // Check if already exists
    if config.find_repo(&repo_name).is_some() {
        ui::warn(&format!("Repo '{}' already exists in config", repo_name));
    }

    // Add repo
    config.add_repo(RefsRepo {
        name: repo_name.clone(),
        url: url.to_string(),
        default_branch,
    });

    // Save config
    config.save()?;
    ui::success(&format!("Added '{}' to refs.json", repo_name));

    // Clone if requested
    if clone {
        println!();
        let pb = progress::spinner(&format!("Cloning {}...", repo_name));

        let root = config.root_path()?;
        fs::create_dir_all(&root)?;

        let repo = config.find_repo(&repo_name).unwrap().clone();
        match clone_with_retry(&root, &repo, 3) {
            Ok(()) => {
                progress::finish_success(&pb, &format!("Cloned '{}'", repo_name));
            }
            Err(e) => {
                progress::finish_error(&pb, &format!("Failed to clone: {}", e));
                if !ctx.quiet {
                    ui::info("You can retry later with: bossa refs sync");
                }
            }
        }
    }

    Ok(())
}

// ============================================================================
// Remove
// ============================================================================

fn remove(_ctx: &Context, name: &str, delete: bool) -> Result<()> {
    ui::header(&format!("Removing Repo: {}", name));

    let mut config = RefsConfig::load()?;

    if !config.remove_repo(name) {
        ui::warn(&format!("Repo '{}' not found in config", name));
        return Ok(());
    }

    config.save()?;
    ui::success(&format!("Removed '{}' from refs.json", name));

    if delete {
        let root = config.root_path()?;
        let repo_path = root.join(name);

        if repo_path.exists() {
            let pb = progress::spinner(&format!("Deleting {}...", repo_path.display()));
            fs::remove_dir_all(&repo_path)?;
            progress::finish_success(&pb, "Local clone deleted");
        }
    }

    Ok(())
}
