//! Stow command - native dotfile symlink management
//!
//! This module provides a native Rust replacement for GNU stow, designed
//! specifically for dotfile management.

use anyhow::{Context, Result, bail};
use chrono::Utc;
use colored::Colorize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::Context as AppContext;
use crate::cli::StowCommand;
use crate::schema::{BossaConfig, SymlinksConfig};
use crate::state::{BossaState, TrackedSymlink};

/// Symlink state for reporting
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum SymlinkState {
    /// Symlink exists and points to correct target
    Correct,
    /// Symlink doesn't exist
    Missing,
    /// Symlink exists but points to wrong target
    Wrong { actual: PathBuf, expected: PathBuf },
    /// Regular file/directory exists (blocking)
    Blocked,
}

/// A symlink operation to perform
#[derive(Debug, Clone)]
struct SymlinkOp {
    source: PathBuf,
    target: PathBuf,
    state: SymlinkState,
    package: String,
}

/// A package whose source directory is missing
#[derive(Debug, Clone)]
struct MissingPackage {
    name: String,
    expected_path: PathBuf,
}

/// A package whose source directory exists but contains no files
#[derive(Debug, Clone)]
struct EmptyPackage {
    name: String,
    path: PathBuf,
}

/// Result of collecting symlink operations
#[derive(Debug, Default)]
struct CollectResult {
    ops: Vec<SymlinkOp>,
    missing_packages: Vec<MissingPackage>,
    empty_packages: Vec<EmptyPackage>,
}

pub fn run(_ctx: &AppContext, cmd: StowCommand) -> Result<()> {
    match cmd {
        StowCommand::Status => status(),
        StowCommand::Sync {
            packages,
            dry_run,
            force,
        } => sync(&packages, dry_run, force),
        StowCommand::Diff { packages } => diff(&packages),
        StowCommand::List => list(),
        StowCommand::Add { package } => add(&package),
        StowCommand::Rm { package, unlink } => rm(&package, unlink),
        StowCommand::Unlink { packages, dry_run } => unlink(&packages, dry_run),
        StowCommand::Init {
            source,
            target,
            force,
        } => init(source.as_deref(), target.as_deref(), force),
    }
}

/// Package status summary
#[derive(Debug, Default, Clone)]
struct PackageStatus {
    linked: usize,
    unlinked: usize,
    wrong: usize,
    blocked: usize,
}

impl PackageStatus {
    fn total(&self) -> usize {
        self.linked + self.unlinked + self.wrong + self.blocked
    }

    fn status_label(&self) -> (String, colored::Color) {
        if self.total() == 0 {
            ("empty".to_string(), colored::Color::BrightBlack)
        } else if self.unlinked == 0 && self.wrong == 0 && self.blocked == 0 {
            ("linked".to_string(), colored::Color::Green)
        } else if self.linked == 0 {
            ("unlinked".to_string(), colored::Color::BrightBlack)
        } else {
            ("partial".to_string(), colored::Color::Yellow)
        }
    }
}

/// Show status of all symlinks in a table format
fn status() -> Result<()> {
    let config = BossaConfig::load()?;
    let symlinks = get_symlinks_config(&config)?;

    let source_base = expand_path(&symlinks.source);

    // Collect stats per package
    let mut package_stats: HashMap<String, PackageStatus> = HashMap::new();

    // Initialize all packages (even if directory doesn't exist)
    for pkg in &symlinks.packages {
        package_stats.insert(pkg.clone(), PackageStatus::default());
    }

    // Collect operations and aggregate stats
    let CollectResult { ops, .. } = collect_symlink_ops(symlinks)?;
    for op in &ops {
        let stats = package_stats.entry(op.package.clone()).or_default();
        match &op.state {
            SymlinkState::Correct => stats.linked += 1,
            SymlinkState::Missing => stats.unlinked += 1,
            SymlinkState::Wrong { .. } => stats.wrong += 1,
            SymlinkState::Blocked => stats.blocked += 1,
        }
    }

    // Print header
    println!();
    println!(
        "  {:<20} {:<12} {:>6}",
        "Package".bold(),
        "Status".bold(),
        "Files".bold()
    );
    println!("  {} {} {}", "─".repeat(20), "─".repeat(12), "─".repeat(6));

    // Sort packages alphabetically
    let mut packages: Vec<_> = symlinks.packages.iter().collect();
    packages.sort();

    // Print each package
    let mut total_linked = 0;
    let mut total_unlinked = 0;

    for pkg in packages {
        let stats = package_stats.get(pkg).cloned().unwrap_or_default();
        let (label, color) = stats.status_label();
        let exists = source_base.join(pkg).exists();

        let status_icon = match label.as_str() {
            "linked" => "✓".green(),
            "partial" => "◐".yellow(),
            "unlinked" => "○".bright_black(),
            _ => "?".bright_black(),
        };

        let status_text = label.color(color);
        let files = if exists {
            stats.total().to_string()
        } else {
            "-".to_string()
        };

        println!(
            "  {:<20} {} {:<10} {:>6}",
            pkg,
            status_icon,
            status_text,
            files.bright_black()
        );

        total_linked += stats.linked;
        total_unlinked += stats.unlinked + stats.wrong + stats.blocked;
    }

    println!();

    // Summary line
    if total_unlinked > 0 {
        println!(
            "  {} linked, {} to sync",
            total_linked.to_string().green(),
            total_unlinked.to_string().yellow()
        );
        println!();
        println!("  Run {} to sync", "bossa stow sync".cyan());
    } else if total_linked > 0 {
        println!(
            "  {} All {} symlinks are in place",
            "✓".green(),
            total_linked
        );
    } else {
        println!(
            "  {} No symlinks configured or source missing",
            "○".bright_black()
        );
    }

    println!();

    Ok(())
}

