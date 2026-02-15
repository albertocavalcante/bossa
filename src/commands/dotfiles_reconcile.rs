//! Dotfiles reconciliation — hash-based multi-source drift detection.
//!
//! Performs three-way blake3 hash comparison across two dotfile sources
//! and a deployed target to detect drift and reconcile divergent files.
//! Requires `[dotfiles_reconcile]` in config.

use anyhow::{Context, Result, bail};
use colored::Colorize;
use dialoguer::{Confirm, Input};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::Context as AppContext;
use crate::cli::ReconcileStrategy;
use crate::schema::{BossaConfig, DotfilesReconcileConfig};

// ============================================================================
// Types
// ============================================================================

/// State of a single file across three locations
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum FileState {
    /// A == B == target (or A == B and no target)
    Synced,
    /// Exists only in source A
    OnlyInA { hash: String },
    /// Exists only in source B
    OnlyInB { hash: String },
    /// Deployed at target but not in either source
    OnlyInTarget { hash: String },
    /// A == target != B
    DriftedTargetMatchesA { a: String, b: String },
    /// B == target != A
    DriftedTargetMatchesB { a: String, b: String },
    /// All three differ
    DriftedAllDiffer { a: String, b: String, t: String },
    /// A == B != target (edited directly at target)
    StaleTarget { source: String, target: String },
}

/// A file entry with its reconciliation state
#[derive(Debug, Clone)]
struct FileEntry {
    relative_path: String,
    state: FileState,
    size_a: Option<u64>,
    size_b: Option<u64>,
    size_target: Option<u64>,
}

/// Full reconciliation report
#[derive(Debug, Default)]
struct ReconcileReport {
    synced: Vec<FileEntry>,
    only_a: Vec<FileEntry>,
    only_b: Vec<FileEntry>,
    only_target: Vec<FileEntry>,
    drifted: Vec<FileEntry>,
    stale: Vec<FileEntry>,
}

/// Hash and size for a single file
type FileInfo = (String, u64);

/// Tree of relative_path -> (blake3_hash, size)
type FileTree = BTreeMap<String, FileInfo>;

// ============================================================================
// Public entry points
// ============================================================================

/// Three-way hash comparison report (`bossa dotfiles drift`)
pub fn drift(_ctx: &AppContext, filter_packages: &[String]) -> Result<()> {
    let config = load_config()?;

    if filter_packages.is_empty() {
        let report = build_full_report(&config)?;
        display_report(&report, &config);
    } else {
        let report = build_full_report_filtered(&config, filter_packages)?;
        display_diff(&report, &config);
    }

    Ok(())
}

