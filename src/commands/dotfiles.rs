//! Dotfiles command — manage the dotfiles repository lifecycle.
//!
//! Three sub-commands: `status`, `sync`, `diff`.
//! `sync` is idempotent: clone if missing, pull if clean, init submodules, run stow.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;
use std::process::Command;

use crate::Context as AppContext;
use crate::cli::{DotfilesCommand, DotfilesSyncArgs};
use crate::schema::{BossaConfig, DotfilesConfig};
use crate::state::BossaState;
use crate::ui;

// ============================================================================
// Result enums for internal phases
// ============================================================================

#[derive(Debug)]
enum PullResult {
    UpToDate,
    Updated(usize),
    Skipped(String),
}

#[derive(Debug)]
enum RepoState {
    NotCloned,
    Clean,
    Dirty,
}

#[derive(Debug)]
struct SubmoduleResult {
    name: String,
    ok: bool,
    msg: String,
}

#[derive(Debug)]
enum PrivateResult {
    Initialized,
    Skipped(String),
    Failed(String),
}

// ============================================================================
// Public entry points
// ============================================================================

pub fn run(ctx: &AppContext, cmd: DotfilesCommand) -> Result<()> {
    let config = BossaConfig::load()?;
    let dotfiles = config.dotfiles.as_ref().ok_or_else(|| {
        anyhow::anyhow!("No [dotfiles] section in config. Add it to ~/.config/bossa/config.toml")
    })?;
    dotfiles.validate()?;

    match cmd {
        DotfilesCommand::Status => status(ctx, dotfiles),
        DotfilesCommand::Sync(args) => sync(ctx, dotfiles, &config, args),
        DotfilesCommand::Diff => diff(ctx, dotfiles, &config),
    }
}

/// Simplified entry point for the nova bootstrap pipeline.
/// Runs sync with defaults (no dry-run, stow enabled, private enabled).
pub fn sync_for_nova(config: &BossaConfig) -> Result<()> {
    let dotfiles = match config.dotfiles.as_ref() {
        Some(d) => d,
        None => {
            ui::dim("No [dotfiles] config — skipping");
            return Ok(());
        }
    };
    if let Err(e) = dotfiles.validate() {
        ui::warn(&format!("Invalid dotfiles config: {e}"));
        return Ok(());
    }

    let ctx = AppContext {
        verbose: 0,
        quiet: true,
    };
    let args = DotfilesSyncArgs {
        dry_run: false,
        no_stow: false,
        no_private: false,
    };
    sync(&ctx, dotfiles, config, args)
}

// ============================================================================
// status
// ============================================================================