/// Sync (create/update) symlinks
fn sync(packages: &[String], dry_run: bool, force: bool) -> Result<()> {
    let config = BossaConfig::load()?;
    let symlinks = get_symlinks_config(&config)?;

    // Filter packages if specified
    let packages_to_sync: Vec<_> = if packages.is_empty() {
        symlinks.packages.clone()
    } else {
        // Validate specified packages exist in config
        for p in packages {
            if !symlinks.packages.contains(p) {
                bail!(
                    "Package '{}' not in config. Available: {}",
                    p,
                    symlinks.packages.join(", ")
                );
            }
        }
        packages.to_vec()
    };

    let filtered_config = SymlinksConfig {
        source: symlinks.source.clone(),
        target: symlinks.target.clone(),
        packages: packages_to_sync,
        ignore: symlinks.ignore.clone(),
    };

    let CollectResult {
        ops,
        missing_packages,
        empty_packages,
    } = collect_symlink_ops(&filtered_config)?;

    // Count what needs to be done
    let to_create: Vec<_> = ops
        .iter()
        .filter(|op| matches!(op.state, SymlinkState::Missing))
        .collect();
    let to_fix: Vec<_> = ops
        .iter()
        .filter(|op| matches!(op.state, SymlinkState::Wrong { .. }))
        .collect();
    let blocked: Vec<_> = ops
        .iter()
        .filter(|op| matches!(op.state, SymlinkState::Blocked))
        .collect();

    // Handle missing package directories
    if !missing_packages.is_empty() {
        if dry_run {
            print_missing_packages(&missing_packages);
        } else {
            create_missing_package_dirs(&missing_packages);
        }
    }

    // Show hints for empty source directories
    print_empty_packages(&empty_packages);

    if to_create.is_empty() && to_fix.is_empty() && (blocked.is_empty() || !force) {
        // Collect correct ops grouped by package
        let correct: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op.state, SymlinkState::Correct))
            .collect();

        let has_real_packages =
            filtered_config.packages.len() > missing_packages.len() + empty_packages.len();

        if has_real_packages && !correct.is_empty() {
            // Group by package
            let mut by_package: std::collections::BTreeMap<&str, Vec<&SymlinkOp>> =
                std::collections::BTreeMap::new();
            for op in &correct {
                by_package.entry(&op.package).or_default().push(op);
            }

            let target_base = expand_path(&symlinks.target);
            let pkg_count = by_package.len();

            println!(
                "{}",
                format!(
                    "✓ All symlinks are up to date ({} links across {} packages)",
                    correct.len(),
                    pkg_count
                )
                .green()
            );
            println!();

            for (pkg_name, mut pkg_ops) in by_package {
                // Sort ops by relative target path for stable output
                pkg_ops.sort_by(|a, b| a.target.cmp(&b.target));

                println!(
                    "  {} ({} {})",
                    pkg_name.bold(),
                    pkg_ops.len(),
                    if pkg_ops.len() == 1 { "link" } else { "links" }
                );

                // Find max target width for alignment
                let rel_targets: Vec<_> = pkg_ops
                    .iter()
                    .map(|op| {
                        op.target
                            .strip_prefix(&target_base)
                            .unwrap_or(&op.target)
                            .display()
                            .to_string()
                    })
                    .collect();
                let max_width = rel_targets.iter().map(String::len).max().unwrap_or(0);

                for (op, rel) in pkg_ops.iter().zip(&rel_targets) {
                    println!(
                        "    {:<width$} → {}",
                        rel,
                        contract_home(&op.source),
                        width = max_width
                    );
                }
                println!();
            }
        }

        // Still show blocked files warning if any
        if !blocked.is_empty() {
            println!(
                "{} {} files blocked (use --force to overwrite)",
                "⚠".yellow(),
                blocked.len()
            );
            let target_base = expand_path(&symlinks.target);
            for op in &blocked {
                let rel_target = op.target.strip_prefix(&target_base).unwrap_or(&op.target);
                println!("  {} {}", "⊘".red(), rel_target.display());
            }
        }

        return Ok(());
    }

    let mode = if dry_run {
        "Would sync".yellow()
    } else {
        "Syncing".green()
    };

    println!("{} {} symlinks...", mode, to_create.len() + to_fix.len());
    println!();

    // Load state for tracking (only if not dry run)
    let mut state = if !dry_run {
        Some(BossaState::load().unwrap_or_default())
    } else {
        None
    };

    // Create missing symlinks
    for op in &to_create {
        let rel_target = op
            .target
            .strip_prefix(expand_path(&symlinks.target))
            .unwrap_or(&op.target);
        println!(
            "  {} {} → {}",
            if dry_run { "○" } else { "+" }.yellow(),
            rel_target.display(),
            contract_home(&op.source)
        );

        if !dry_run {
            create_symlink(&op.source, &op.target)?;
            // Track the symlink in state
            if let Some(ref mut s) = state {
                track_symlink(s, &op.source, &op.target);
            }
        }
    }

    // Fix wrong symlinks
    for op in &to_fix {
        let rel_target = op
            .target
            .strip_prefix(expand_path(&symlinks.target))
            .unwrap_or(&op.target);
        println!(
            "  {} {} → {}",
            "~".blue(),
            rel_target.display(),
            contract_home(&op.source)
        );

        if !dry_run {
            // Remove existing symlink
            fs::remove_file(&op.target)
                .with_context(|| format!("Failed to remove: {}", op.target.display()))?;
            create_symlink(&op.source, &op.target)?;
            // Track the symlink in state (update with new source)
            if let Some(ref mut s) = state {
                // Remove old entry first (if any), then add new
                s.symlinks.remove(&op.target.to_string_lossy());
                track_symlink(s, &op.source, &op.target);
            }
        }
    }

    // Handle blocked (only if force)
    if force && !blocked.is_empty() {
        println!();
        println!("{}", "Force overwriting blocked files:".red().bold());
        for op in &blocked {
            let rel_target = op
                .target
                .strip_prefix(expand_path(&symlinks.target))
                .unwrap_or(&op.target);
            println!(
                "  {} {} → {}",
                "!".red(),
                rel_target.display(),
                contract_home(&op.source)
            );

            if !dry_run {
                // Backup and remove
                let backup = op.target.with_extension("bak");
                fs::rename(&op.target, &backup).with_context(|| {
                    format!(
                        "Failed to backup {} to {}",
                        op.target.display(),
                        backup.display()
                    )
                })?;
                create_symlink(&op.source, &op.target)?;
                println!("    {} backed up to {}", "→".dimmed(), backup.display());
                // Track the symlink in state
                if let Some(ref mut s) = state {
                    track_symlink(s, &op.source, &op.target);
                }
            }
        }
    } else if !blocked.is_empty() {
        println!();
        println!(
            "{} {} files blocked (use --force to overwrite)",
            "⚠".yellow(),
            blocked.len()
        );
        for op in &blocked {
            let rel_target = op
                .target
                .strip_prefix(expand_path(&symlinks.target))
                .unwrap_or(&op.target);
            println!("  {} {}", "⊘".red(), rel_target.display());
        }
    }

    // Save state if we made changes
    if let Some(s) = state
        && let Err(e) = s.save()
    {
        log::warn!("Failed to save state: {e}");
    }

    println!();
    if dry_run {
        println!(
            "{}",
            "Dry run complete. Run without --dry-run to apply.".dimmed()
        );
    } else {
        println!(
            "{} {} symlinks synced",
            "✓".green(),
            to_create.len() + to_fix.len()
        );
    }

    Ok(())
}

