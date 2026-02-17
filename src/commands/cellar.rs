//! Homebrew cellar warehousing on external SSD.
//!
//! Mirrors the local Homebrew Cellar to an external drive via rsync,
//! then trims non-essential packages locally to save internal SSD space.
//! Packages can be restored on demand.

use anyhow::{Result, bail};
use colored::Colorize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::Context as AppContext;
use crate::cli::CellarCommand;
use crate::runner;
use crate::schema::{BossaConfig, CellarConfig};
use crate::ui;

/// Standard Homebrew Cellar path on Apple Silicon Macs.
const LOCAL_CELLAR: &str = "/opt/homebrew/Cellar";

/// Standard Homebrew Caskroom path on Apple Silicon Macs.
const LOCAL_CASKROOM: &str = "/opt/homebrew/Caskroom";

pub fn run(ctx: &AppContext, cmd: CellarCommand) -> Result<()> {
    match cmd {
        CellarCommand::Init => init(ctx),
        CellarCommand::Stash { dry_run } => stash(ctx, dry_run),
        CellarCommand::Trim { dry_run } => trim(ctx, dry_run),
        CellarCommand::Restore { package, all } => {
            if all {
                restore_all(ctx)
            } else if let Some(pkg) = package {
                restore(ctx, &pkg)
            } else {
                bail!("Provide a package name or use --all");
            }
        }
        CellarCommand::Sync { dry_run } => sync(ctx, dry_run),
        CellarCommand::Status => status(ctx),
        CellarCommand::Promote { package } => promote(ctx, &package),
        CellarCommand::Demote { package } => demote(ctx, &package),
    }
}

// ============================================================================
// Commands
// ============================================================================

fn init(_ctx: &AppContext) -> Result<()> {
    ui::header("Cellar Init");

    let config = load_validated_config()?;

    let cellar_dir = external_cellar_path(&config);
    let caskroom_dir = external_caskroom_path(&config);

    for dir in [&cellar_dir, &caskroom_dir] {
        if dir.exists() {
            println!("  {} {} (exists)", "✓".green(), dir.display());
        } else {
            std::fs::create_dir_all(dir)?;
            println!("  {} Created {}", "✓".green(), dir.display());
        }
    }

    ui::success("Cellar directory structure ready");
    Ok(())
}

fn stash(_ctx: &AppContext, dry_run: bool) -> Result<()> {
    ui::header("Cellar Stash");

    let config = load_validated_config()?;

    if !Path::new(LOCAL_CELLAR).exists() {
        bail!("Local Cellar not found at {LOCAL_CELLAR}");
    }

    let external = external_cellar_path(&config);
    std::fs::create_dir_all(&external)?;

    rsync_mirror(LOCAL_CELLAR, &external, dry_run)?;

    // Also stash Caskroom metadata
    if Path::new(LOCAL_CASKROOM).exists() {
        let caskroom_ext = external_caskroom_path(&config);
        std::fs::create_dir_all(&caskroom_ext)?;
        println!();
        rsync_mirror(LOCAL_CASKROOM, &caskroom_ext, dry_run)?;
    }

    if dry_run {
        ui::warn("Dry run — no changes made");
    } else {
        ui::success("Cellar stashed to external drive");
    }
    Ok(())
}