fn status(_ctx: &AppContext, config: &DotfilesConfig) -> Result<()> {
    let path = config.expanded_path()?;
    let state = BossaState::load()?;

    ui::header("Dotfiles");

    // -- Repository ----------------------------------------------------------
    if path.exists() {
        let branch = git_current_branch(&path).unwrap_or_else(|| "unknown".into());
        let remote = git_remote_url(&path).unwrap_or_else(|| "unknown".into());

        let ahead_behind = git_ahead_behind(&path, &config.branch);
        match ahead_behind {
            Some((0, 0)) => {
                println!(
                    "  {} {:<20} {} ({})",
                    "✓".green(),
                    "Repository",
                    path.display(),
                    branch
                );
                println!("  {:<23} {}", "", remote.dimmed());
                println!("  {:<23} {}", "", "Up to date with origin".dimmed());
            }
            Some((ahead, behind)) => {
                println!(
                    "  {} {:<20} {} ({})",
                    "⚠".yellow(),
                    "Repository",
                    path.display(),
                    branch
                );
                println!("  {:<23} {}", "", remote.dimmed());
                if behind > 0 {
                    println!(
                        "  {:<23} {}",
                        "",
                        format!("Behind origin/{} by {} commit(s)", config.branch, behind).yellow()
                    );
                }
                if ahead > 0 {
                    println!(
                        "  {:<23} {}",
                        "",
                        format!("Ahead of origin/{} by {} commit(s)", config.branch, ahead)
                            .yellow()
                    );
                }
            }
            None => {
                println!(
                    "  {} {:<20} {} ({})",
                    "✓".green(),
                    "Repository",
                    path.display(),
                    branch
                );
                println!("  {:<23} {}", "", remote.dimmed());
                println!(
                    "  {:<23} {}",
                    "",
                    "Could not determine remote status".dimmed()
                );
            }
        }
    } else {
        println!(
            "  {} {:<20} {} {}",
            "✗".red(),
            "Repository",
            config.path,
            "(not cloned)".red()
        );
        println!(
            "  {:<23} Run '{}' to set up",
            "",
            "bossa dotfiles sync".bold()
        );
    }

    // -- Submodules ----------------------------------------------------------
    println!();
    println!("  {}", "Submodules".bold());

    if path.exists() {
        for sub in &config.public_submodules {
            let sub_path = path.join(sub);
            let is_init = sub_path.exists()
                && sub_path
                    .read_dir()
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false);
            if is_init {
                println!("  {} {:<30} initialized", "✓".green(), sub);
            } else {
                println!("  {} {:<30} not initialized", "✗".red(), sub);
            }
        }

        if let Some(ref private) = config.private {
            let priv_path = path.join(&private.path);
            let is_init = priv_path.exists()
                && priv_path
                    .read_dir()
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false);
            let auth_ok = check_gh_auth();
            if is_init {
                let auth_label = if auth_ok {
                    "auth: ✓".green()
                } else {
                    "auth: ✗".red()
                };
                println!(
                    "  {} {:<30} initialized ({})",
                    "✓".green(),
                    private.path,
                    auth_label
                );
            } else if !auth_ok {
                println!("  {} {:<30} skipped (no auth)", "⊘".yellow(), private.path);
            } else {
                println!("  {} {:<30} not initialized", "✗".red(), private.path);
            }
        }

        for skip in &config.skip_submodules {
            println!("  {} {:<30} skipped", "⊘".dimmed(), skip);
        }
    } else {
        println!("  {} Repository not cloned yet", "–".dimmed());
    }

    // -- Symlinks via stow ---------------------------------------------------
    println!();
    println!("  {}", "Symlinks (via stow)".bold());

    if path.exists() {
        // Count symlinks from the config's symlinks section
        let home = dirs::home_dir().unwrap_or_default();
        let mut link_count: usize = 0;
        let mut pkg_status: Vec<(String, bool)> = Vec::new();

        // Load the BossaConfig to get symlinks info
        let full_config = BossaConfig::load()?;
        if let Some(ref symlinks) = full_config.symlinks {
            for pkg in &symlinks.packages {
                let pkg_dir = path.join(pkg);
                if pkg_dir.exists() {
                    let count = count_symlinks_in_dir(&pkg_dir, &home);
                    link_count += count;
                    pkg_status.push((pkg.clone(), count > 0));
                } else {
                    pkg_status.push((pkg.clone(), false));
                }
            }
        }

        if !pkg_status.is_empty() {
            let linked_pkgs = pkg_status.iter().filter(|(_, ok)| *ok).count();
            println!(
                "  {} {} packages linked ({} symlinks)",
                "✓".green(),
                linked_pkgs,
                link_count
            );
            let status_line: Vec<String> = pkg_status
                .iter()
                .map(|(name, ok)| {
                    if *ok {
                        format!("{} {}", name, "✓".green())
                    } else {
                        format!("{} {}", name, "?".yellow())
                    }
                })
                .collect();
            println!("    {}", status_line.join("  "));
        } else {
            println!("  {} No symlink packages configured", "–".dimmed());
        }
    } else {
        println!("  {} Repository not cloned yet", "–".dimmed());
    }

    // -- Last synced ---------------------------------------------------------
    println!();
    if let Some(ts) = state.dotfiles.last_sync {
        let ago = format_time_ago(ts);
        println!("  Last synced: {}", ago.dimmed());
    } else {
        println!("  Last synced: {}", "never".dimmed());
    }

    Ok(())
}

// ============================================================================
// sync
// ============================================================================