/// Execute reconciliation (`bossa dotfiles reconcile`)
pub fn reconcile(
    _ctx: &AppContext,
    dry_run: bool,
    strategy: ReconcileStrategy,
    filter_packages: &[String],
) -> Result<()> {
    let config = load_config()?;
    let report = build_full_report_filtered(&config, filter_packages)?;

    let total_actions = report.only_a.len()
        + report.only_b.len()
        + report.only_target.len()
        + report.drifted.len()
        + report.stale.len();

    if total_actions == 0 {
        println!("{} All dotfiles are in sync", "✓".green());
        return Ok(());
    }

    let mode = if dry_run {
        "Would reconcile".yellow()
    } else {
        "Reconciling".green()
    };

    println!("{mode} {total_actions} files...");
    println!();

    let source_a = expand_path(&config.source_a);
    let source_b = expand_path(&config.source_b);
    let target = expand_path(&config.target);

    let mut copied = 0usize;

    // Handle only_a: copy A -> B (and optionally target)
    for entry in &report.only_a {
        let action = match strategy {
            ReconcileStrategy::AWins | ReconcileStrategy::NewestWins => "copy A → B",
            ReconcileStrategy::BWins => "skip (B-wins: not in B)",
            ReconcileStrategy::Interactive => "copy A → B",
        };
        println!(
            "  {} {} ({})",
            "→".cyan(),
            entry.relative_path,
            action.dimmed()
        );
        if !dry_run && !matches!(strategy, ReconcileStrategy::BWins) {
            copy_file(
                &source_a.join(&entry.relative_path),
                &source_b.join(&entry.relative_path),
            )?;
            copied += 1;
        }
    }

    // Handle only_b: copy B -> A
    for entry in &report.only_b {
        let action = match strategy {
            ReconcileStrategy::BWins | ReconcileStrategy::NewestWins => "copy B → A",
            ReconcileStrategy::AWins => "skip (A-wins: not in A)",
            ReconcileStrategy::Interactive => "copy B → A",
        };
        println!(
            "  {} {} ({})",
            "→".cyan(),
            entry.relative_path,
            action.dimmed()
        );
        if !dry_run && !matches!(strategy, ReconcileStrategy::AWins) {
            copy_file(
                &source_b.join(&entry.relative_path),
                &source_a.join(&entry.relative_path),
            )?;
            copied += 1;
        }
    }

    // Handle only_target: show warning (file exists at target but not in sources)
    for entry in &report.only_target {
        println!(
            "  {} {} ({})",
            "⚠".yellow(),
            entry.relative_path,
            "only at target, not in sources".dimmed()
        );
    }

    // Handle drifted files
    for entry in &report.drifted {
        match (&entry.state, strategy) {
            (FileState::DriftedTargetMatchesA { .. }, ReconcileStrategy::AWins) => {
                println!(
                    "  {} {} ({})",
                    "→".cyan(),
                    entry.relative_path,
                    "A → B (A-wins, target matches A)".dimmed()
                );
                if !dry_run {
                    copy_file(
                        &source_a.join(&entry.relative_path),
                        &source_b.join(&entry.relative_path),
                    )?;
                    copied += 1;
                }
            }
            (FileState::DriftedTargetMatchesB { .. }, ReconcileStrategy::BWins) => {
                println!(
                    "  {} {} ({})",
                    "→".cyan(),
                    entry.relative_path,
                    "B → A (B-wins, target matches B)".dimmed()
                );
                if !dry_run {
                    copy_file(
                        &source_b.join(&entry.relative_path),
                        &source_a.join(&entry.relative_path),
                    )?;
                    copied += 1;
                }
            }
            (FileState::DriftedTargetMatchesA { .. }, ReconcileStrategy::BWins) => {
                println!(
                    "  {} {} ({})",
                    "→".cyan(),
                    entry.relative_path,
                    "B → A,target (B-wins)".dimmed()
                );
                if !dry_run {
                    let src = source_b.join(&entry.relative_path);
                    copy_file(&src, &source_a.join(&entry.relative_path))?;
                    copy_file(&src, &target.join(&entry.relative_path))?;
                    copied += 1;
                }
            }
            (FileState::DriftedTargetMatchesB { .. }, ReconcileStrategy::AWins) => {
                println!(
                    "  {} {} ({})",
                    "→".cyan(),
                    entry.relative_path,
                    "A → B,target (A-wins)".dimmed()
                );
                if !dry_run {
                    let src = source_a.join(&entry.relative_path);
                    copy_file(&src, &source_b.join(&entry.relative_path))?;
                    copy_file(&src, &target.join(&entry.relative_path))?;
                    copied += 1;
                }
            }
            (_, ReconcileStrategy::NewestWins) => {
                let a_path = source_a.join(&entry.relative_path);
                let b_path = source_b.join(&entry.relative_path);
                let a_newer = is_newer(&a_path, &b_path);
                if a_newer {
                    println!(
                        "  {} {} ({})",
                        "→".cyan(),
                        entry.relative_path,
                        "A → B (newest-wins, A is newer)".dimmed()
                    );
                    if !dry_run {
                        copy_file(&a_path, &b_path)?;
                        copied += 1;
                    }
                } else {
                    println!(
                        "  {} {} ({})",
                        "→".cyan(),
                        entry.relative_path,
                        "B → A (newest-wins, B is newer)".dimmed()
                    );
                    if !dry_run {
                        copy_file(&b_path, &a_path)?;
                        copied += 1;
                    }
                }
            }
            (_, ReconcileStrategy::Interactive) => {
                println!(
                    "  {} {} ({})",
                    "⚠".yellow(),
                    entry.relative_path,
                    "conflict - skipped (interactive not yet implemented)".dimmed()
                );
            }
            _ => {
                println!(
                    "  {} {} ({})",
                    "?".bright_black(),
                    entry.relative_path,
                    "unhandled state".dimmed()
                );
            }
        }
    }

    // Handle stale target files (A == B != target)
    for entry in &report.stale {
        println!(
            "  {} {} ({})",
            "⚠".yellow(),
            entry.relative_path,
            "target diverged from sources".dimmed()
        );
        if !dry_run {
            copy_file(
                &source_a.join(&entry.relative_path),
                &target.join(&entry.relative_path),
            )?;
            copied += 1;
        }
    }

    println!();
    if dry_run {
        println!(
            "{}",
            "Dry run complete. Run without --dry-run to apply.".dimmed()
        );
    } else {
        println!("{} {copied} files reconciled", "✓".green());
    }

    Ok(())
}