fn trim(_ctx: &AppContext, dry_run: bool) -> Result<()> {
    ui::header("Cellar Trim");

    let config = load_validated_config()?;

    let external = external_cellar_path(&config);
    let ext_kegs: BTreeSet<String> = list_kegs(&external).into_iter().collect();
    if ext_kegs.is_empty() {
        bail!(
            "External cellar is empty at {}. Run 'bossa cellar stash' first.",
            external.display()
        );
    }

    let local_kegs = list_kegs(Path::new(LOCAL_CELLAR));
    if local_kegs.is_empty() {
        ui::info("No local kegs found — nothing to trim");
        return Ok(());
    }

    let keep_set = resolve_keep_set(&config)?;

    let candidates: Vec<&String> = local_kegs
        .iter()
        .filter(|pkg| !keep_set.contains(pkg.as_str()))
        .collect();

    if candidates.is_empty() {
        ui::success("All local packages are essential — nothing to trim");
        return Ok(());
    }

    // Safety: verify every candidate exists in external cellar
    let missing: Vec<&str> = candidates
        .iter()
        .filter(|pkg| !ext_kegs.contains(pkg.as_str()))
        .map(|pkg| pkg.as_str())
        .collect();

    if !missing.is_empty() {
        bail!(
            "These packages are not in external cellar (run stash first): {}",
            missing.join(", ")
        );
    }

    let mut total_reclaimed: u64 = 0;

    println!(
        "  {} packages to trim (keeping {} essential)\n",
        candidates.len().to_string().bold(),
        keep_set.len().to_string().green()
    );

    for pkg in &candidates {
        let keg_path = Path::new(LOCAL_CELLAR).join(pkg);
        let size = runner::dir_size(&keg_path).unwrap_or(0);

        if dry_run {
            println!(
                "  {} Would trim {} ({})",
                "→".cyan(),
                pkg,
                ui::format_size(size)
            );
        } else {
            let _ = runner::run("brew", &["unlink", pkg]);
            std::fs::remove_dir_all(&keg_path)?;
            println!(
                "  {} Trimmed {} ({})",
                "✓".green(),
                pkg,
                ui::format_size(size)
            );
        }
        total_reclaimed += size;
    }

    println!();
    if dry_run {
        println!(
            "  Would reclaim {}",
            ui::format_size(total_reclaimed).bold()
        );
        ui::warn("Dry run — no changes made");
    } else {
        ui::success(&format!(
            "Trimmed {} packages, reclaimed {}",
            candidates.len(),
            ui::format_size(total_reclaimed)
        ));
    }

    Ok(())
}

fn restore(_ctx: &AppContext, pkg: &str) -> Result<()> {
    ui::header("Cellar Restore");

    let config = load_validated_config()?;
    let external = external_cellar_path(&config);

    // Resolve deps so we restore the full dependency tree
    let deps = resolve_deps_for_restore(pkg, &external)?;

    for dep in &deps {
        restore_single_keg(dep, &external)?;
    }

    restore_single_keg(pkg, &external)?;

    ui::success(&format!("Restored {pkg} (and {} deps)", deps.len()));
    Ok(())
}

fn restore_all(_ctx: &AppContext) -> Result<()> {
    ui::header("Cellar Restore All");

    let config = load_validated_config()?;
    let external = external_cellar_path(&config);
    let ext_kegs = list_kegs(&external);

    if ext_kegs.is_empty() {
        ui::info("External cellar is empty — nothing to restore");
        return Ok(());
    }

    let local_kegs: BTreeSet<String> = list_kegs(Path::new(LOCAL_CELLAR)).into_iter().collect();
    let mut restored = 0u32;

    for pkg in &ext_kegs {
        if !local_kegs.contains(pkg) {
            restore_single_keg(pkg, &external)?;
            restored += 1;
        }
    }

    if restored == 0 {
        ui::success("All external packages already present locally");
    } else {
        ui::success(&format!("Restored {restored} packages"));
    }
    Ok(())
}

fn sync(ctx: &AppContext, dry_run: bool) -> Result<()> {
    stash(ctx, dry_run)?;
    println!();
    trim(ctx, dry_run)
}

/// Public entry point for non-interactive sync (used by nova).
pub fn sync_for_nova(ctx: &AppContext) -> Result<()> {
    let config = BossaConfig::load()?.cellar;
    if config.path.is_empty() {
        return Ok(());
    }
    sync(ctx, false)
}