fn sync(
    ctx: &AppContext,
    config: &DotfilesConfig,
    full_config: &BossaConfig,
    args: DotfilesSyncArgs,
) -> Result<()> {
    let path = config.expanded_path()?;
    let mut state = BossaState::load()?;
    let total_steps = 4 + if args.no_stow { 0 } else { 1 };
    let mut step_num = 0;

    if args.dry_run && !ctx.quiet {
        ui::info("Dry run — no changes will be made");
        println!();
    }

    // Step 1: Clone ----------------------------------------------------------
    step_num += 1;
    if !ctx.quiet {
        ui::step(step_num, total_steps, "Repository");
    }

    if !path.exists() {
        if args.dry_run {
            println!(
                "  {} clone {} → {}",
                "would".yellow(),
                config.repo,
                path.display()
            );
        } else {
            clone_repo(config)?;
            state.mark_dotfiles_cloned();
        }
    } else if !ctx.quiet {
        ui::dim("Already cloned");
    }

    // Step 2: Pull -----------------------------------------------------------
    step_num += 1;
    if !ctx.quiet {
        ui::step(step_num, total_steps, "Pull");
    }

    if path.exists() && !args.dry_run {
        match pull_repo(&path, &config.branch) {
            Ok(PullResult::UpToDate) => {
                if !ctx.quiet {
                    ui::dim("Already up to date");
                }
            }
            Ok(PullResult::Updated(n)) => {
                if !ctx.quiet {
                    ui::success(&format!("Pulled {} new commit(s)", n));
                }
            }
            Ok(PullResult::Skipped(reason)) => {
                if !ctx.quiet {
                    ui::warn(&format!("Pull skipped: {}", reason));
                }
            }
            Err(e) => {
                ui::warn(&format!("Pull failed: {} — continuing", e));
            }
        }
    } else if path.exists() && args.dry_run {
        match check_repo_state(&path) {
            Ok(RepoState::Clean) => {
                let behind = git_ahead_behind(&path, &config.branch)
                    .map(|(_, b)| b)
                    .unwrap_or(0);
                if behind > 0 {
                    println!(
                        "  {} pull {} commit(s) from origin/{}",
                        "would".yellow(),
                        behind,
                        config.branch
                    );
                } else {
                    ui::dim("Already up to date");
                }
            }
            Ok(RepoState::Dirty) => {
                println!("  {} skip pull (uncommitted changes)", "would".yellow());
            }
            _ => {}
        }
    }

    // Step 3: Public submodules ----------------------------------------------
    step_num += 1;
    if !ctx.quiet {
        ui::step(step_num, total_steps, "Submodules");
    }

    if path.exists() {
        if args.dry_run {
            for sub in &config.public_submodules {
                if config.skip_submodules.contains(sub) {
                    continue;
                }
                let sub_path = path.join(sub);
                let is_init = sub_path.exists()
                    && sub_path
                        .read_dir()
                        .map(|mut d| d.next().is_some())
                        .unwrap_or(false);
                if !is_init {
                    println!("  {} init submodule {}", "would".yellow(), sub);
                }
            }
        } else {
            let results =
                init_submodules(&path, &config.public_submodules, &config.skip_submodules)?;
            for r in &results {
                if r.ok {
                    state.mark_submodule_initialized(&r.name);
                    if !ctx.quiet {
                        ui::dim(&format!("{}: {}", r.name, r.msg));
                    }
                } else {
                    ui::warn(&format!("{}: {}", r.name, r.msg));
                }
            }
        }
    }

    // Step 4: Private submodule ----------------------------------------------
    step_num += 1;
    if !ctx.quiet {
        ui::step(step_num, total_steps, "Private submodule");
    }

    if path.exists() && !args.no_private {
        if let Some(ref private) = config.private {
            if args.dry_run {
                let priv_path = path.join(&private.path);
                let is_init = priv_path.exists()
                    && priv_path
                        .read_dir()
                        .map(|mut d| d.next().is_some())
                        .unwrap_or(false);
                if !is_init {
                    if check_gh_auth() {
                        println!(
                            "  {} init private submodule {}",
                            "would".yellow(),
                            private.path
                        );
                    } else {
                        println!("  {} skip private (no gh auth)", "would".yellow());
                    }
                } else if !ctx.quiet {
                    ui::dim("Already initialized");
                }
            } else {
                match init_private(&path, private) {
                    Ok(PrivateResult::Initialized) => {
                        state.mark_private_initialized();
                        if !ctx.quiet {
                            ui::success("Private submodule initialized");
                        }
                    }
                    Ok(PrivateResult::Skipped(reason)) => {
                        if !ctx.quiet {
                            ui::dim(&format!("Private: {}", reason));
                        }
                    }
                    Ok(PrivateResult::Failed(reason)) => {
                        ui::warn(&format!("Private submodule: {}", reason));
                    }
                    Err(e) => {
                        ui::warn(&format!("Private submodule error: {} — continuing", e));
                    }
                }
            }
        } else if !ctx.quiet {
            ui::dim("No private submodule configured");
        }
    } else if args.no_private && !ctx.quiet {
        ui::dim("Skipped (--no-private)");
    }

    // Step 5: Stow -----------------------------------------------------------
    if !args.no_stow {
        step_num += 1;
        if !ctx.quiet {
            ui::step(step_num, total_steps, "Symlinks");
        }

        if path.exists() {
            if args.dry_run {
                if let Some(ref symlinks) = full_config.symlinks {
                    let missing = count_missing_symlinks(&path, symlinks);
                    if missing > 0 {
                        println!(
                            "  {} create/update {} symlink(s)",
                            "would".yellow(),
                            missing
                        );
                    } else if !ctx.quiet {
                        ui::dim("All symlinks up to date");
                    }
                } else if !ctx.quiet {
                    ui::dim("No [symlinks] config");
                }
            } else {
                // Delegate to nova's symlink logic by re-running the stow stage
                run_stow_sync(full_config, ctx)?;
            }
        }
    }

    // Finalize ---------------------------------------------------------------
    if !args.dry_run {
        state.mark_dotfiles_synced();
        state.touch()?;
    }

    if !ctx.quiet && !args.dry_run {
        println!();
        ui::success("Dotfiles sync complete");
    }

    Ok(())
}