/// Health check (`bossa dotfiles check`)
pub fn check(_ctx: &AppContext) -> Result<()> {
    let config = load_config()?;
    let mut issues = 0;

    println!("{}", "Dotfiles Reconciliation Health Check".bold());
    println!("{}", "─".repeat(40));

    // Check source A
    let source_a = expand_path(&config.source_a);
    if source_a.exists() {
        println!("  {} Source A: {}", "✓".green(), config.source_a);
        if source_a.join(".git").exists() {
            println!("    {} Git repository", "✓".green());
        } else {
            println!("    {} Not a git repository", "⚠".yellow());
        }
    } else {
        println!("  {} Source A: {} (not found)", "✗".red(), config.source_a);
        issues += 1;
    }

    // Check source B
    let source_b = expand_path(&config.source_b);
    if source_b.exists() {
        println!("  {} Source B: {}", "✓".green(), config.source_b);
        if source_b.join(".git").exists() {
            println!("    {} Git repository", "✓".green());
        } else {
            println!("    {} Not a git repository", "⚠".yellow());
        }
    } else {
        println!("  {} Source B: {} (not found)", "✗".red(), config.source_b);
        issues += 1;
    }

    // Check target
    let target = expand_path(&config.target);
    if target.exists() {
        println!("  {} Target:   {}", "✓".green(), config.target);
    } else {
        println!("  {} Target:   {} (not found)", "✗".red(), config.target);
        issues += 1;
    }

    // Check packages
    if !config.packages.is_empty() {
        println!();
        println!("  {}", "Packages:".bold());
        for pkg in &config.packages {
            let a_exists = source_a.join(pkg).exists();
            let b_exists = source_b.join(pkg).exists();
            let icon = if a_exists && b_exists {
                "✓".green()
            } else if a_exists || b_exists {
                "◐".yellow()
            } else {
                "✗".red()
            };
            let detail = match (a_exists, b_exists) {
                (true, true) => "both sources".to_string(),
                (true, false) => "A only".to_string(),
                (false, true) => "B only".to_string(),
                (false, false) => {
                    issues += 1;
                    "neither source".to_string()
                }
            };
            println!("    {icon} {pkg} ({detail})");
        }
    }

    // Check for broken symlinks in target
    if target.exists() && !config.packages.is_empty() {
        let mut broken_links = Vec::new();
        for pkg in &config.packages {
            let target_pkg = target.join(pkg);
            if target_pkg.exists() {
                check_broken_symlinks(&target_pkg, &target, &mut broken_links);
            }
        }
        if !broken_links.is_empty() {
            println!();
            println!("  {}", "Broken symlinks:".red().bold());
            for link in &broken_links {
                println!("    {} {}", "✗".red(), link);
                issues += 1;
            }
        }
    }

    println!();
    if issues == 0 {
        println!("  {} All checks passed", "✓".green());
    } else {
        println!(
            "  {} {issues} issue{} found",
            "⚠".yellow(),
            if issues == 1 { "" } else { "s" }
        );
    }
    println!();

    Ok(())
}

// ============================================================================
// Core Logic
// ============================================================================

/// Walk a directory tree and hash all files with blake3
fn hash_tree(root: &Path, ignore: &[String]) -> Result<FileTree> {
    let mut tree = BTreeMap::new();

    if !root.exists() {
        return Ok(tree);
    }

    walk_and_hash(root, root, ignore, &mut tree)?;
    Ok(tree)
}

/// Recursive walker for hash_tree
fn walk_and_hash(
    base: &Path,
    current: &Path,
    ignore: &[String],
    tree: &mut FileTree,
) -> Result<()> {
    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("Cannot read directory {}: {e}", current.display());
            return Ok(());
        }
    };

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip ignored patterns
        if ignore.iter().any(|p| name == *p || path.ends_with(p)) {
            continue;
        }

        if path.is_file() {
            let relative = path
                .strip_prefix(base)
                .with_context(|| format!("Failed to strip prefix from {}", path.display()))?;
            let relative_str = relative.to_string_lossy().to_string();

            let content =
                fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
            let hash = blake3::hash(&content).to_hex().to_string();
            let size = content.len() as u64;

            tree.insert(relative_str, (hash, size));
        } else if path.is_dir() {
            walk_and_hash(base, &path, ignore, tree)?;
        }
    }

    Ok(())
}

/// Three-way comparison producing a reconciliation report
fn build_report(tree_a: &FileTree, tree_b: &FileTree, tree_target: &FileTree) -> ReconcileReport {
    let mut report = ReconcileReport::default();

    // Collect all unique paths
    let all_paths: HashSet<&String> = tree_a
        .keys()
        .chain(tree_b.keys())
        .chain(tree_target.keys())
        .collect();

    for path in all_paths {
        let a = tree_a.get(path);
        let b = tree_b.get(path);
        let t = tree_target.get(path);

        let entry = FileEntry {
            relative_path: path.clone(),
            state: classify(a, b, t),
            size_a: a.map(|(_, s)| *s),
            size_b: b.map(|(_, s)| *s),
            size_target: t.map(|(_, s)| *s),
        };

        match &entry.state {
            FileState::Synced => report.synced.push(entry),
            FileState::OnlyInA { .. } => report.only_a.push(entry),
            FileState::OnlyInB { .. } => report.only_b.push(entry),
            FileState::OnlyInTarget { .. } => report.only_target.push(entry),
            FileState::DriftedTargetMatchesA { .. }
            | FileState::DriftedTargetMatchesB { .. }
            | FileState::DriftedAllDiffer { .. } => report.drifted.push(entry),
            FileState::StaleTarget { .. } => report.stale.push(entry),
        }
    }

    // Sort all vectors by path for consistent output
    report
        .synced
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    report
        .only_a
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    report
        .only_b
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    report
        .only_target
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    report
        .drifted
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    report
        .stale
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    report
}