/// Preview what sync would do
fn diff(packages: &[String]) -> Result<()> {
    sync(packages, true, false)
}

/// List configured packages
fn list() -> Result<()> {
    let config = BossaConfig::load()?;
    let symlinks = get_symlinks_config(&config)?;

    let source_base = expand_path(&symlinks.source);

    println!("{}", "Configured Packages".bold());
    println!("{}", "─".repeat(40));

    for package in &symlinks.packages {
        let package_path = source_base.join(package);
        let exists = package_path.exists();

        let status = if exists { "✓".green() } else { "✗".red() };

        println!("  {status} {package}");
    }

    println!();
    println!(
        "{} {} → {}",
        "Config:".dimmed(),
        symlinks.source,
        symlinks.target
    );

    Ok(())
}

/// Add a package to config
fn add(package: &str) -> Result<()> {
    let mut config = BossaConfig::load()?;

    let symlinks = config.symlinks.get_or_insert_with(Default::default);

    if symlinks.packages.contains(&package.to_string()) {
        println!("{} '{}' is already in config", "⚠".yellow(), package);
        return Ok(());
    }

    // Verify package directory exists
    let source_base = expand_path(&symlinks.source);
    let package_path = source_base.join(package);
    if !package_path.exists() {
        println!(
            "{} Source directory does not exist yet: {}",
            "⚠".yellow(),
            package_path.display()
        );
        println!("  It will be created when you run 'bossa stow sync {package}'.");
    }

    symlinks.packages.push(package.to_string());
    symlinks.packages.sort();

    config.save()?;

    println!("{} Added '{}' to symlinks config", "✓".green(), package);
    println!("  Run 'bossa stow sync {package}' to create symlinks");

    Ok(())
}

