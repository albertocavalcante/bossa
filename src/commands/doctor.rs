use anyhow::Result;
use colored::Colorize;
use std::path::Path;

use crate::config::{self, RefsConfig, WorkspacesConfig};
use crate::runner;
use crate::ui;
use crate::Context;

pub fn run(_ctx: &Context) -> Result<()> {
    ui::banner();
    ui::header("System Health Check");

    let mut all_good = true;

    // Check 1: Required commands
    all_good &= check_commands();

    // Check 2: Configuration files
    all_good &= check_configs();

    // Check 3: Directory structure
    all_good &= check_directories();

    // Check 4: Git configuration
    all_good &= check_git();

    // Check 5: T9 drive (if expected)
    all_good &= check_t9();

    // Check 6: Homebrew health
    all_good &= check_brew();

    // Summary
    println!();
    if all_good {
        ui::success("All systems healthy!");
    } else {
        ui::warn("Some issues detected - see above for details");
    }

    Ok(())
}

fn check_commands() -> bool {
    ui::section("Required Commands");

    let commands = [
        ("git", "Version control"),
        ("brew", "Package manager"),
        ("stow", "Symlink manager"),
        ("jq", "JSON processor"),
        ("gh", "GitHub CLI"),
    ];

    let mut all_ok = true;

    for (cmd, desc) in commands {
        if runner::command_exists(cmd) {
            println!("  {} {} - {}", "✓".green(), cmd, desc.dimmed());
        } else {
            println!("  {} {} - {} {}", "✗".red(), cmd, desc, "(missing)".red());
            all_ok = false;
        }
    }

    all_ok
}

fn check_configs() -> bool {
    ui::section("Configuration Files");

    let config_dir = match config::config_dir() {
        Ok(d) => d,
        Err(_) => {
            ui::error("Could not determine config directory");
            return false;
        }
    };

    let configs = [
        ("refs.json", "Reference repositories"),
        ("workspaces.json", "Developer workspaces"),
    ];

    let mut all_ok = true;

    for (file, desc) in configs {
        let path = config_dir.join(file);
        if path.exists() {
            // Try to parse it
            let valid = match file {
                "refs.json" => RefsConfig::load().is_ok(),
                "workspaces.json" => WorkspacesConfig::load().is_ok(),
                _ => true,
            };

            if valid {
                println!("  {} {} - {}", "✓".green(), file, desc.dimmed());
            } else {
                println!(
                    "  {} {} - {} {}",
                    "⚠".yellow(),
                    file,
                    desc,
                    "(invalid JSON)".yellow()
                );
                all_ok = false;
            }
        } else {
            println!(
                "  {} {} - {} {}",
                "○".dimmed(),
                file,
                desc,
                "(not configured)".dimmed()
            );
        }
    }

    // Check Brewfile
    let brewfile = dirs::home_dir().map(|h| h.join("dotfiles/scripts/brew/Brewfile"));
    if let Some(path) = brewfile {
        if path.exists() {
            println!("  {} Brewfile - {}", "✓".green(), "Package list".dimmed());
        } else {
            println!(
                "  {} Brewfile - {} {}",
                "○".dimmed(),
                "Package list",
                "(not found)".dimmed()
            );
        }
    }

    all_ok
}

fn check_directories() -> bool {
    ui::section("Directory Structure");

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error("Could not determine home directory");
            return false;
        }
    };

    let dirs = [
        ("dev/ws", "Workspaces root"),
        ("dev/refs", "Reference repos"),
        ("bin", "User scripts"),
        (".config/workspace-setup", "Bossa config"),
    ];

    let mut all_ok = true;

    for (dir, desc) in dirs {
        let path = home.join(dir);
        let exists = path.exists();
        let is_symlink = path.is_symlink();

        if exists {
            let extra = if is_symlink {
                format!(
                    " -> {}",
                    std::fs::read_link(&path)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                )
                .dimmed()
                .to_string()
            } else {
                String::new()
            };
            println!(
                "  {} ~/{} - {}{}",
                "✓".green(),
                dir,
                desc.dimmed(),
                extra
            );
        } else {
            println!(
                "  {} ~/{} - {} {}",
                "✗".yellow(),
                dir,
                desc,
                "(missing)".yellow()
            );
            all_ok = false;
        }
    }

    all_ok
}

fn check_git() -> bool {
    ui::section("Git Configuration");

    let mut all_ok = true;

    // Check user config
    let user_name = runner::run_capture("git", &["config", "--global", "user.name"]);
    let user_email = runner::run_capture("git", &["config", "--global", "user.email"]);

    match user_name {
        Ok(name) => println!("  {} user.name: {}", "✓".green(), name),
        Err(_) => {
            println!(
                "  {} user.name: {}",
                "✗".red(),
                "(not set)".red()
            );
            all_ok = false;
        }
    }

    match user_email {
        Ok(email) => println!("  {} user.email: {}", "✓".green(), email),
        Err(_) => {
            println!(
                "  {} user.email: {}",
                "✗".red(),
                "(not set)".red()
            );
            all_ok = false;
        }
    }

    // Check signing key
    let signing_key = runner::run_capture("git", &["config", "--global", "user.signingkey"]);
    match signing_key {
        Ok(key) => println!(
            "  {} signing key: {}",
            "✓".green(),
            if key.len() > 20 {
                format!("{}...", &key[..20])
            } else {
                key
            }
        ),
        Err(_) => println!(
            "  {} signing key: {}",
            "○".dimmed(),
            "(not configured)".dimmed()
        ),
    }

    all_ok
}

fn check_t9() -> bool {
    ui::section("T9 External Drive");

    let t9_path = Path::new("/Volumes/T9");

    if t9_path.exists() {
        println!("  {} T9 mounted at /Volumes/T9", "✓".green());

        // Check if refs symlink points to T9
        if let Some(home) = dirs::home_dir() {
            let refs_path = home.join("dev/refs");
            if refs_path.is_symlink() {
                if let Ok(target) = std::fs::read_link(&refs_path) {
                    if target.to_string_lossy().contains("T9") {
                        println!(
                            "  {} refs symlinked to T9",
                            "✓".green()
                        );
                    }
                }
            }
        }

        // Check free space
        // Note: Would need sys-info crate for proper disk space check
        println!(
            "  {} {}",
            "ℹ".blue(),
            "Run 'bossa t9 stats' for detailed info".dimmed()
        );

        true
    } else {
        println!(
            "  {} T9 not mounted {}",
            "○".dimmed(),
            "(optional)".dimmed()
        );
        true // Not a failure - T9 is optional
    }
}

fn check_brew() -> bool {
    ui::section("Homebrew Health");

    if !runner::command_exists("brew") {
        println!("  {} Homebrew not installed", "✗".red());
        return false;
    }

    println!("  {} Homebrew installed", "✓".green());

    // Check for issues
    let doctor_output = runner::run_capture("brew", &["doctor"]);
    match doctor_output {
        Ok(output) => {
            if output.contains("ready to brew") || output.is_empty() {
                println!("  {} No issues detected", "✓".green());
                true
            } else {
                println!(
                    "  {} Some issues detected - run 'brew doctor' for details",
                    "⚠".yellow()
                );
                false
            }
        }
        Err(_) => {
            println!(
                "  {} Could not run brew doctor",
                "⚠".yellow()
            );
            false
        }
    }
}
