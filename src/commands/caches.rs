use anyhow::{Context, Result, bail};
use chrono::Utc;
use colored::Colorize;
use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::Path;

use crate::cli::CachesCommand;
use crate::config::{CachesConfig, ConfigFormat};
use crate::state::{BossaState, TrackedSymlink};
use crate::ui;
use crate::ui::format_size;

/// Get home directory with proper error handling
fn home_dir() -> Result<std::path::PathBuf> {
    dirs::home_dir().context("Could not determine home directory")
}

pub fn run(cmd: CachesCommand) -> Result<()> {
    match cmd {
        CachesCommand::Status => status(),
        CachesCommand::Apply { dry_run } => apply(dry_run),
        CachesCommand::Audit => audit(),
        CachesCommand::Doctor => doctor(),
        CachesCommand::Init { force } => init(force),
    }
}

/// Show current cache status
fn status() -> Result<()> {
    ui::header("Cache Status");

    let config = match CachesConfig::load() {
        Ok(c) => c,
        Err(_) => {
            ui::warn("No caches.toml found. Run 'bossa caches init' to create one.");
            return Ok(());
        }
    };

    // Check drive status
    let drive = &config.external_drive;
    if config.is_drive_mounted() {
        println!(
            "  {} {} mounted at {}",
            "✓".green(),
            drive.name.cyan(),
            drive.mount_point
        );

        // Show cache root size
        let cache_root = config.cache_root();
        if cache_root.exists()
            && let Ok(size) = dir_size(&cache_root)
        {
            println!(
                "    Cache root: {} ({})",
                cache_root.display(),
                format_size(size)
            );
        }
    } else {
        println!(
            "  {} {} not mounted (expected at {})",
            "✗".red(),
            drive.name.cyan(),
            drive.mount_point
        );
        return Ok(());
    }

    println!();
    println!("{}", "Symlinks:".bold());

    for symlink in &config.symlinks {
        let source = config.expand_source(&symlink.source);
        let target = config.target_path(&symlink.target);

        let status = check_symlink_status(&source, &target);
        let icon = match &status {
            SymlinkStatus::Valid => "✓".green(),
            SymlinkStatus::NotCreated => "○".yellow(),
            SymlinkStatus::Broken => "✗".red(),
            SymlinkStatus::NotSymlink => "⚠".yellow(),
        };

        let size_str = if target.exists() {
            if let Ok(size) = dir_size(&target) {
                format!(" ({})", format_size(size))
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        println!(
            "  {} {:<25} → {}{}",
            icon,
            symlink.name.cyan(),
            symlink.target,
            size_str.dimmed()
        );

        if matches!(status, SymlinkStatus::Broken) {
            println!("      {} Symlink target missing", "↳".dimmed());
        } else if matches!(status, SymlinkStatus::NotSymlink) {
            println!("      {} Source exists but is not a symlink", "↳".dimmed());
        }
    }

    // Show bazelrc status
    if let Some(bazelrc) = &config.bazelrc {
        println!();
        println!("{}", "Bazel:".bold());
        let bazelrc_path = home_dir()?.join(".bazelrc");
        if bazelrc_path.exists() {
            println!("  {} ~/.bazelrc exists", "✓".green());
            if let Some(output_base) = &bazelrc.output_base {
                let has_output_base = fs::read_to_string(&bazelrc_path)
                    .map(|c| c.contains("--output_base"))
                    .unwrap_or(false);
                if has_output_base {
                    println!("    output_base: {output_base}");
                } else {
                    println!(
                        "  {} output_base not configured in ~/.bazelrc",
                        "○".yellow()
                    );
                }
            }
        } else {
            println!("  {} ~/.bazelrc not found", "○".yellow());
        }
    }

    // Show JetBrains status
    if !config.jetbrains.is_empty() {
        println!();
        println!("{}", "JetBrains:".bold());
        for jb in &config.jetbrains {
            let props_path = home_dir()?
                .join("Library/Application Support/JetBrains")
                .join(&jb.product)
                .join("idea.properties");
            if props_path.exists() {
                println!("  {} {} configured", "✓".green(), jb.product);
            } else {
                println!("  {} {} not configured", "○".yellow(), jb.product);
            }
        }
    }

    Ok(())
}

/// Apply cache configuration (create symlinks, configs)
fn apply(dry_run: bool) -> Result<()> {
    ui::header("Applying Cache Configuration");

    let config = CachesConfig::load()
        .context("No caches.toml found. Run 'bossa caches init' to create one.")?;

    // Check drive is mounted
    if !config.is_drive_mounted() {
        bail!(
            "External drive {} is not mounted at {}",
            config.external_drive.name,
            config.external_drive.mount_point
        );
    }

    let cache_root = config.cache_root();
    if !cache_root.exists() {
        if dry_run {
            println!("  {} Would create {}", "→".cyan(), cache_root.display());
        } else {
            fs::create_dir_all(&cache_root)?;
            ui::success(&format!("Created {}", cache_root.display()));
        }
    }

    // Process symlinks
    println!();
    println!("{}", "Symlinks:".bold());

    for symlink in &config.symlinks {
        let source = config.expand_source(&symlink.source);
        let target = config.target_path(&symlink.target);

        let status = check_symlink_status(&source, &target);

        match status {
            SymlinkStatus::Valid => {
                println!("  {} {} (already configured)", "✓".green(), symlink.name);
            }
            SymlinkStatus::NotCreated => {
                if dry_run {
                    println!(
                        "  {} Would create symlink: {} → {}",
                        "→".cyan(),
                        source.display(),
                        target.display()
                    );
                } else {
                    // Create target directory if needed
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }

                    // Create empty target dir if source doesn't exist
                    if !source.exists() && !target.exists() {
                        fs::create_dir_all(&target)?;
                    }

                    // If source exists and is not a symlink, move it
                    if source.exists() && !source.is_symlink() {
                        println!("    Moving {} to {}", source.display(), target.display());
                        let status = std::process::Command::new("mv")
                            .arg(&source)
                            .arg(&target)
                            .status()
                            .context("Failed to execute mv command")?;
                        if !status.success() {
                            bail!(
                                "Failed to move {} to {}",
                                source.display(),
                                target.display()
                            );
                        }
                    }

                    // Create symlink
                    if let Some(parent) = source.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    unix_fs::symlink(&target, &source)?;
                    // Track in unified symlink inventory
                    track_symlink_in_inventory(&source, &target);
                    println!("  {} {} → {}", "✓".green(), symlink.name, symlink.target);
                }
            }
            SymlinkStatus::Broken => {
                if dry_run {
                    println!(
                        "  {} Would fix broken symlink: {}",
                        "→".cyan(),
                        symlink.name
                    );
                } else {
                    // Remove broken symlink and recreate
                    fs::remove_file(&source)?;
                    if !target.exists() {
                        fs::create_dir_all(&target)?;
                    }
                    unix_fs::symlink(&target, &source)?;
                    // Track in unified symlink inventory
                    track_symlink_in_inventory(&source, &target);
                    println!("  {} {} (fixed)", "✓".green(), symlink.name);
                }
            }
            SymlinkStatus::NotSymlink => {
                if dry_run {
                    println!(
                        "  {} Would move and symlink: {} → {}",
                        "→".cyan(),
                        source.display(),
                        target.display()
                    );
                } else {
                    // Create target parent
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    // Move existing to target
                    let status = std::process::Command::new("mv")
                        .arg(&source)
                        .arg(&target)
                        .status()
                        .context("Failed to execute mv command")?;
                    if !status.success() {
                        bail!(
                            "Failed to move {} to {}",
                            source.display(),
                            target.display()
                        );
                    }
                    // Create symlink
                    unix_fs::symlink(&target, &source)?;
                    // Track in unified symlink inventory
                    track_symlink_in_inventory(&source, &target);
                    println!("  {} {} (migrated)", "✓".green(), symlink.name);
                }
            }
        }
    }

    // Configure bazelrc
    if let Some(bazelrc) = &config.bazelrc {
        println!();
        println!("{}", "Bazel:".bold());
        if let Some(output_base) = &bazelrc.output_base {
            let bazelrc_path = home_dir()?.join(".bazelrc");
            let content = format!(
                "# User-level Bazel configuration\n\
                 # Managed by bossa caches\n\n\
                 # Store output base on external drive to save internal SSD space\n\
                 startup --output_base={output_base}\n"
            );

            // Create output_base dir
            let output_base_path = Path::new(output_base);
            if !output_base_path.exists() {
                if dry_run {
                    println!("  {} Would create {output_base}", "→".cyan());
                } else {
                    fs::create_dir_all(output_base_path)?;
                }
            }

            if dry_run {
                println!("  {} Would write ~/.bazelrc", "→".cyan());
            } else {
                // Check if we should update
                let should_write = if bazelrc_path.exists() {
                    let existing = fs::read_to_string(&bazelrc_path)?;
                    !existing.contains("--output_base")
                } else {
                    true
                };

                if should_write {
                    fs::write(&bazelrc_path, content)?;
                    println!("  {} ~/.bazelrc configured", "✓".green());
                } else {
                    println!("  {} ~/.bazelrc already configured", "✓".green());
                }
            }
        }
    }

    // Configure JetBrains IDEs
    if !config.jetbrains.is_empty() {
        println!();
        println!("{}", "JetBrains:".bold());

        for jb in &config.jetbrains {
            let props_dir = home_dir()?
                .join("Library/Application Support/JetBrains")
                .join(&jb.product);
            let props_path = props_dir.join("idea.properties");

            let mut lines = Vec::new();
            if let Some(system_path) = &jb.system_path {
                lines.push(format!("idea.system.path={system_path}"));
                if !dry_run {
                    fs::create_dir_all(system_path)?;
                }
            }
            if let Some(log_path) = &jb.log_path {
                lines.push(format!("idea.log.path={log_path}"));
                if !dry_run {
                    fs::create_dir_all(log_path)?;
                }
            }

            if !lines.is_empty() {
                let content = format!(
                    "# JetBrains IDE configuration\n\
                     # Managed by bossa caches\n\n\
                     {}\n",
                    lines.join("\n")
                );

                if dry_run {
                    println!("  {} Would write {}", "→".cyan(), props_path.display());
                } else {
                    fs::create_dir_all(&props_dir)?;
                    fs::write(&props_path, content)?;
                    println!("  {} {} configured", "✓".green(), jb.product);
                }
            }
        }
    }

    if dry_run {
        println!();
        ui::warn("Dry run - no changes made");
    } else {
        println!();
        ui::success("Cache configuration applied!");
    }

    Ok(())
}

/// Audit cache configuration for drift
fn audit() -> Result<()> {
    ui::header("Cache Audit");

    let config = match CachesConfig::load() {
        Ok(c) => c,
        Err(_) => {
            ui::warn("No caches.toml found");
            return Ok(());
        }
    };

    let mut issues = Vec::new();

    // Check drive
    if !config.is_drive_mounted() {
        issues.push(format!(
            "External drive {} not mounted",
            config.external_drive.name
        ));
    }

    // Check symlinks
    for symlink in &config.symlinks {
        let source = config.expand_source(&symlink.source);
        let target = config.target_path(&symlink.target);
        let status = check_symlink_status(&source, &target);

        match status {
            SymlinkStatus::Valid => {}
            SymlinkStatus::NotCreated => {
                issues.push(format!("Symlink not created: {}", symlink.name));
            }
            SymlinkStatus::Broken => {
                issues.push(format!("Broken symlink: {}", symlink.name));
            }
            SymlinkStatus::NotSymlink => {
                issues.push(format!("Source exists but not symlinked: {}", symlink.name));
            }
        }
    }

    if issues.is_empty() {
        ui::success("No issues found - all caches properly configured");
    } else {
        println!("{} issue(s) found:\n", issues.len());
        for issue in &issues {
            println!("  {} {}", "✗".red(), issue);
        }
        println!();
        ui::info("Run 'bossa caches apply' to fix these issues");
    }

    Ok(())
}

/// Health check for cache system
fn doctor() -> Result<()> {
    ui::header("Cache Health Check");

    let mut all_ok = true;

    // Check config exists
    if CachesConfig::exists() {
        println!("  {} caches.toml found", "✓".green());
    } else {
        println!("  {} caches.toml not found", "✗".red());
        println!("      Run 'bossa caches init' to create one");
        return Ok(());
    }

    let config = CachesConfig::load()?;

    // Check drive mounted
    if config.is_drive_mounted() {
        println!("  {} {} mounted", "✓".green(), config.external_drive.name);
    } else {
        println!("  {} {} not mounted", "✗".red(), config.external_drive.name);
        all_ok = false;
    }

    // Check cache root exists
    let cache_root = config.cache_root();
    if cache_root.exists() {
        println!("  {} Cache root exists", "✓".green());
    } else {
        println!(
            "  {} Cache root missing: {}",
            "✗".red(),
            cache_root.display()
        );
        all_ok = false;
    }

    // Check symlinks
    let mut symlink_ok = 0;
    let mut symlink_bad = 0;
    for symlink in &config.symlinks {
        let source = config.expand_source(&symlink.source);
        let target = config.target_path(&symlink.target);
        match check_symlink_status(&source, &target) {
            SymlinkStatus::Valid => symlink_ok += 1,
            _ => symlink_bad += 1,
        }
    }

    if symlink_bad == 0 {
        println!("  {} All {} symlinks valid", "✓".green(), symlink_ok);
    } else {
        println!(
            "  {} {}/{} symlinks have issues",
            "✗".red(),
            symlink_bad,
            symlink_ok + symlink_bad
        );
        all_ok = false;
    }

    println!();
    if all_ok {
        ui::success("All cache systems healthy!");
    } else {
        ui::warn("Some issues found. Run 'bossa caches apply' to fix.");
    }

    Ok(())
}

/// Initialize cache configuration
fn init(force: bool) -> Result<()> {
    ui::header("Initialize Cache Configuration");

    let dir = crate::config::config_dir()?;
    let path = dir.join("caches.toml");

    if path.exists() && !force {
        ui::warn(&format!("Config already exists: {}", path.display()));
        ui::info("Use --force to overwrite");
        return Ok(());
    }

    let config = CachesConfig::default_config();
    config.save_as(ConfigFormat::Toml)?;

    ui::success(&format!("Created {}", path.display()));
    println!();
    ui::info("Edit the config file to customize, then run 'bossa caches apply'");

    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

/// Track a symlink in the unified inventory.
/// This is called after successful symlink creation to record it in BossaState.
/// Errors are logged but do not fail the operation (non-critical).
fn track_symlink_in_inventory(source: &Path, target: &Path) {
    if let Ok(mut state) = BossaState::load() {
        state.symlinks.add(TrackedSymlink {
            source: source.to_string_lossy().to_string(),
            target: target.to_string_lossy().to_string(),
            subsystem: "caches".to_string(),
            created_at: Utc::now(),
        });
        if let Err(e) = state.save() {
            log::warn!("Failed to save symlink to state: {e}");
        }
    } else {
        log::warn!("Failed to load state for symlink tracking");
    }
}

#[derive(Debug)]
enum SymlinkStatus {
    Valid,
    NotCreated,
    Broken,
    NotSymlink,
}

fn check_symlink_status(source: &Path, target: &Path) -> SymlinkStatus {
    if !source.exists() && !source.is_symlink() {
        return SymlinkStatus::NotCreated;
    }

    if source.is_symlink() {
        if let Ok(link_target) = fs::read_link(source)
            && link_target == target
            && target.exists()
        {
            return SymlinkStatus::Valid;
        }
        return SymlinkStatus::Broken;
    }

    SymlinkStatus::NotSymlink
}

fn dir_size(path: &Path) -> Result<u64> {
    let path_str = path.to_str().context("Path contains invalid UTF-8")?;
    let output = std::process::Command::new("du")
        .args(["-sk", path_str])
        .output()
        .context("Failed to run du command")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let size_kb: u64 = stdout
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Ok(size_kb * 1024)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_check_symlink_status_not_created() {
        let temp_dir = TempDir::new().unwrap();
        let source = temp_dir.path().join("nonexistent_source");
        let target = temp_dir.path().join("nonexistent_target");

        let status = check_symlink_status(&source, &target);
        assert!(matches!(status, SymlinkStatus::NotCreated));
    }

    #[test]
    fn test_check_symlink_status_valid() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("target_dir");
        let source = temp_dir.path().join("source_link");

        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &source).unwrap();

        let status = check_symlink_status(&source, &target);
        assert!(matches!(status, SymlinkStatus::Valid));
    }

    #[test]
    fn test_check_symlink_status_broken() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("target_that_will_be_removed");
        let source = temp_dir.path().join("source_link");

        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &source).unwrap();
        std::fs::remove_dir(&target).unwrap();

        let status = check_symlink_status(&source, &target);
        assert!(matches!(status, SymlinkStatus::Broken));
    }

    #[test]
    fn test_check_symlink_status_not_symlink() {
        let temp_dir = TempDir::new().unwrap();
        let source = temp_dir.path().join("regular_dir");
        let target = temp_dir.path().join("target_dir");

        std::fs::create_dir(&source).unwrap();

        let status = check_symlink_status(&source, &target);
        assert!(matches!(status, SymlinkStatus::NotSymlink));
    }
}