// ============================================================================
// diff (dry-run wrapper)
// ============================================================================

fn diff(ctx: &AppContext, config: &DotfilesConfig, full_config: &BossaConfig) -> Result<()> {
    let args = DotfilesSyncArgs {
        dry_run: true,
        no_stow: false,
        no_private: false,
    };
    sync(ctx, config, full_config, args)
}

// ============================================================================
// Internal helpers — git operations
// ============================================================================

fn clone_repo(config: &DotfilesConfig) -> Result<()> {
    let path = config.expanded_path()?;

    ui::info(&format!("Cloning {} → {}", config.repo, path.display()));

    let output = Command::new("git")
        .args(["clone", "--branch", &config.branch, &config.repo])
        .arg(&path)
        .output()
        .context("Failed to run git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git clone failed: {}", stderr.trim());
    }

    ui::success("Repository cloned");
    Ok(())
}

fn pull_repo(path: &Path, branch: &str) -> Result<PullResult> {
    match check_repo_state(path)? {
        RepoState::NotCloned => return Ok(PullResult::Skipped("not cloned".into())),
        RepoState::Dirty => return Ok(PullResult::Skipped("uncommitted changes".into())),
        RepoState::Clean => {}
    }

    // Fetch first
    let fetch = Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(path)
        .output()
        .context("Failed to run git fetch")?;

    if !fetch.status.success() {
        return Ok(PullResult::Skipped("fetch failed (network?)".into()));
    }

    // Count commits behind
    let behind = git_ahead_behind(path, branch).map(|(_, b)| b).unwrap_or(0);

    if behind == 0 {
        return Ok(PullResult::UpToDate);
    }

    // Fast-forward pull
    let pull = Command::new("git")
        .args(["pull", "--ff-only", "origin", branch])
        .current_dir(path)
        .output()
        .context("Failed to run git pull")?;

    if !pull.status.success() {
        let stderr = String::from_utf8_lossy(&pull.stderr);
        return Ok(PullResult::Skipped(format!(
            "pull --ff-only failed: {}",
            stderr.trim()
        )));
    }

    Ok(PullResult::Updated(behind))
}

fn check_repo_state(path: &Path) -> Result<RepoState> {
    if !path.exists() {
        return Ok(RepoState::NotCloned);
    }

    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(path)
        .output()
        .context("Failed to run git status")?;

    if !output.status.success() {
        return Ok(RepoState::NotCloned);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        Ok(RepoState::Clean)
    } else {
        Ok(RepoState::Dirty)
    }
}