/// Classify a file's state based on presence/absence and hash values
fn classify(a: Option<&FileInfo>, b: Option<&FileInfo>, t: Option<&FileInfo>) -> FileState {
    match (a, b, t) {
        // All three present
        (Some((ha, _)), Some((hb, _)), Some((ht, _))) => {
            if ha == hb && hb == ht {
                FileState::Synced
            } else if ha == hb {
                FileState::StaleTarget {
                    source: ha.clone(),
                    target: ht.clone(),
                }
            } else if ha == ht {
                FileState::DriftedTargetMatchesA {
                    a: ha.clone(),
                    b: hb.clone(),
                }
            } else if hb == ht {
                FileState::DriftedTargetMatchesB {
                    a: ha.clone(),
                    b: hb.clone(),
                }
            } else {
                FileState::DriftedAllDiffer {
                    a: ha.clone(),
                    b: hb.clone(),
                    t: ht.clone(),
                }
            }
        }
        // Both sources, no target
        (Some((ha, _)), Some((hb, _)), None) => {
            if ha == hb {
                FileState::Synced
            } else {
                FileState::DriftedAllDiffer {
                    a: ha.clone(),
                    b: hb.clone(),
                    t: String::new(),
                }
            }
        }
        // Only in A (with or without target)
        (Some((ha, _)), None, Some((ht, _))) => {
            if ha == ht {
                FileState::OnlyInA { hash: ha.clone() }
            } else {
                FileState::DriftedAllDiffer {
                    a: ha.clone(),
                    b: String::new(),
                    t: ht.clone(),
                }
            }
        }
        (Some((ha, _)), None, None) => FileState::OnlyInA { hash: ha.clone() },
        // Only in B (with or without target)
        (None, Some((hb, _)), Some((ht, _))) => {
            if hb == ht {
                FileState::OnlyInB { hash: hb.clone() }
            } else {
                FileState::DriftedAllDiffer {
                    a: String::new(),
                    b: hb.clone(),
                    t: ht.clone(),
                }
            }
        }
        (None, Some((hb, _)), None) => FileState::OnlyInB { hash: hb.clone() },
        // Only in target
        (None, None, Some((ht, _))) => FileState::OnlyInTarget { hash: ht.clone() },
        // Nothing (shouldn't happen since we only iterate known paths)
        (None, None, None) => FileState::Synced,
    }
}

// ============================================================================
// Report Building
// ============================================================================

/// Build a full report from config (all packages)
fn build_full_report(config: &DotfilesReconcileConfig) -> Result<ReconcileReport> {
    build_full_report_filtered(config, &[])
}

/// Build a full report, optionally filtering to specific packages
fn build_full_report_filtered(
    config: &DotfilesReconcileConfig,
    filter_packages: &[String],
) -> Result<ReconcileReport> {
    let source_a = expand_path(&config.source_a);
    let source_b = expand_path(&config.source_b);
    let target_base = expand_path(&config.target);

    let ignore = default_ignore(&config.ignore);

    let packages = resolve_packages(config, &source_a, &source_b, filter_packages)?;

    let mut combined_report = ReconcileReport::default();

    for pkg in &packages {
        let a_dir = source_a.join(pkg);
        let b_dir = source_b.join(pkg);
        let t_dir = target_base.join(pkg);

        let tree_a = hash_tree(&a_dir, &ignore)?;
        let tree_b = hash_tree(&b_dir, &ignore)?;
        let tree_target = hash_tree(&t_dir, &ignore)?;

        let report = build_report(&tree_a, &tree_b, &tree_target);

        // Prefix all paths with the package name
        let prefix = |entries: Vec<FileEntry>, pkg: &str| -> Vec<FileEntry> {
            entries
                .into_iter()
                .map(|mut e| {
                    e.relative_path = format!("{pkg}/{}", e.relative_path);
                    e
                })
                .collect()
        };

        combined_report.synced.extend(prefix(report.synced, pkg));
        combined_report.only_a.extend(prefix(report.only_a, pkg));
        combined_report.only_b.extend(prefix(report.only_b, pkg));
        combined_report
            .only_target
            .extend(prefix(report.only_target, pkg));
        combined_report.drifted.extend(prefix(report.drifted, pkg));
        combined_report.stale.extend(prefix(report.stale, pkg));
    }

    Ok(combined_report)
}

/// Resolve which packages to compare
fn resolve_packages(
    config: &DotfilesReconcileConfig,
    source_a: &Path,
    source_b: &Path,
    filter: &[String],
) -> Result<Vec<String>> {
    if !filter.is_empty() {
        for pkg in filter {
            if !source_a.join(pkg).exists() && !source_b.join(pkg).exists() {
                bail!("Package '{pkg}' not found in either source");
            }
        }
        return Ok(filter.to_vec());
    }

    if !config.packages.is_empty() {
        return Ok(config.packages.clone());
    }

    // Auto-discover: find all subdirectories that exist in either source
    let mut packages = HashSet::new();
    let ignore = default_ignore(&config.ignore);

    for source in [source_a, source_b] {
        if let Ok(entries) = fs::read_dir(source) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().is_dir() && !ignore.contains(&name) {
                    packages.insert(name);
                }
            }
        }
    }

    let mut sorted: Vec<String> = packages.into_iter().collect();
    sorted.sort();
    Ok(sorted)
}

// ============================================================================
// Display
// ============================================================================