/// Remove a package from config
fn rm(package: &str, do_unlink: bool) -> Result<()> {
    let mut config = BossaConfig::load()?;

    let symlinks = match &mut config.symlinks {
        Some(s) => s,
        None => bail!("No symlinks configured"),
    };

    if !symlinks.packages.contains(&package.to_string()) {
        bail!(
            "Package '{}' not in config. Available: {}",
            package,
            symlinks.packages.join(", ")
        );
    }

    // Unlink first if requested
    if do_unlink {
        unlink(&[package.to_string()], false)?;
    }

    symlinks.packages.retain(|p| p != package);

    config.save()?;

    println!("{} Removed '{}' from symlinks config", "✓".green(), package);

    Ok(())
}

/// Remove symlinks (opposite of sync)
fn unlink(packages: &[String], dry_run: bool) -> Result<()> {
    let config = BossaConfig::load()?;
    let symlinks = get_symlinks_config(&config)?;

    // Filter packages if specified
    let packages_to_unlink: Vec<_> = if packages.is_empty() {
        symlinks.packages.clone()
    } else {
        // Validate specified packages exist in config
        for p in packages {
            if !symlinks.packages.contains(p) {
                bail!(
                    "Package '{}' not in config. Available: {}",
                    p,
                    symlinks.packages.join(", ")
                );
            }
        }
        packages.to_vec()
    };

    let filtered_config = SymlinksConfig {
        source: symlinks.source.clone(),
        target: symlinks.target.clone(),
        packages: packages_to_unlink,
        ignore: symlinks.ignore.clone(),
    };

    let CollectResult {
        ops,
        missing_packages,
        ..
    } = collect_symlink_ops(&filtered_config)?;

    // Only unlink correct symlinks (not missing or blocked)
    let to_unlink: Vec<_> = ops
        .iter()
        .filter(|op| matches!(op.state, SymlinkState::Correct | SymlinkState::Wrong { .. }))
        .collect();

    if to_unlink.is_empty() {
        print_missing_packages(&missing_packages);
        println!("{}", "No symlinks to remove".dimmed());
        return Ok(());
    }

    let mode = if dry_run {
        "Would unlink".yellow()
    } else {
        "Unlinking".red()
    };

    println!("{} {} symlinks...", mode, to_unlink.len());
    println!();

    // Load state for tracking (only if not dry run)
    let mut state = if !dry_run {
        Some(BossaState::load().unwrap_or_default())
    } else {
        None
    };

    for op in &to_unlink {
        let rel_target = op
            .target
            .strip_prefix(expand_path(&symlinks.target))
            .unwrap_or(&op.target);
        println!(
            "  {} {}",
            if dry_run { "○" } else { "-" }.red(),
            rel_target.display()
        );

        if !dry_run {
            fs::remove_file(&op.target)
                .with_context(|| format!("Failed to remove: {}", op.target.display()))?;
            // Remove from state tracking
            if let Some(ref mut s) = state {
                s.symlinks.remove(&op.target.to_string_lossy());
            }
        }
    }

    // Save state if we made changes
    if let Some(s) = state
        && let Err(e) = s.save()
    {
        log::warn!("Failed to save state: {e}");
    }

    println!();
    if dry_run {
        println!(
            "{}",
            "Dry run complete. Run without --dry-run to apply.".dimmed()
        );
    } else {
        println!("{} {} symlinks removed", "✓".green(), to_unlink.len());
    }

    Ok(())
}