fn init_submodules(
    path: &Path,
    public: &[String],
    skip: &[String],
) -> Result<Vec<SubmoduleResult>> {
    let mut results = Vec::new();

    for sub in public {
        if skip.contains(sub) {
            results.push(SubmoduleResult {
                name: sub.clone(),
                ok: true,
                msg: "skipped".into(),
            });
            continue;
        }

        let sub_path = path.join(sub);
        let is_init = sub_path.exists()
            && sub_path
                .read_dir()
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);

        if is_init {
            results.push(SubmoduleResult {
                name: sub.clone(),
                ok: true,
                msg: "already initialized".into(),
            });
            continue;
        }

        let output = Command::new("git")
            .args(["submodule", "update", "--init", "--", sub])
            .current_dir(path)
            .output()
            .context("Failed to run git submodule update")?;

        if output.status.success() {
            results.push(SubmoduleResult {
                name: sub.clone(),
                ok: true,
                msg: "initialized".into(),
            });
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            results.push(SubmoduleResult {
                name: sub.clone(),
                ok: false,
                msg: format!("failed: {}", stderr.trim()),
            });
        }
    }

    Ok(results)
}

fn init_private(
    path: &Path,
    private: &crate::schema::DotfilesPrivateConfig,
) -> Result<PrivateResult> {
    // Check if already initialized
    let priv_path = path.join(&private.path);
    let is_init = priv_path.exists()
        && priv_path
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);

    if is_init {
        return Ok(PrivateResult::Skipped("already initialized".into()));
    }

    // Check auth if required
    if private.requires_auth && !check_gh_auth() {
        return Ok(PrivateResult::Skipped("gh not authenticated".into()));
    }

    // Init the submodule
    let output = Command::new("git")
        .args(["submodule", "update", "--init", "--", &private.path])
        .current_dir(path)
        .output()
        .context("Failed to run git submodule update for private")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Ok(PrivateResult::Failed(stderr.trim().to_string()));
    }

    // Run setup script if present
    if let Some(ref script) = private.setup_script {
        let script_path = priv_path.join(script);
        if script_path.exists() {
            let script_output = Command::new("bash")
                .arg(&script_path)
                .current_dir(&priv_path)
                .output()
                .context("Failed to run private setup script")?;

            if !script_output.status.success() {
                let stderr = String::from_utf8_lossy(&script_output.stderr);
                ui::warn(&format!("Setup script warning: {}", stderr.trim()));
            }
        }
    }

    Ok(PrivateResult::Initialized)
}