fn status(_ctx: &AppContext) -> Result<()> {
    ui::header("Cellar Status");

    let config = BossaConfig::load()?.cellar;

    if config.path.is_empty() {
        ui::warn("Cellar not configured. Add [cellar] section to config.toml.");
        return Ok(());
    }

    let volume = volume_mount_point(&config);
    if !volume.exists() {
        println!(
            "  {} External volume not mounted ({})",
            "✗".red(),
            volume.display()
        );
        return Ok(());
    }

    let external = external_cellar_path(&config);
    let local_kegs: BTreeSet<String> = list_kegs(Path::new(LOCAL_CELLAR)).into_iter().collect();
    let ext_kegs: BTreeSet<String> = list_kegs(&external).into_iter().collect();

    let both: BTreeSet<&String> = local_kegs.intersection(&ext_kegs).collect();
    let local_only: BTreeSet<&String> = local_kegs.difference(&ext_kegs).collect();
    let ext_only: BTreeSet<&String> = ext_kegs.difference(&local_kegs).collect();

    let keep_set = resolve_keep_set(&config).unwrap_or_default();

    let local_size = runner::dir_size(Path::new(LOCAL_CELLAR)).unwrap_or(0);
    let ext_size = runner::dir_size(&external).unwrap_or(0);

    println!(
        "  Local Cellar:    {} ({})",
        LOCAL_CELLAR,
        ui::format_size(local_size)
    );
    println!(
        "  External Cellar: {} ({})",
        external.display(),
        ui::format_size(ext_size)
    );
    println!(
        "  Essential list:  {} packages",
        config.local.len().to_string().green()
    );
    println!(
        "  Keep set (with deps): {} packages",
        keep_set.len().to_string().green()
    );
    println!();

    println!(
        "  {} local-only, {} external-only, {} both",
        local_only.len().to_string().cyan(),
        ext_only.len().to_string().yellow(),
        both.len().to_string().green()
    );
    println!();

    if !local_only.is_empty() {
        println!("{}", "  Local only (not yet stashed):".bold());
        for pkg in &local_only {
            let marker = if keep_set.contains(pkg.as_str()) {
                "★".green()
            } else {
                "○".yellow()
            };
            let size = runner::dir_size(&Path::new(LOCAL_CELLAR).join(pkg)).unwrap_or(0);
            println!(
                "    {} {:<30} {}",
                marker,
                pkg,
                ui::format_size(size).dimmed()
            );
        }
        println!();
    }

    if !ext_only.is_empty() {
        println!("{}", "  External only (trimmed locally):".bold());
        for pkg in &ext_only {
            let size = runner::dir_size(&external.join(pkg)).unwrap_or(0);
            println!(
                "    {} {:<30} {}",
                "●".yellow(),
                pkg,
                ui::format_size(size).dimmed()
            );
        }
        println!();
    }

    Ok(())
}

fn promote(_ctx: &AppContext, pkg: &str) -> Result<()> {
    ui::header("Cellar Promote");

    let mut config = BossaConfig::load()?;

    if config.cellar.local.iter().any(|p| p == pkg) {
        ui::info(&format!("{pkg} is already in the local keep-list"));
    } else {
        config.cellar.local.push(pkg.to_string());
        config.cellar.local.sort();
        config.save()?;
        ui::success(&format!("Added {pkg} to local keep-list"));
    }

    // Restore if available externally and not present locally
    let external = external_cellar_path(&config.cellar);
    let local_path = Path::new(LOCAL_CELLAR).join(pkg);
    if !local_path.exists() && external.join(pkg).exists() {
        restore_single_keg(pkg, &external)?;
        ui::success(&format!("Restored {pkg} from external cellar"));
    }

    Ok(())
}

fn demote(_ctx: &AppContext, pkg: &str) -> Result<()> {
    ui::header("Cellar Demote");

    let mut config = BossaConfig::load()?;

    let before_len = config.cellar.local.len();
    config.cellar.local.retain(|p| p != pkg);

    if config.cellar.local.len() == before_len {
        ui::warn(&format!("{pkg} was not in the local keep-list"));
        return Ok(());
    }

    config.save()?;
    ui::success(&format!("Removed {pkg} from local keep-list"));

    // If external cellar has it backed up, trim locally
    if !config.cellar.path.is_empty() && volume_mount_point(&config.cellar).exists() {
        let external = external_cellar_path(&config.cellar);
        let local_path = Path::new(LOCAL_CELLAR).join(pkg);
        if local_path.exists() && external.join(pkg).exists() {
            let _ = runner::run("brew", &["unlink", pkg]);
            std::fs::remove_dir_all(&local_path)?;
            ui::success(&format!("Trimmed {pkg} from local Cellar"));
        }
    }

    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

/// Load cellar config, validate the path is set, and ensure the volume is mounted.
fn load_validated_config() -> Result<CellarConfig> {
    let config = BossaConfig::load()?.cellar;

    if config.path.is_empty() {
        bail!(
            "Cellar path not configured. Add [cellar] section to config.toml:\n\n\
             [cellar]\n\
             path = \"/Volumes/T9/homebrew\"\n\
             local = [\"git\", \"neovim\"]"
        );
    }

    let mount = volume_mount_point(&config);
    if !mount.exists() {
        bail!("External volume not mounted at {}", mount.display());
    }

    Ok(config)
}

/// Find the volume mount point from a cellar path.
///
/// For macOS paths under `/Volumes/X/...`, returns `/Volumes/X`.
/// For other paths (e.g., `/mnt/external/...`), falls back to the cellar
/// path's parent directory.
fn volume_mount_point(config: &CellarConfig) -> PathBuf {
    let path = PathBuf::from(&config.path);
    path.ancestors()
        .find(|p| p.parent() == Some(Path::new("/Volumes")))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            path.parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| path.clone())
        })
}