/// Initialize symlinks config
fn init(source: Option<&str>, target: Option<&str>, force: bool) -> Result<()> {
    let mut config = BossaConfig::load()?;

    if config.symlinks.is_some() && !force {
        bail!("Symlinks already configured. Use --force to overwrite.");
    }

    // Default source: ~/dotfiles
    let source_path = source.map(expand_path).unwrap_or_else(|| {
        let home = dirs::home_dir().unwrap_or_default();
        home.join("dotfiles")
    });

    // Default target: ~
    let target_path = target.map_or_else(|| dirs::home_dir().unwrap_or_default(), expand_path);

    if !source_path.exists() {
        bail!("Source directory does not exist: {}", source_path.display());
    }

    // Auto-detect packages (directories in source)
    let mut packages = Vec::new();
    for entry in fs::read_dir(&source_path)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files and common non-package directories
        if name.starts_with('.')
            || name == "README.md"
            || name == "LICENSE"
            || name == "scripts"
            || name == "tools"
        {
            continue;
        }

        if path.is_dir() {
            packages.push(name);
        }
    }

    packages.sort();

    let symlinks_config = SymlinksConfig {
        source: format!(
            "~/{}",
            source_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ),
        target: "~".to_string(),
        packages: packages.clone(),
        ignore: vec![
            ".git".to_string(),
            ".github".to_string(),
            ".gitignore".to_string(),
            ".gitmodules".to_string(),
            "README.md".to_string(),
            "LICENSE".to_string(),
        ],
    };

    config.symlinks = Some(symlinks_config);
    let config_path = config.save()?;

    println!("{}", "Symlinks Config Initialized".bold().green());
    println!("{}", "─".repeat(40));
    println!("Source:   {}", source_path.display());
    println!("Target:   {}", target_path.display());
    println!("Packages: {}", packages.join(", "));
    println!();
    println!("Config saved to: {}", config_path.display());
    println!();
    println!("Next steps:");
    println!(
        "  {} - Preview what will be linked",
        "bossa stow diff".cyan()
    );
    println!("  {} - Create the symlinks", "bossa stow sync".cyan());

    Ok(())
}

// ============================================================================
// Helper Functions
// ============================================================================

fn get_symlinks_config(config: &BossaConfig) -> Result<&SymlinksConfig> {
    config.symlinks.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "No symlinks configured. Run 'bossa stow init' or add [symlinks] to config.toml"
        )
    })
}

fn expand_path(path: &str) -> PathBuf {
    crate::paths::expand(path)
}