fn check_gh_auth() -> bool {
    Command::new("gh")
        .args(["auth", "status"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn git_current_branch(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn git_remote_url(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(path)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn git_ahead_behind(path: &Path, branch: &str) -> Option<(usize, usize)> {
    let output = Command::new("git")
        .args([
            "rev-list",
            "--left-right",
            "--count",
            &format!("HEAD...origin/{}", branch),
        ])
        .current_dir(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = stdout.split_whitespace().collect();
    if parts.len() == 2 {
        let ahead = parts[0].parse().ok()?;
        let behind = parts[1].parse().ok()?;
        Some((ahead, behind))
    } else {
        None
    }
}

// ============================================================================
// Internal helpers — stow / symlinks
// ============================================================================

fn run_stow_sync(config: &BossaConfig, ctx: &AppContext) -> Result<()> {
    let symlinks = match &config.symlinks {
        Some(s) => s,
        None => {
            if !ctx.quiet {
                ui::dim("No [symlinks] config — skipping");
            }
            return Ok(());
        }
    };

    if symlinks.source.is_empty() || symlinks.packages.is_empty() {
        return Ok(());
    }

    let source_base = shellexpand::tilde(&symlinks.source).to_string();
    let target_base = shellexpand::tilde(&symlinks.target).to_string();

    let mut created = 0usize;
    let mut existed = 0usize;

    for package in &symlinks.packages {
        let package_source = std::path::Path::new(&source_base).join(package);
        if package_source.exists() {
            let (c, e) = create_symlinks_recursive(
                &package_source,
                &package_source,
                Path::new(&target_base),
                &symlinks.ignore,
            )?;
            created += c;
            existed += e;
        }
    }

    if !ctx.quiet {
        if created > 0 {
            ui::success(&format!(
                "Created {} new symlink(s), {} already existed",
                created, existed
            ));
        } else {
            ui::dim(&format!("All {} symlinks up to date", existed));
        }
    }

    Ok(())
}

fn create_symlinks_recursive(
    base: &Path,
    current: &Path,
    target_base: &Path,
    ignore: &[String],
) -> Result<(usize, usize)> {
    let mut created = 0usize;
    let mut existed = 0usize;

    if !current.is_dir() {
        return Ok((0, 0));
    }

    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();

        if ignore.iter().any(|p| file_name == *p || path.ends_with(p)) {
            continue;
        }

        let relative = path.strip_prefix(base)?;
        let target = target_base.join(relative);

        if path.is_file() || (path.is_symlink() && !path.is_dir()) {
            if target.is_symlink() {
                // Check if it points to the right place
                if let Ok(link_target) = std::fs::read_link(&target) {
                    if link_target == path {
                        existed += 1;
                        continue;
                    }
                }
                // Remove wrong symlink
                std::fs::remove_file(&target)?;
            }

            if !target.exists() {
                // Ensure parent directory exists
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::os::unix::fs::symlink(&path, &target).with_context(|| {
                    format!(
                        "Failed to create symlink: {} → {}",
                        target.display(),
                        path.display()
                    )
                })?;
                created += 1;
            } else {
                existed += 1;
            }
        } else if path.is_dir() {
            let (c, e) = create_symlinks_recursive(base, &path, target_base, ignore)?;
            created += c;
            existed += e;
        }
    }

    Ok((created, existed))
}

fn count_symlinks_in_dir(dir: &Path, target_base: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() || path.is_symlink() {
                // Check if corresponding symlink exists in target
                if let Ok(relative) = path.strip_prefix(dir.parent().unwrap_or(dir)) {
                    let target = target_base.join(relative);
                    if target.is_symlink() {
                        count += 1;
                    }
                }
            } else if path.is_dir() {
                count += count_symlinks_in_dir(&path, target_base);
            }
        }
    }
    count
}

fn count_missing_symlinks(dotfiles_path: &Path, symlinks: &crate::schema::SymlinksConfig) -> usize {
    let source_base = shellexpand::tilde(&symlinks.source).to_string();
    let target_base = shellexpand::tilde(&symlinks.target).to_string();
    let mut missing = 0;

    // Only count if source actually matches the dotfiles path
    let source_path = Path::new(&source_base);
    if !source_path.exists() && dotfiles_path.exists() {
        // Source doesn't exist yet — all symlinks would be new
        return symlinks.packages.len();
    }

    for package in &symlinks.packages {
        let pkg_dir = Path::new(&source_base).join(package);
        if pkg_dir.exists() {
            missing += count_missing_in_dir(
                &pkg_dir,
                &pkg_dir,
                Path::new(&target_base),
                &symlinks.ignore,
            );
        }
    }

    missing
}

fn count_missing_in_dir(
    base: &Path,
    current: &Path,
    target_base: &Path,
    ignore: &[String],
) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(current) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if ignore.iter().any(|p| file_name == *p || path.ends_with(p)) {
                continue;
            }

            if let Ok(relative) = path.strip_prefix(base) {
                let target = target_base.join(relative);
                if path.is_file() || (path.is_symlink() && !path.is_dir()) {
                    if !target.is_symlink() {
                        count += 1;
                    }
                } else if path.is_dir() {
                    count += count_missing_in_dir(base, &path, target_base, ignore);
                }
            }
        }
    }
    count
}

// ============================================================================
// Formatting helpers
// ============================================================================

fn format_time_ago(ts: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(ts);

    if duration.num_days() > 0 {
        let days = duration.num_days();
        if days == 1 {
            "1 day ago".to_string()
        } else {
            format!("{} days ago", days)
        }
    } else if duration.num_hours() > 0 {
        let hours = duration.num_hours();
        if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", hours)
        }
    } else if duration.num_minutes() > 0 {
        let mins = duration.num_minutes();
        if mins == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{} minutes ago", mins)
        }
    } else {
        "just now".to_string()
    }
}