/// Display the reconciliation report summary
fn display_report(report: &ReconcileReport, config: &DotfilesReconcileConfig) {
    println!();
    println!("{}", "Dotfiles Drift Report".bold());
    println!("{}", "─".repeat(50));
    println!("  Source A: {}", config.source_a.cyan());
    println!("  Source B: {}", config.source_b.cyan());
    println!("  Target:  {}", config.target.cyan());
    println!();

    // Summary table
    println!("  {:<24} {:>6}", "Category".bold(), "Count".bold());
    println!("  {} {}", "─".repeat(24), "─".repeat(6));

    let rows = [
        ("✓ Synced", report.synced.len(), colored::Color::Green),
        ("A Only in A", report.only_a.len(), colored::Color::Cyan),
        ("B Only in B", report.only_b.len(), colored::Color::Blue),
        (
            "T Only in target",
            report.only_target.len(),
            colored::Color::Yellow,
        ),
        ("⚡ Drifted", report.drifted.len(), colored::Color::Red),
        ("⚠ Stale target", report.stale.len(), colored::Color::Yellow),
    ];

    for (label, count, color) in rows {
        if count > 0 {
            println!(
                "  {:<24} {:>6}",
                label.color(color),
                count.to_string().color(color)
            );
        } else {
            println!(
                "  {:<24} {:>6}",
                label.bright_black(),
                count.to_string().bright_black()
            );
        }
    }

    println!();

    // Show details for non-synced files
    if !report.only_a.is_empty() {
        println!("  {}", "Only in A:".cyan().bold());
        for entry in &report.only_a {
            let size = format_size(entry.size_a.unwrap_or(0));
            println!(
                "    {} {} ({})",
                "A".cyan(),
                entry.relative_path,
                size.dimmed()
            );
        }
        println!();
    }

    if !report.only_b.is_empty() {
        println!("  {}", "Only in B:".blue().bold());
        for entry in &report.only_b {
            let size = format_size(entry.size_b.unwrap_or(0));
            println!(
                "    {} {} ({})",
                "B".blue(),
                entry.relative_path,
                size.dimmed()
            );
        }
        println!();
    }

    if !report.only_target.is_empty() {
        println!("  {}", "Only in target:".yellow().bold());
        for entry in &report.only_target {
            let size = format_size(entry.size_target.unwrap_or(0));
            println!(
                "    {} {} ({})",
                "T".yellow(),
                entry.relative_path,
                size.dimmed()
            );
        }
        println!();
    }

    if !report.drifted.is_empty() {
        println!("  {}", "Drifted:".red().bold());
        for entry in &report.drifted {
            let detail = match &entry.state {
                FileState::DriftedTargetMatchesA { .. } => "target=A, B differs",
                FileState::DriftedTargetMatchesB { .. } => "target=B, A differs",
                FileState::DriftedAllDiffer { .. } => "all differ",
                _ => "drift",
            };
            println!(
                "    {} {} ({})",
                "⚡".red(),
                entry.relative_path,
                detail.dimmed()
            );
        }
        println!();
    }

    if !report.stale.is_empty() {
        println!(
            "  {}",
            "Stale target (A==B, target diverged):".yellow().bold()
        );
        for entry in &report.stale {
            println!("    {} {}", "⚠".yellow(), entry.relative_path);
        }
        println!();
    }

    // Action hint
    let total_actions =
        report.only_a.len() + report.only_b.len() + report.drifted.len() + report.stale.len();
    if total_actions > 0 {
        println!(
            "  Run {} to preview reconciliation",
            "bossa dotfiles reconcile --dry-run".cyan()
        );
    } else {
        println!("  {} All dotfiles are in sync", "✓".green());
    }
    println!();
}

/// Display detailed per-file diff
fn display_diff(report: &ReconcileReport, config: &DotfilesReconcileConfig) {
    println!();
    println!("{}", "Dotfiles Drift Detail".bold());
    println!("{}", "─".repeat(50));
    println!("  Source A: {}", config.source_a.cyan());
    println!("  Source B: {}", config.source_b.cyan());
    println!("  Target:  {}", config.target.cyan());
    println!();

    let source_a = expand_path(&config.source_a);
    let source_b = expand_path(&config.source_b);
    let target = expand_path(&config.target);

    let has_diff = !report.only_a.is_empty()
        || !report.only_b.is_empty()
        || !report.only_target.is_empty()
        || !report.drifted.is_empty()
        || !report.stale.is_empty();

    if !has_diff {
        println!("  {} No differences found", "✓".green());
        println!();
        return;
    }

    // Show drifted files with content diff
    for entry in &report.drifted {
        let a_path = source_a.join(&entry.relative_path);
        let b_path = source_b.join(&entry.relative_path);

        println!("  {} {}", "───".red(), entry.relative_path.bold());

        match &entry.state {
            FileState::DriftedTargetMatchesA { a, b } => {
                println!("    A: {} (target matches)", &a[..8.min(a.len())].green());
                println!("    B: {}", &b[..8.min(b.len())].red());
            }
            FileState::DriftedTargetMatchesB { a, b } => {
                println!("    A: {}", &a[..8.min(a.len())].red());
                println!("    B: {} (target matches)", &b[..8.min(b.len())].green());
            }
            FileState::DriftedAllDiffer { a, b, t } => {
                println!("    A: {}", &a[..8.min(a.len())].yellow());
                println!("    B: {}", &b[..8.min(b.len())].yellow());
                if !t.is_empty() {
                    println!("    T: {}", &t[..8.min(t.len())].yellow());
                }
            }
            _ => {}
        }

        // Show text diff if files are readable as text
        if a_path.exists() && b_path.exists() {
            show_text_diff(&a_path, &b_path);
        }

        println!();
    }

    // Show stale entries
    for entry in &report.stale {
        let a_path = source_a.join(&entry.relative_path);
        let t_path = target.join(&entry.relative_path);

        println!("  {} {}", "───".yellow(), entry.relative_path.bold());
        println!("    Sources agree, but target has been edited directly");

        if a_path.exists() && t_path.exists() {
            show_text_diff(&a_path, &t_path);
        }
        println!();
    }

    // Show only_a / only_b entries briefly
    for entry in &report.only_a {
        println!("  {} {} (only in A)", "+".cyan(), entry.relative_path);
    }
    for entry in &report.only_b {
        println!("  {} {} (only in B)", "+".blue(), entry.relative_path);
    }
    for entry in &report.only_target {
        println!(
            "  {} {} (only in target)",
            "?".yellow(),
            entry.relative_path
        );
    }

    println!();
}