fn external_cellar_path(config: &CellarConfig) -> PathBuf {
    PathBuf::from(&config.path).join("Cellar")
}

fn external_caskroom_path(config: &CellarConfig) -> PathBuf {
    PathBuf::from(&config.path).join("Caskroom")
}

/// Run rsync as an exact mirror (`-a --delete`) from `src_dir` to `dst_dir`.
fn rsync_mirror(src_dir: &str, dst_dir: &Path, dry_run: bool) -> Result<()> {
    let src = format!("{src_dir}/");
    let dst = format!("{}/", dst_dir.display());

    let mut args = vec!["-a", "--delete", "--info=progress2"];
    if dry_run {
        args.push("--dry-run");
    }
    args.push(&src);
    args.push(&dst);

    println!("  rsync {} → {}", src.dimmed(), dst.dimmed());
    if dry_run {
        println!("  {}", "(dry run)".yellow());
    }

    let status = runner::run("rsync", &args)?;
    if !status.success() {
        bail!("rsync failed with exit code {status}");
    }
    Ok(())
}

/// List formula names (directories) under a Cellar path.
fn list_kegs(cellar: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(cellar) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Resolve the full keep-local set: user list + transitive deps of each.
fn resolve_keep_set(config: &CellarConfig) -> Result<BTreeSet<String>> {
    let mut keep = BTreeSet::new();

    for pkg in &config.local {
        keep.insert(pkg.clone());
        if let Ok(deps) = runner::run_capture("brew", &["deps", "--installed", pkg]) {
            for line in deps.lines() {
                let dep = line.trim();
                if !dep.is_empty() {
                    keep.insert(dep.to_string());
                }
            }
        }
    }

    Ok(keep)
}

/// Resolve dependencies for a package to restore from the external cellar.
/// Only returns deps that exist in the external cellar and are missing locally.
fn resolve_deps_for_restore(pkg: &str, external_cellar: &Path) -> Result<Vec<String>> {
    let ext_kegs: BTreeSet<String> = list_kegs(external_cellar).into_iter().collect();
    let local_kegs: BTreeSet<String> = list_kegs(Path::new(LOCAL_CELLAR)).into_iter().collect();

    let deps = match runner::run_capture("brew", &["deps", pkg]) {
        Ok(output) => output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|d| !d.is_empty())
            .filter(|d| ext_kegs.contains(d) && !local_kegs.contains(d))
            .collect(),
        Err(_) => Vec::new(),
    };

    Ok(deps)
}