/// Contract home directory prefix to `~` for display
fn contract_home(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

/// Print info for missing package directories (read-only context: status, diff)
fn print_missing_packages(missing: &[MissingPackage]) {
    for pkg in missing {
        println!(
            "{} Package '{}' source directory does not exist yet",
            "⚠".yellow(),
            pkg.name
        );
        println!("    Expected: {}", pkg.expected_path.display());
        println!(
            "    Run '{}' to create it",
            format!("bossa stow sync {}", pkg.name).cyan()
        );
    }
}

/// Create missing package source directories, printing what was created.
/// Returns the number of directories created.
fn create_missing_package_dirs(missing: &[MissingPackage]) -> usize {
    let mut created = 0;
    for pkg in missing {
        match fs::create_dir_all(&pkg.expected_path) {
            Ok(()) => {
                println!(
                    "  {} Created source directory for '{}'",
                    "+".yellow(),
                    pkg.name,
                );
                println!(
                    "    Add dotfiles to {} and re-run sync",
                    pkg.expected_path.display()
                );
                created += 1;
            }
            Err(e) => {
                println!(
                    "{} Failed to create directory for '{}': {e}",
                    "✗".red(),
                    pkg.name
                );
            }
        }
    }
    created
}

/// Print hints for packages whose source directory exists but is empty
fn print_empty_packages(empty: &[EmptyPackage]) {
    for pkg in empty {
        println!(
            "{} Package '{}' source directory is empty",
            "⚠".yellow(),
            pkg.name
        );
        println!("    Add dotfiles to {} and re-run sync", pkg.path.display());
    }
}

/// Collect all symlink operations for the given config
fn collect_symlink_ops(config: &SymlinksConfig) -> Result<CollectResult> {
    let source_base = expand_path(&config.source);
    let target_base = expand_path(&config.target);

    let mut result = CollectResult::default();

    for package in &config.packages {
        let package_source = source_base.join(package);

        if !package_source.exists() {
            log::debug!(
                "Package directory does not exist: {}",
                package_source.display()
            );
            result.missing_packages.push(MissingPackage {
                name: package.clone(),
                expected_path: package_source,
            });
            continue;
        }

        let ops_before = result.ops.len();
        walk_package(
            &package_source,
            &package_source,
            &target_base,
            package,
            &config.ignore,
            &mut result.ops,
        )?;

        if result.ops.len() == ops_before {
            result.empty_packages.push(EmptyPackage {
                name: package.clone(),
                path: package_source,
            });
        }
    }

    Ok(result)
}

/// Recursively walk a package directory and collect symlink operations
fn walk_package(
    base: &Path,
    current: &Path,
    target_base: &Path,
    package: &str,
    ignore: &[String],
    ops: &mut Vec<SymlinkOp>,
) -> Result<()> {
    if !current.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip ignored patterns
        if ignore.iter().any(|p| name == *p || path.ends_with(p)) {
            continue;
        }

        // Calculate relative path and target
        let relative = path.strip_prefix(base)?;
        let target = target_base.join(relative);

        if path.is_file() || path.is_symlink() {
            // Check current state
            let state = check_symlink_state(&path, &target);
            ops.push(SymlinkOp {
                source: path,
                target,
                state,
                package: package.to_string(),
            });
        } else if path.is_dir() {
            // Recurse into directories
            walk_package(base, &path, target_base, package, ignore, ops)?;
        }
    }

    Ok(())
}

/// Check the current state of a symlink
fn check_symlink_state(source: &Path, target: &Path) -> SymlinkState {
    if !target.exists() && !target.is_symlink() {
        return SymlinkState::Missing;
    }

    if target.is_symlink() {
        match fs::read_link(target) {
            Ok(link_target) => {
                // Canonicalize for comparison
                let expected = source
                    .canonicalize()
                    .unwrap_or_else(|_| source.to_path_buf());
                let actual = if link_target.is_absolute() {
                    link_target.canonicalize().unwrap_or(link_target)
                } else {
                    target
                        .parent()
                        .map(|p| p.join(&link_target))
                        .and_then(|p| p.canonicalize().ok())
                        .unwrap_or(link_target)
                };

                if expected == actual {
                    SymlinkState::Correct
                } else {
                    SymlinkState::Wrong { actual, expected }
                }
            }
            Err(_) => SymlinkState::Wrong {
                actual: PathBuf::new(),
                expected: source.to_path_buf(),
            },
        }
    } else {
        SymlinkState::Blocked
    }
}

/// Create a symlink, ensuring parent directory exists
fn create_symlink(source: &Path, target: &Path) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    // Create symlink
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, target).with_context(|| {
        format!(
            "Failed to create symlink: {} -> {}",
            target.display(),
            source.display()
        )
    })?;

    #[cfg(windows)]
    {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        if source.is_dir() {
            // Try junction first (doesn't need admin)
            match junction::create(source, target) {
                Ok(()) => (),
                Err(e) => {
                    log::debug!("Junction failed ({}), trying symlink_dir", e);
                    symlink_dir(source, target).with_context(|| {
                        format!(
                            "Failed to create directory symlink: {} -> {}",
                            target.display(),
                            source.display()
                        )
                    })?;
                }
            }
        } else {
            symlink_file(source, target).with_context(|| {
                format!(
                    "Failed to create file symlink: {} -> {}",
                    target.display(),
                    source.display()
                )
            })?;
        }
    }

    #[cfg(not(any(unix, windows)))]
    bail!("Symlinks not supported on this platform");

    Ok(())
}

