use anyhow::Result;
use colored::Colorize;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::cli::{NovaArgs, NovaStage};
use crate::progress::{self, StageProgress};
use crate::runner;
use crate::ui;
use crate::Context;

pub fn run(ctx: &Context, args: NovaArgs) -> Result<()> {
    // Handle --list-stages
    if args.list_stages {
        list_stages();
        return Ok(());
    }

    ui::banner();
    ui::header("Nova - Full System Bootstrap");

    // Determine which stages to run
    let stages_to_run = determine_stages(&args)?;

    if stages_to_run.is_empty() {
        ui::warn("No stages to run");
        return Ok(());
    }

    // Show what we're going to do
    println!();
    ui::info(&format!("Running {} stages:", stages_to_run.len()));
    for stage in &stages_to_run {
        println!("  {} {}", "→".cyan(), stage.name());
    }
    println!();

    if args.dry_run {
        ui::warn("Dry run - no changes will be made");
        println!();
        for stage in &stages_to_run {
            println!(
                "  {} {} - {}",
                "[DRY]".yellow(),
                stage.name(),
                stage.description()
            );
        }
        return Ok(());
    }

    // Get dotfiles directory
    let dotfiles_dir = get_dotfiles_dir()?;

    // Run stages
    let mut progress = StageProgress::new(stages_to_run.len());
    let mut failed_stages: Vec<&NovaStage> = Vec::new();

    for stage in &stages_to_run {
        let pb = progress.next(stage.description());

        let result = run_stage(ctx, stage, &dotfiles_dir);

        match result {
            Ok(()) => {
                progress::finish_success(&pb, &format!("{} complete", stage.name()));
            }
            Err(e) => {
                progress::finish_error(&pb, &format!("{} failed: {}", stage.name(), e));
                failed_stages.push(stage);
            }
        }
    }

    // Summary
    println!();
    println!("{}", "═".repeat(60).dimmed());

    if failed_stages.is_empty() {
        ui::success("Bootstrap complete! Restart your terminal to apply all changes.");
    } else {
        ui::warn(&format!(
            "Bootstrap completed with {} failures:",
            failed_stages.len()
        ));
        for stage in &failed_stages {
            println!("  {} {}", "✗".red(), stage.name());
        }
        println!();
        ui::info("You can re-run failed stages with:");
        ui::dim(&format!(
            "  bossa nova --only={}",
            failed_stages
                .iter()
                .map(|s| s.name())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }

    Ok(())
}

fn list_stages() {
    println!();
    println!("{}", "Available Nova Stages".bold());
    println!("{}", "─".repeat(50).dimmed());
    println!();

    for stage in NovaStage::all() {
        println!(
            "  {:<15} {}",
            stage.name().cyan(),
            stage.description().dimmed()
        );
    }

    println!();
    println!("{}", "Usage Examples".bold());
    println!("{}", "─".repeat(50).dimmed());
    println!();
    println!("  {} Run all stages", "bossa nova".cyan());
    println!(
        "  {} Skip specific stages",
        "bossa nova --skip=brew,pnpm".cyan()
    );
    println!(
        "  {} Run only specific stages",
        "bossa nova --only=stow,refs".cyan()
    );
    println!(
        "  {} Preview without changes",
        "bossa nova --dry-run".cyan()
    );
    println!();
}

fn determine_stages(args: &NovaArgs) -> Result<Vec<NovaStage>> {
    let all_stages: Vec<NovaStage> = NovaStage::all().to_vec();

    // If --only is specified, use only those
    if let Some(only) = &args.only {
        let only_set: HashSet<_> = only.split(',').map(|s| s.trim()).collect();
        let stages: Vec<NovaStage> = all_stages
            .into_iter()
            .filter(|s| only_set.contains(s.name()))
            .collect();

        // Warn about unknown stages
        for name in &only_set {
            if NovaStage::from_name(name).is_none() {
                ui::warn(&format!("Unknown stage: {}", name));
            }
        }

        return Ok(stages);
    }

    // If --skip is specified, filter those out
    if let Some(skip) = &args.skip {
        let skip_set: HashSet<_> = skip.split(',').map(|s| s.trim()).collect();

        // Warn about unknown stages
        for name in &skip_set {
            if NovaStage::from_name(name).is_none() {
                ui::warn(&format!("Unknown stage to skip: {}", name));
            }
        }

        let stages: Vec<NovaStage> = all_stages
            .into_iter()
            .filter(|s| !skip_set.contains(s.name()))
            .collect();

        return Ok(stages);
    }

    // Default: all stages
    Ok(all_stages)
}

fn get_dotfiles_dir() -> Result<PathBuf> {
    // Try common locations
    let candidates = [
        dirs::home_dir().map(|h| h.join("dotfiles")),
        dirs::home_dir().map(|h| h.join(".dotfiles")),
        dirs::home_dir().map(|h| h.join("dev/ws/utils/dotfiles")),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() && candidate.join("install.sh").exists() {
            return Ok(candidate);
        }
    }

    anyhow::bail!(
        "Could not find dotfiles directory. Expected ~/dotfiles or ~/dev/ws/utils/dotfiles"
    )
}

fn run_stage(ctx: &Context, stage: &NovaStage, dotfiles_dir: &PathBuf) -> Result<()> {
    match stage {
        NovaStage::Defaults => {
            let script = dotfiles_dir.join("scripts/macos/setup-defaults.sh");
            if script.exists() {
                runner::run("bash", &[script.to_str().unwrap()])?;
            }
            Ok(())
        }

        NovaStage::Terminal => {
            let script = dotfiles_dir.join("scripts/macos/setup-terminal.sh");
            if script.exists() {
                runner::run("bash", &[script.to_str().unwrap()])?;
            }
            Ok(())
        }

        NovaStage::GitSigning => {
            // Copy allowed_signers if needed
            let src = dotfiles_dir.join("git/allowed_signers");
            if src.exists() {
                let ssh_dir = dirs::home_dir().unwrap().join(".ssh");
                std::fs::create_dir_all(&ssh_dir)?;

                let dest = ssh_dir.join("allowed_signers");
                if !dest.exists() {
                    std::fs::copy(&src, &dest)?;
                }
            }
            Ok(())
        }

        NovaStage::Homebrew => {
            if !runner::command_exists("brew") {
                ui::info("Installing Homebrew...");
                runner::run(
                    "bash",
                    &[
                        "-c",
                        r#"curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh | bash"#,
                    ],
                )?;

                // Setup PATH
                if cfg!(target_arch = "aarch64") {
                    runner::run("bash", &["-c", "eval $(/opt/homebrew/bin/brew shellenv)"])?;
                }
            }
            Ok(())
        }

        NovaStage::Bash => {
            // Check if we need modern bash
            let version = runner::run_capture("bash", &["--version"])?;
            if version.contains("version 3.") {
                runner::run("brew", &["install", "bash"])?;
            }
            Ok(())
        }

        NovaStage::Essential => {
            let script = dotfiles_dir.join("scripts/brew/brew-manager.sh");
            if script.exists() {
                runner::run(script.to_str().unwrap(), &["apply-essential"])?;
            }
            Ok(())
        }

        NovaStage::Brew => {
            let script = dotfiles_dir.join("scripts/brew/brew-manager.sh");
            if script.exists() {
                runner::run(script.to_str().unwrap(), &["apply"])?;
            }
            Ok(())
        }

        NovaStage::Pnpm => {
            if runner::command_exists("pnpm") {
                let script = dotfiles_dir.join("scripts/pnpm/pnpm-manager.sh");
                if script.exists() {
                    runner::run(script.to_str().unwrap(), &["apply"])?;
                }
            } else if !ctx.quiet {
                ui::warn("pnpm not available, skipping node packages");
            }
            Ok(())
        }

        NovaStage::Dock => {
            let script = dotfiles_dir.join("scripts/dock/setup-dock.sh");
            if script.exists() {
                runner::run("bash", &[script.to_str().unwrap()])?;
            }
            Ok(())
        }

        NovaStage::Ecosystem => {
            let script = dotfiles_dir.join("scripts/ecosystem/setup-ecosystem.sh");
            if script.exists() {
                runner::run("bash", &[script.to_str().unwrap()])?;
            }
            Ok(())
        }

        NovaStage::Handlers => {
            let script = dotfiles_dir.join("scripts/macos/setup-handlers.sh");
            if script.exists() {
                runner::run("bash", &[script.to_str().unwrap()])?;
            }
            Ok(())
        }

        NovaStage::Stow => {
            if !runner::command_exists("stow") {
                runner::run("brew", &["install", "stow"])?;
            }

            // Find stow packages (directories that aren't ignored)
            let ignore_dirs = ["scripts", "config", "private", "mcp", ".git", ".github", "tools"];

            let mut stow_dirs = Vec::new();
            for entry in std::fs::read_dir(dotfiles_dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if !ignore_dirs.contains(&name.as_str()) && !name.starts_with('.') {
                    stow_dirs.push(name);
                }
            }

            if !stow_dirs.is_empty() {
                let home = dirs::home_dir().unwrap();
                let mut args = vec!["--restow", "--target", home.to_str().unwrap()];
                args.extend(stow_dirs.iter().map(|s| s.as_str()));

                // Change to dotfiles dir for stow
                std::env::set_current_dir(dotfiles_dir)?;
                runner::run("stow", &args)?;
            }

            Ok(())
        }

        NovaStage::Mcp => {
            let script = dotfiles_dir.join("scripts/mcp/setup-mcp.py");
            if script.exists() {
                runner::run("uv", &["run", script.to_str().unwrap()])?;
            }
            Ok(())
        }

        NovaStage::Refs => {
            // Use bossa's own refs sync
            runner::run_script("refs-setup", &["sync"])?;
            Ok(())
        }

        NovaStage::Workspaces => {
            // Use bossa's own workspace sync
            runner::run_script("workspace-setup", &["sync"])?;
            Ok(())
        }
    }
}