/// Restore a single keg from external cellar to local, then `brew link`.
fn restore_single_keg(pkg: &str, external_cellar: &Path) -> Result<()> {
    let src_keg = external_cellar.join(pkg);
    if !src_keg.exists() {
        bail!("Package {pkg} not found in external cellar");
    }

    let dst_keg = Path::new(LOCAL_CELLAR).join(pkg);
    if dst_keg.exists() {
        return Ok(());
    }

    let src = format!("{}/", src_keg.display());
    let dst = format!("{}/", dst_keg.display());

    std::fs::create_dir_all(&dst_keg)?;

    let status = runner::run("rsync", &["-a", "--info=progress2", &src, &dst])?;
    if !status.success() {
        bail!("rsync failed restoring {pkg}");
    }

    let link_status = runner::run("brew", &["link", "--overwrite", pkg])?;
    if !link_status.success() {
        ui::warn(&format!("brew link {pkg} failed — may need manual linking"));
    }

    println!("  {} Restored {}", "✓".green(), pkg);
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_kegs_nonexistent_path() {
        let kegs = list_kegs(Path::new("/nonexistent/cellar/path"));
        assert!(kegs.is_empty());
    }

    #[test]
    fn test_list_kegs_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let kegs = list_kegs(tmp.path());
        assert!(kegs.is_empty());
    }

    #[test]
    fn test_list_kegs_with_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("git")).unwrap();
        std::fs::create_dir(tmp.path().join("neovim")).unwrap();
        std::fs::write(tmp.path().join("some_file"), "not a dir").unwrap();

        let kegs = list_kegs(tmp.path());
        assert_eq!(kegs, vec!["git", "neovim"]);
    }

    #[test]
    fn test_list_kegs_sorted() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("zsh")).unwrap();
        std::fs::create_dir(tmp.path().join("awk")).unwrap();
        std::fs::create_dir(tmp.path().join("make")).unwrap();

        let kegs = list_kegs(tmp.path());
        assert_eq!(kegs, vec!["awk", "make", "zsh"]);
    }

    #[test]
    fn test_list_kegs_ignores_hidden_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("git")).unwrap();
        std::fs::create_dir(tmp.path().join(".hidden")).unwrap();

        let kegs = list_kegs(tmp.path());
        // .hidden is still a directory, so it will appear — this tests that
        // list_kegs returns all directories without filtering hidden ones.
        assert_eq!(kegs, vec![".hidden", "git"]);
    }

    #[test]
    fn test_external_cellar_path() {
        let config = CellarConfig {
            path: "/Volumes/T9/homebrew".to_string(),
            local: vec![],
        };
        assert_eq!(
            external_cellar_path(&config),
            PathBuf::from("/Volumes/T9/homebrew/Cellar")
        );
    }

    #[test]
    fn test_external_caskroom_path() {
        let config = CellarConfig {
            path: "/Volumes/T9/homebrew".to_string(),
            local: vec![],
        };
        assert_eq!(
            external_caskroom_path(&config),
            PathBuf::from("/Volumes/T9/homebrew/Caskroom")
        );
    }

    #[test]
    fn test_volume_mount_point_standard() {
        let config = CellarConfig {
            path: "/Volumes/T9/homebrew".to_string(),
            local: vec![],
        };
        assert_eq!(volume_mount_point(&config), PathBuf::from("/Volumes/T9"));
    }

    #[test]
    fn test_volume_mount_point_deep() {
        let config = CellarConfig {
            path: "/Volumes/MyDrive/deep/path/homebrew".to_string(),
            local: vec![],
        };
        assert_eq!(
            volume_mount_point(&config),
            PathBuf::from("/Volumes/MyDrive")
        );
    }

    #[test]
    fn test_volume_mount_point_non_volumes_path() {
        let config = CellarConfig {
            path: "/mnt/external/homebrew".to_string(),
            local: vec![],
        };
        // Falls back to parent directory
        assert_eq!(volume_mount_point(&config), PathBuf::from("/mnt/external"));
    }

    #[test]
    fn test_volume_mount_point_root_path() {
        let config = CellarConfig {
            path: "/homebrew".to_string(),
            local: vec![],
        };
        assert_eq!(volume_mount_point(&config), PathBuf::from("/"));
    }

    #[test]
    fn test_cellar_config_default() {
        let config = CellarConfig::default();
        assert!(config.path.is_empty());
        assert!(config.local.is_empty());
    }

    #[test]
    fn test_cellar_config_deserialize_empty_toml() {
        let config: CellarConfig = toml::from_str("").unwrap();
        assert!(config.path.is_empty());
        assert!(config.local.is_empty());
    }

    #[test]
    fn test_cellar_config_deserialize_full() {
        let toml_str = r#"
path = "/Volumes/T9/homebrew"
local = ["git", "neovim", "ripgrep"]
"#;
        let config: CellarConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.path, "/Volumes/T9/homebrew");
        assert_eq!(config.local, vec!["git", "neovim", "ripgrep"]);
    }
}
