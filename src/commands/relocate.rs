//! Relocate command - move directories and update all path references

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::Context as AppContext;
use crate::cli::RelocateCommand;
use crate::paths;
use crate::scanner::{PathReference, ShellScanner};
use crate::state::{BossaState, TrackedSymlink};
use crate::ui;

pub fn run(ctx: &AppContext, cmd: RelocateCommand) -> Result<()> {
    let from_path = paths::expand(&cmd.from);
    let to_path = paths::expand(&cmd.to);

    ui::header("Relocate");
    println!("  From: {}", from_path.display());
    println!("  To:   {}", to_path.display());
    println!();

    // Validate paths
    if !to_path.exists() {
        anyhow::bail!(
            "Destination does not exist: {}\nCreate it first or use --force",
            to_path.display()
        );
    }

    // Scan for references
    ui::header("Scanning for references...");

    let scanner = ShellScanner::new(&from_path);
    let shell_refs = scanner.scan_all()?;

    // Load state to find managed symlinks
    let state = BossaState::load().unwrap_or_default();
    let symlink_refs: Vec<&TrackedSymlink> = state.symlinks.find_by_source_prefix(&from_path);

    // Display findings
    println!();
    if !shell_refs.is_empty() {
        ui::header(&format!("Shell Config References ({})", shell_refs.len()));
        for r in &shell_refs {
            println!(
                "  {}:{} [{}]",
                r.file.display(),
                r.line,
                format!("{:?}", r.ref_type).dimmed()
            );
            println!("    {}", r.content.trim());
            println!(
                "    {} {}",
                "->".cyan(),
                ShellScanner::replace_path(
                    &r.content,
                    from_path.to_string_lossy().as_ref(),
                    to_path.to_string_lossy().as_ref()
                )
                .trim()
            );
            println!();
        }
    }

    if !symlink_refs.is_empty() {
        ui::header(&format!("Managed Symlinks ({})", symlink_refs.len()));
        for s in &symlink_refs {
            let new_source = s.source.replace(
                from_path.to_string_lossy().as_ref(),
                to_path.to_string_lossy().as_ref(),
            );
            println!("  {} -> {}", s.target, s.source);
            println!("    {} {} -> {}", "->".cyan(), s.target, new_source);
            println!();
        }
    }

    let total = shell_refs.len() + symlink_refs.len();
    if total == 0 {
        println!("{}", "No references found.".dimmed());
        return Ok(());
    }

    // Summary
    println!();
    println!(
        "Found {} shell references, {} managed symlinks",
        shell_refs.len(),
        symlink_refs.len()
    );

    if cmd.scan_only {
        println!();
        println!(
            "{}",
            "Scan complete. Use without --scan-only to apply changes.".dimmed()
        );
        return Ok(());
    }

    if cmd.dry_run {
        println!();
        println!("{}", "Dry run - no changes made.".dimmed());
        return Ok(());
    }

    // Confirm
    if !cmd.yes && !ctx.quiet {
        println!();
        print!("Proceed with updates? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Apply changes
    println!();
    ui::header("Applying changes...");

    // Update shell configs
    if cmd.update_configs || !cmd.symlink {
        for r in &shell_refs {
            update_shell_config(r, &from_path, &to_path, !cmd.no_backup)?;
        }
    }

    // Update managed symlinks
    for s in &symlink_refs {
        update_symlink(s, &from_path, &to_path, cmd.dry_run)?;
    }

    // Create fallback symlink if requested
    if cmd.symlink && !from_path.exists() {
        create_fallback_symlink(&from_path, &to_path)?;
    }

    println!();
    println!("{} Relocation complete!", "[ok]".green());

    Ok(())
}

fn update_shell_config(
    reference: &PathReference,
    from: &Path,
    to: &Path,
    backup: bool,
) -> Result<()> {
    let content = fs::read_to_string(&reference.file)
        .with_context(|| format!("Failed to read {}", reference.file.display()))?;

    let new_content = content.replace(
        from.to_string_lossy().as_ref(),
        to.to_string_lossy().as_ref(),
    );

    if backup {
        let backup_path = reference.file.with_extension("bak");
        fs::copy(&reference.file, &backup_path)
            .with_context(|| format!("Failed to backup {}", reference.file.display()))?;
        println!("  {} Backed up {}", "->".dimmed(), backup_path.display());
    }

    fs::write(&reference.file, new_content)
        .with_context(|| format!("Failed to write {}", reference.file.display()))?;

    println!("  {} Updated {}", "[ok]".green(), reference.file.display());

    Ok(())
}

fn update_symlink(symlink: &TrackedSymlink, from: &Path, to: &Path, dry_run: bool) -> Result<()> {
    let target_path = PathBuf::from(&symlink.target);
    let new_source = PathBuf::from(symlink.source.replace(
        from.to_string_lossy().as_ref(),
        to.to_string_lossy().as_ref(),
    ));

    if dry_run {
        println!(
            "  {} Would update {} -> {}",
            "[--]".yellow(),
            target_path.display(),
            new_source.display()
        );
        return Ok(());
    }

    // Remove old symlink and create new one
    if target_path.is_symlink() {
        fs::remove_file(&target_path)?;
        std::os::unix::fs::symlink(&new_source, &target_path)?;
        println!(
            "  {} Updated {} -> {}",
            "[ok]".green(),
            target_path.display(),
            new_source.display()
        );
    }

    Ok(())
}

fn create_fallback_symlink(from: &PathBuf, to: &PathBuf) -> Result<()> {
    // Create parent directory if needed
    if let Some(parent) = from.parent() {
        fs::create_dir_all(parent)?;
    }

    std::os::unix::fs::symlink(to, from).with_context(|| {
        format!(
            "Failed to create symlink {} -> {}",
            from.display(),
            to.display()
        )
    })?;

    println!(
        "  {} Created fallback symlink: {} -> {}",
        "[ok]".green(),
        from.display(),
        to.display()
    );

    Ok(())
}