/// Show a text diff between two files using the `similar` crate
fn show_text_diff(a: &Path, b: &Path) {
    let Ok(text_a) = fs::read_to_string(a) else {
        return;
    };
    let Ok(text_b) = fs::read_to_string(b) else {
        return;
    };

    let diff = similar::TextDiff::from_lines(&text_a, &text_b);
    let mut has_changes = false;

    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Delete => {
                has_changes = true;
                print!("    {}", format!("- {change}").red());
            }
            similar::ChangeTag::Insert => {
                has_changes = true;
                print!("    {}", format!("+ {change}").green());
            }
            similar::ChangeTag::Equal => {}
        }
    }

    if !has_changes {
        println!("    {}", "(files are identical)".dimmed());
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Load reconciliation config from main config, launching setup wizard if missing
fn load_config() -> Result<DotfilesReconcileConfig> {
    let mut config = BossaConfig::load()?;
    if let Some(reconcile) = config.dotfiles_reconcile {
        return Ok(reconcile);
    }
    setup_reconcile_config(&mut config)
}

/// Interactive setup wizard for `[dotfiles_reconcile]` config section.
///
/// Falls back to a static error with a TOML example when stdin is not a
/// terminal (CI, pipes) so the process never hangs waiting for input.
fn setup_reconcile_config(config: &mut BossaConfig) -> Result<DotfilesReconcileConfig> {
    if !std::io::stdin().is_terminal() {
        bail!(
            "No [dotfiles_reconcile] section in config.\n\
             Add it to your config file, for example:\n\n\
             [dotfiles_reconcile]\n\
             source_a = \"~/.dotfiles\"\n\
             source_b = \"/Volumes/T9/dev/ws/utils/dotfiles\"\n\
             target = \"~\""
        );
    }

    println!();
    println!(
        "{}",
        "No [dotfiles_reconcile] config found — let's set it up.".yellow()
    );
    println!();

    // Pre-fill source_a from [dotfiles].path when available
    let default_a = config
        .dotfiles
        .as_ref()
        .map(|d| d.path.clone())
        .unwrap_or_default();

    let source_a: String = Input::new()
        .with_prompt("Source A (primary dotfiles directory)")
        .with_initial_text(&default_a)
        .interact_text()
        .context("Failed to read source_a")?;

    let source_b: String = Input::new()
        .with_prompt("Source B (secondary dotfiles directory)")
        .interact_text()
        .context("Failed to read source_b")?;

    let target: String = Input::new()
        .with_prompt("Target (where dotfiles are deployed)")
        .with_initial_text("~")
        .interact_text()
        .context("Failed to read target")?;

    // Auto-discover packages from both source directories
    let path_a = expand_path(&source_a);
    let path_b = expand_path(&source_b);
    let ignore = default_ignore(&[]);
    let mut packages = HashSet::new();

    for source in [&path_a, &path_b] {
        if let Ok(entries) = fs::read_dir(source) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().is_dir() && !ignore.contains(&name) {
                    packages.insert(name);
                }
            }
        }
    }

    let mut sorted_packages: Vec<String> = packages.into_iter().collect();
    sorted_packages.sort();

    // Summary
    println!();
    println!("{}", "Configuration summary:".bold());
    println!("  source_a = {}", source_a.cyan());
    println!("  source_b = {}", source_b.cyan());
    println!("  target   = {}", target.cyan());
    if sorted_packages.is_empty() {
        println!(
            "  packages = {} (will auto-discover at runtime)",
            "[]".dimmed()
        );
    } else {
        println!("  packages = {sorted_packages:?}");
    }
    println!();

    let confirmed = Confirm::new()
        .with_prompt("Save to config?")
        .default(true)
        .interact()
        .context("Failed to read confirmation")?;

    if !confirmed {
        bail!("Setup cancelled");
    }

    let reconcile = DotfilesReconcileConfig {
        source_a,
        source_b,
        target,
        packages: sorted_packages,
        ignore: Vec::new(),
        strategy: "interactive".to_string(),
    };

    config.dotfiles_reconcile = Some(reconcile.clone());
    let path = config.save()?;
    println!();
    println!(
        "{} Saved to {}",
        "✓".green(),
        path.display().to_string().dimmed()
    );
    println!();

    Ok(reconcile)
}

/// Expand ~ and env vars in a path
fn expand_path(path: &str) -> PathBuf {
    crate::paths::expand(path)
}