/// Track a symlink in the state inventory
fn track_symlink(state: &mut BossaState, source: &Path, target: &Path) {
    state.symlinks.add(TrackedSymlink {
        source: source.to_string_lossy().to_string(),
        target: target.to_string_lossy().to_string(),
        subsystem: "stow".to_string(),
        created_at: Utc::now(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    // ── PackageStatus tests ──────────────────────────────────────────

    #[test]
    fn status_label_empty() {
        let s = PackageStatus::default();
        assert_eq!(s.total(), 0);
        let (label, _) = s.status_label();
        assert_eq!(label, "empty");
    }

    #[test]
    fn status_label_linked() {
        let s = PackageStatus {
            linked: 3,
            ..Default::default()
        };
        assert_eq!(s.total(), 3);
        let (label, color) = s.status_label();
        assert_eq!(label, "linked");
        assert_eq!(color, colored::Color::Green);
    }

    #[test]
    fn status_label_unlinked() {
        let s = PackageStatus {
            unlinked: 2,
            ..Default::default()
        };
        let (label, _) = s.status_label();
        assert_eq!(label, "unlinked");
    }

    #[test]
    fn status_label_partial() {
        let s = PackageStatus {
            linked: 1,
            unlinked: 1,
            ..Default::default()
        };
        let (label, color) = s.status_label();
        assert_eq!(label, "partial");
        assert_eq!(color, colored::Color::Yellow);
    }

    #[test]
    fn status_label_blocked() {
        let s = PackageStatus {
            blocked: 1,
            ..Default::default()
        };
        let (label, _) = s.status_label();
        // linked == 0, so "unlinked"
        assert_eq!(label, "unlinked");
    }

    #[test]
    fn total_arithmetic() {
        let s = PackageStatus {
            linked: 1,
            unlinked: 2,
            wrong: 3,
            blocked: 4,
        };
        assert_eq!(s.total(), 10);
    }

    // ── check_symlink_state tests ────────────────────────────────────

    #[test]
    fn check_state_missing() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("source_file");
        fs::write(&source, "hello").unwrap();
        let target = tmp.path().join("nonexistent_link");

        assert!(matches!(
            check_symlink_state(&source, &target),
            SymlinkState::Missing
        ));
    }

    #[test]
    fn check_state_correct() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("source_file");
        fs::write(&source, "hello").unwrap();
        let target = tmp.path().join("link");
        symlink(&source, &target).unwrap();

        assert!(matches!(
            check_symlink_state(&source, &target),
            SymlinkState::Correct
        ));
    }

    #[test]
    fn check_state_wrong() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("source_file");
        fs::write(&source, "hello").unwrap();
        let other = tmp.path().join("other_file");
        fs::write(&other, "other").unwrap();
        let target = tmp.path().join("link");
        symlink(&other, &target).unwrap();

        assert!(matches!(
            check_symlink_state(&source, &target),
            SymlinkState::Wrong { .. }
        ));
    }

    #[test]
    fn check_state_blocked() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("source_file");
        fs::write(&source, "hello").unwrap();
        let target = tmp.path().join("regular_file");
        fs::write(&target, "blocking").unwrap();

        assert!(matches!(
            check_symlink_state(&source, &target),
            SymlinkState::Blocked
        ));
    }

    // ── collect_symlink_ops tests ────────────────────────────────────

    fn make_config(tmp: &TempDir, packages: &[&str], ignore: &[&str]) -> SymlinksConfig {
        let source = tmp.path().join("source");
        let target = tmp.path().join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        SymlinksConfig {
            source: source.to_string_lossy().to_string(),
            target: target.to_string_lossy().to_string(),
            packages: packages.iter().map(ToString::to_string).collect(),
            ignore: ignore.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn collect_missing_package_dir() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp, &["nonexistent"], &[]);

        let result = collect_symlink_ops(&config).unwrap();
        assert!(result.ops.is_empty());
        assert_eq!(result.missing_packages.len(), 1);
        assert_eq!(result.missing_packages[0].name, "nonexistent");
    }

    #[test]
    fn collect_existing_package() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp, &["mypkg"], &[]);

        // Create package dir with a file
        let pkg_dir = tmp.path().join("source").join("mypkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("file.txt"), "content").unwrap();

        let result = collect_symlink_ops(&config).unwrap();
        assert!(result.missing_packages.is_empty());
        assert_eq!(result.ops.len(), 1);
        assert_eq!(result.ops[0].package, "mypkg");
        assert!(matches!(result.ops[0].state, SymlinkState::Missing));
    }

    #[test]
    fn collect_mixed_present_and_missing() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp, &["exists", "gone"], &[]);

        let pkg_dir = tmp.path().join("source").join("exists");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("a.txt"), "a").unwrap();

        let result = collect_symlink_ops(&config).unwrap();
        assert_eq!(result.ops.len(), 1);
        assert_eq!(result.ops[0].package, "exists");
        assert_eq!(result.missing_packages.len(), 1);
        assert_eq!(result.missing_packages[0].name, "gone");
    }

    #[test]
    fn collect_ignore_patterns_filter() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp, &["pkg"], &[".git", "README.md"]);

        let pkg_dir = tmp.path().join("source").join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("keep.txt"), "keep").unwrap();
        fs::write(pkg_dir.join("README.md"), "ignore me").unwrap();
        fs::create_dir_all(pkg_dir.join(".git")).unwrap();
        fs::write(pkg_dir.join(".git").join("config"), "git stuff").unwrap();

        let result = collect_symlink_ops(&config).unwrap();
        assert!(result.missing_packages.is_empty());
        assert_eq!(result.ops.len(), 1);
        assert_eq!(
            result.ops[0].source.file_name().unwrap().to_str().unwrap(),
            "keep.txt"
        );
    }

    // ── create_missing_package_dirs tests ───────────────────────────

    #[test]
    fn create_missing_dirs_creates_directories() {
        let tmp = TempDir::new().unwrap();
        let dir1 = tmp.path().join("source").join("pkg1");
        let dir2 = tmp.path().join("source").join("pkg2");

        let missing = vec![
            MissingPackage {
                name: "pkg1".to_string(),
                expected_path: dir1.clone(),
            },
            MissingPackage {
                name: "pkg2".to_string(),
                expected_path: dir2.clone(),
            },
        ];

        let created = create_missing_package_dirs(&missing);
        assert_eq!(created, 2);
        assert!(dir1.exists());
        assert!(dir2.exists());
    }

    #[test]
    fn create_missing_dirs_empty_list() {
        let created = create_missing_package_dirs(&[]);
        assert_eq!(created, 0);
    }

    #[test]
    fn create_missing_dirs_already_exists() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("source").join("pkg");
        fs::create_dir_all(&dir).unwrap();

        let missing = vec![MissingPackage {
            name: "pkg".to_string(),
            expected_path: dir.clone(),
        }];

        // create_dir_all succeeds even if directory exists
        let created = create_missing_package_dirs(&missing);
        assert_eq!(created, 1);
        assert!(dir.exists());
    }

    // ── empty package detection tests ───────────────────────────────

    #[test]
    fn collect_empty_package_dir() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp, &["emptypkg"], &[]);

        // Create the package dir but put no files in it
        let pkg_dir = tmp.path().join("source").join("emptypkg");
        fs::create_dir_all(&pkg_dir).unwrap();

        let result = collect_symlink_ops(&config).unwrap();
        assert!(result.ops.is_empty());
        assert!(result.missing_packages.is_empty());
        assert_eq!(result.empty_packages.len(), 1);
        assert_eq!(result.empty_packages[0].name, "emptypkg");
    }

    #[test]
    fn collect_package_with_only_ignored_files_is_empty() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp, &["pkg"], &[".git", "README.md"]);

        let pkg_dir = tmp.path().join("source").join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        // Only ignored files — should count as empty
        fs::write(pkg_dir.join("README.md"), "ignored").unwrap();

        let result = collect_symlink_ops(&config).unwrap();
        assert!(result.ops.is_empty());
        assert!(result.missing_packages.is_empty());
        assert_eq!(result.empty_packages.len(), 1);
        assert_eq!(result.empty_packages[0].name, "pkg");
    }

    #[test]
    fn collect_mixed_missing_empty_and_present() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp, &["present", "empty", "gone"], &[]);

        // "present" has a file
        let present_dir = tmp.path().join("source").join("present");
        fs::create_dir_all(&present_dir).unwrap();
        fs::write(present_dir.join("file.txt"), "content").unwrap();

        // "empty" exists but has no files
        let empty_dir = tmp.path().join("source").join("empty");
        fs::create_dir_all(&empty_dir).unwrap();

        // "gone" doesn't exist at all

        let result = collect_symlink_ops(&config).unwrap();
        assert_eq!(result.ops.len(), 1);
        assert_eq!(result.ops[0].package, "present");
        assert_eq!(result.missing_packages.len(), 1);
        assert_eq!(result.missing_packages[0].name, "gone");
        assert_eq!(result.empty_packages.len(), 1);
        assert_eq!(result.empty_packages[0].name, "empty");
    }
}