/// Default ignore patterns merged with user config
fn default_ignore(user_ignore: &[String]) -> Vec<String> {
    let mut ignore: Vec<String> = vec![
        ".git".to_string(),
        ".github".to_string(),
        ".DS_Store".to_string(),
        "README.md".to_string(),
        "LICENSE".to_string(),
    ];
    for pattern in user_ignore {
        if !ignore.contains(pattern) {
            ignore.push(pattern.clone());
        }
    }
    ignore
}

/// Format a file size in human-readable form
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Copy a file, creating parent directories as needed
fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }
    fs::copy(src, dst)
        .with_context(|| format!("Failed to copy {} → {}", src.display(), dst.display()))?;
    Ok(())
}

/// Check if file a has a newer mtime than file b
fn is_newer(a: &Path, b: &Path) -> bool {
    let a_mtime = a
        .metadata()
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let b_mtime = b
        .metadata()
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    a_mtime > b_mtime
}

/// Recursively check for broken symlinks in a directory
fn check_broken_symlinks(dir: &Path, base: &Path, broken: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_symlink() && !path.exists() {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            broken.push(rel);
        } else if path.is_dir() {
            check_broken_symlinks(&path, base, broken);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // ── hash_tree tests ──────────────────────────────────────────────

    #[test]
    fn hash_tree_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let tree = hash_tree(tmp.path(), &[]).unwrap();
        assert!(tree.is_empty());
    }

    #[test]
    fn hash_tree_single_file() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "hello.txt", "hello world");

        let tree = hash_tree(tmp.path(), &[]).unwrap();
        assert_eq!(tree.len(), 1);
        assert!(tree.contains_key("hello.txt"));
        let (hash, size) = &tree["hello.txt"];
        assert!(!hash.is_empty());
        assert_eq!(*size, 11);
    }

    #[test]
    fn hash_tree_nested_files() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "a/b/c.txt", "nested");
        write_file(tmp.path(), "top.txt", "top");

        let tree = hash_tree(tmp.path(), &[]).unwrap();
        assert_eq!(tree.len(), 2);
        assert!(tree.contains_key("a/b/c.txt"));
        assert!(tree.contains_key("top.txt"));
    }

    #[test]
    fn hash_tree_respects_ignore() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "keep.txt", "keep");
        write_file(tmp.path(), ".git/config", "git stuff");
        write_file(tmp.path(), "README.md", "readme");

        let tree = hash_tree(tmp.path(), &[".git".to_string(), "README.md".to_string()]).unwrap();
        assert_eq!(tree.len(), 1);
        assert!(tree.contains_key("keep.txt"));
    }

    #[test]
    fn hash_tree_nonexistent_dir() {
        let tree = hash_tree(Path::new("/nonexistent/dir"), &[]).unwrap();
        assert!(tree.is_empty());
    }

    #[test]
    fn hash_tree_consistent_hashes() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "test.txt", "same content");

        let tree1 = hash_tree(tmp.path(), &[]).unwrap();
        let tree2 = hash_tree(tmp.path(), &[]).unwrap();
        assert_eq!(tree1["test.txt"].0, tree2["test.txt"].0);
    }

    // ── classify tests ───────────────────────────────────────────────

    #[test]
    fn classify_all_same() {
        let info = ("abc123".to_string(), 10);
        let state = classify(Some(&info), Some(&info), Some(&info));
        assert!(matches!(state, FileState::Synced));
    }

    #[test]
    fn classify_ab_same_no_target() {
        let info = ("abc123".to_string(), 10);
        let state = classify(Some(&info), Some(&info), None);
        assert!(matches!(state, FileState::Synced));
    }

    #[test]
    fn classify_only_a() {
        let info = ("abc123".to_string(), 10);
        let state = classify(Some(&info), None, None);
        assert!(matches!(state, FileState::OnlyInA { .. }));
    }

    #[test]
    fn classify_only_b() {
        let info = ("abc123".to_string(), 10);
        let state = classify(None, Some(&info), None);
        assert!(matches!(state, FileState::OnlyInB { .. }));
    }

    #[test]
    fn classify_only_target() {
        let info = ("abc123".to_string(), 10);
        let state = classify(None, None, Some(&info));
        assert!(matches!(state, FileState::OnlyInTarget { .. }));
    }

    #[test]
    fn classify_stale_target() {
        let source = ("abc123".to_string(), 10);
        let target = ("def456".to_string(), 10);
        let state = classify(Some(&source), Some(&source), Some(&target));
        assert!(matches!(state, FileState::StaleTarget { .. }));
    }

    #[test]
    fn classify_target_matches_a() {
        let a = ("abc123".to_string(), 10);
        let b = ("def456".to_string(), 10);
        let state = classify(Some(&a), Some(&b), Some(&a));
        assert!(matches!(state, FileState::DriftedTargetMatchesA { .. }));
    }

    #[test]
    fn classify_target_matches_b() {
        let a = ("abc123".to_string(), 10);
        let b = ("def456".to_string(), 10);
        let state = classify(Some(&a), Some(&b), Some(&b));
        assert!(matches!(state, FileState::DriftedTargetMatchesB { .. }));
    }

    #[test]
    fn classify_all_differ() {
        let a = ("aaa".to_string(), 10);
        let b = ("bbb".to_string(), 10);
        let t = ("ttt".to_string(), 10);
        let state = classify(Some(&a), Some(&b), Some(&t));
        assert!(matches!(state, FileState::DriftedAllDiffer { .. }));
    }

    // ── build_report tests ───────────────────────────────────────────

    #[test]
    fn build_report_all_synced() {
        let mut tree = BTreeMap::new();
        tree.insert("file.txt".to_string(), ("abc".to_string(), 3));

        let report = build_report(&tree, &tree, &tree);
        assert_eq!(report.synced.len(), 1);
        assert!(report.only_a.is_empty());
        assert!(report.only_b.is_empty());
        assert!(report.drifted.is_empty());
    }

    #[test]
    fn build_report_only_in_a() {
        let mut tree_a = BTreeMap::new();
        tree_a.insert("file.txt".to_string(), ("abc".to_string(), 3));
        let tree_b = BTreeMap::new();
        let tree_t = BTreeMap::new();

        let report = build_report(&tree_a, &tree_b, &tree_t);
        assert!(report.synced.is_empty());
        assert_eq!(report.only_a.len(), 1);
    }

    #[test]
    fn build_report_mixed() {
        let mut tree_a = BTreeMap::new();
        let mut tree_b = BTreeMap::new();
        let tree_t = BTreeMap::new();

        tree_a.insert("shared.txt".to_string(), ("abc".to_string(), 3));
        tree_b.insert("shared.txt".to_string(), ("abc".to_string(), 3));

        tree_a.insert("a_only.txt".to_string(), ("aaa".to_string(), 3));
        tree_b.insert("b_only.txt".to_string(), ("bbb".to_string(), 3));

        let report = build_report(&tree_a, &tree_b, &tree_t);
        assert_eq!(report.synced.len(), 1);
        assert_eq!(report.only_a.len(), 1);
        assert_eq!(report.only_b.len(), 1);
    }

    // ── format_size tests ────────────────────────────────────────────

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(100), "100B");
        assert_eq!(format_size(0), "0B");
    }

    #[test]
    fn format_size_kb() {
        assert_eq!(format_size(1024), "1.0KB");
        assert_eq!(format_size(2048), "2.0KB");
    }

    #[test]
    fn format_size_mb() {
        assert_eq!(format_size(1024 * 1024), "1.0MB");
    }

    // ── default_ignore tests ─────────────────────────────────────────

    #[test]
    fn default_ignore_includes_defaults() {
        let ignore = default_ignore(&[]);
        assert!(ignore.contains(&".git".to_string()));
        assert!(ignore.contains(&".DS_Store".to_string()));
        assert!(ignore.contains(&"README.md".to_string()));
    }

    #[test]
    fn default_ignore_merges_user_patterns() {
        let ignore = default_ignore(&["*.bak".to_string()]);
        assert!(ignore.contains(&"*.bak".to_string()));
        assert!(ignore.contains(&".git".to_string()));
    }

    #[test]
    fn default_ignore_no_duplicates() {
        let ignore = default_ignore(&[".git".to_string()]);
        let git_count = ignore.iter().filter(|p| *p == ".git").count();
        assert_eq!(git_count, 1);
    }

    // ── is_newer tests ───────────────────────────────────────────────

    #[test]
    fn is_newer_nonexistent_files() {
        let tmp = TempDir::new().unwrap();
        let existing = tmp.path().join("exists.txt");
        let missing = tmp.path().join("missing.txt");

        fs::write(&existing, "content").unwrap();

        assert!(is_newer(&existing, &missing));
        assert!(!is_newer(&missing, &existing));
    }

    // ── Integration: hash_tree + build_report ────────────────────────

    #[test]
    fn integration_hash_and_report() {
        let tmp = TempDir::new().unwrap();

        let a_dir = tmp.path().join("a");
        fs::create_dir_all(&a_dir).unwrap();
        write_file(&a_dir, "shared.txt", "same content");
        write_file(&a_dir, "a_only.txt", "only in A");
        write_file(&a_dir, "drifted.txt", "version A");

        let b_dir = tmp.path().join("b");
        fs::create_dir_all(&b_dir).unwrap();
        write_file(&b_dir, "shared.txt", "same content");
        write_file(&b_dir, "b_only.txt", "only in B");
        write_file(&b_dir, "drifted.txt", "version B");

        let t_dir = tmp.path().join("t");
        fs::create_dir_all(&t_dir).unwrap();
        write_file(&t_dir, "shared.txt", "same content");
        write_file(&t_dir, "drifted.txt", "version A");

        let tree_a = hash_tree(&a_dir, &[]).unwrap();
        let tree_b = hash_tree(&b_dir, &[]).unwrap();
        let tree_t = hash_tree(&t_dir, &[]).unwrap();

        let report = build_report(&tree_a, &tree_b, &tree_t);

        assert_eq!(report.synced.len(), 1); // shared.txt
        assert_eq!(report.only_a.len(), 1); // a_only.txt
        assert_eq!(report.only_b.len(), 1); // b_only.txt
        assert_eq!(report.drifted.len(), 1); // drifted.txt (target matches A)
    }
}
