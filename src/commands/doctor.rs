use anyhow::Result;
use colored::Colorize;
use std::path::Path;

use crate::Context;
use crate::config;
use crate::runner;
use crate::schema::BossaConfig;
use crate::ui;

struct Issue {
    category: &'static str,
    summary: String,
    detail: Option<String>,
    fix: Option<String>,
    fix_cmd: Option<String>,
}

pub fn run(_ctx: &Context) -> Result<()> {
    ui::banner();
    ui::header("System Health Check");

    let mut issues: Vec<Issue> = Vec::new();

    // Check 1: Required commands
    check_commands(&mut issues);

    // Check 2: Configuration files
    check_configs(&mut issues);

    // Check 3: Directory structure
    check_directories(&mut issues);

    // Check 4: Git configuration
    check_git(&mut issues);

    // Check 5: T9 drive (if expected)
    check_t9(&mut issues);

    // Check 6: Homebrew health
    check_brew(&mut issues);

    // Summary
    println!();
    if issues.is_empty() {
        ui::success("All systems healthy!");
    } else {
        print_issue_summary(&issues);
    }

    Ok(())
}

fn print_issue_summary(issues: &[Issue]) {
    let count = issues.len();
    let label = if count == 1 { "Issue" } else { "Issues" };
    ui::header(&format!("{count} {label} Found"));

    for (i, issue) in issues.iter().enumerate() {
        let num = i + 1;
        println!(
            "  {}  {} {}",
            format!("{num}.").bold(),
            issue.summary,
            format!("[{}]", issue.category).dimmed()
        );
        if let Some(detail) = &issue.detail {
            for line in detail.lines() {
                println!("      {}", line.dimmed());
            }
        }
        if let Some(fix) = &issue.fix {
            println!("      {} {}", "Fix:".cyan(), fix);
        }
        if let Some(cmd) = &issue.fix_cmd {
            println!("      {} {}", "$".dimmed(), cmd.bold());
        }
        println!();
    }

    // Collect all fix commands into a quick-fix block
    let fix_cmds: Vec<&str> = issues.iter().filter_map(|i| i.fix_cmd.as_deref()).collect();

    if !fix_cmds.is_empty() {
        ui::section("Quick Fixes");
        println!(
            "  {}",
            "Run these commands to resolve the issues above:".dimmed()
        );
        println!();
        for cmd in &fix_cmds {
            println!("    {}", cmd.bold());
        }
    }
}

fn check_commands(issues: &mut Vec<Issue>) {
    ui::section("Required Commands");

    let commands = [
        ("git", "Version control", "brew install git"),
        (
            "brew",
            "Package manager",
            "Visit https://brew.sh for install instructions",
        ),
        ("stow", "Symlink manager", "brew install stow"),
        ("jq", "JSON processor", "brew install jq"),
        ("gh", "GitHub CLI", "brew install gh"),
    ];

    for (cmd, desc, install_hint) in commands {
        if runner::command_exists(cmd) {
            println!("  {} {} - {}", "✓".green(), cmd, desc.dimmed());
        } else {
            println!("  {} {} - {} {}", "✗".red(), cmd, desc, "(missing)".red());
            issues.push(Issue {
                category: "Required Commands",
                summary: format!("{cmd} is not installed"),
                detail: Some(format!("{desc} — required for bossa to function")),
                fix: Some(format!("Install {cmd}")),
                fix_cmd: Some(install_hint.to_string()),
            });
        }
    }
}

fn check_configs(issues: &mut Vec<Issue>) {
    ui::section("Configuration Files");

    let config_dir = match config::config_dir() {
        Ok(d) => d,
        Err(e) => {
            ui::error("Could not determine config directory");
            issues.push(Issue {
                category: "Configuration Files",
                summary: "Could not determine config directory".into(),
                detail: Some(format!("{e}")),
                fix: Some("Ensure $HOME is set or set BOSSA_CONFIG_DIR".into()),
                fix_cmd: None,
            });
            return;
        }
    };

    // Check for unified config file (new format)
    let config_path = config_dir.join("config.toml");
    let config_json_path = config_dir.join("config.json");

    if config_path.exists() || config_json_path.exists() {
        let file_name = if config_path.exists() {
            "config.toml"
        } else {
            "config.json"
        };
        let config_file = config_dir.join(file_name);
        let result = config::load_config::<BossaConfig>(&config_dir, "config");

        match result {
            Ok((cfg, _)) => {
                // Config parsed — now run semantic validation
                match cfg.validate() {
                    Ok(()) => {
                        println!(
                            "  {} {} - {}",
                            "✓".green(),
                            file_name,
                            "Unified bossa config".dimmed()
                        );
                    }
                    Err(e) => {
                        let reason = format!("{e:#}");
                        println!(
                            "  {} {} - Unified bossa config {}",
                            "⚠".yellow(),
                            file_name,
                            format!("(validation error: {reason})").yellow()
                        );
                        issues.push(Issue {
                            category: "Configuration Files",
                            summary: format!("{file_name} has validation error: {reason}"),
                            detail: Some(format!("{e:#}")),
                            fix: Some(format!("Edit {} and fix the issue", config_file.display())),
                            fix_cmd: Some(format!("$EDITOR {}", config_file.display())),
                        });
                    }
                }
            }
            Err(e) => {
                // Show the full error chain (includes line/column from TOML parser)
                let root_cause = format!("{:#}", e.root_cause());
                println!(
                    "  {} {} - Unified bossa config {}",
                    "⚠".yellow(),
                    file_name,
                    format!("(parse error: {root_cause})").yellow()
                );
                issues.push(Issue {
                    category: "Configuration Files",
                    summary: format!("{file_name} has invalid format"),
                    detail: Some(format!("{e:#}")),
                    fix: Some(format!(
                        "Edit {} and fix the syntax error",
                        config_file.display()
                    )),
                    fix_cmd: Some(format!("$EDITOR {}", config_file.display())),
                });
            }
        }
    } else {
        println!(
            "  {} config.toml - Unified bossa config {}",
            "○".dimmed(),
            "(not configured)".dimmed()
        );
    }

    // Check legacy configs (for migration)
    if let Ok(legacy_dir) = config::legacy_config_dir() {
        let legacy_refs = legacy_dir.join("refs.json");
        let legacy_ws = legacy_dir.join("workspaces.json");

        if legacy_refs.exists() || legacy_ws.exists() {
            println!(
                "  {} Legacy configs found - run 'bossa migrate' to upgrade",
                "ℹ".blue()
            );
            issues.push(Issue {
                category: "Configuration Files",
                summary: "Legacy config files should be migrated".into(),
                detail: Some(format!("Found in {}", legacy_dir.display())),
                fix: Some("Migrate legacy configs to the unified format".into()),
                fix_cmd: Some("bossa migrate".into()),
            });
        }
    }

    // Check Brewfile
    let brewfile = dirs::home_dir().map(|h| h.join("dotfiles/scripts/brew/Brewfile"));
    if let Some(path) = brewfile {
        if path.exists() {
            println!("  {} Brewfile - {}", "✓".green(), "Package list".dimmed());
        } else {
            println!(
                "  {} Brewfile - Package list {}",
                "○".dimmed(),
                "(not found)".dimmed()
            );
        }
    }
}

fn check_directories(issues: &mut Vec<Issue>) {
    ui::section("Directory Structure");

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error("Could not determine home directory");
            issues.push(Issue {
                category: "Directory Structure",
                summary: "Could not determine home directory".into(),
                detail: None,
                fix: Some("Ensure $HOME is set".into()),
                fix_cmd: None,
            });
            return;
        }
    };

    let dirs = [
        ("dev/ws", "Workspaces root"),
        ("dev/refs", "Reference repos"),
        ("bin", "User scripts"),
        (".config/bossa", "Bossa config"),
    ];

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
            println!("  {} ~/{} - {}{}", "✓".green(), dir, desc.dimmed(), extra);
        } else {
            println!(
                "  {} ~/{} - {} {}",
                "✗".yellow(),
                dir,
                desc,
                "(missing)".yellow()
            );
            issues.push(Issue {
                category: "Directory Structure",
                summary: format!("~/{dir} directory is missing"),
                detail: Some(format!("{desc} — expected at {}", path.display())),
                fix: Some("Create the directory".into()),
                fix_cmd: Some(format!("mkdir -p {}", path.display())),
            });
        }
    }
}

fn check_git(issues: &mut Vec<Issue>) {
    ui::section("Git Configuration");

    // Check user config
    let user_name = runner::run_capture("git", &["config", "--global", "user.name"]);
    let user_email = runner::run_capture("git", &["config", "--global", "user.email"]);

    match user_name {
        Ok(name) => println!("  {} user.name: {}", "✓".green(), name),
        Err(_) => {
            println!("  {} user.name: {}", "✗".red(), "(not set)".red());
            issues.push(Issue {
                category: "Git Configuration",
                summary: "Git user.name is not set".into(),
                detail: Some("Required for commit attribution".into()),
                fix: Some("Set your Git display name".into()),
                fix_cmd: Some("git config --global user.name \"Your Name\"".into()),
            });
        }
    }

    match user_email {
        Ok(email) => println!("  {} user.email: {}", "✓".green(), email),
        Err(_) => {
            println!("  {} user.email: {}", "✗".red(), "(not set)".red());
            issues.push(Issue {
                category: "Git Configuration",
                summary: "Git user.email is not set".into(),
                detail: Some("Required for commit attribution".into()),
                fix: Some("Set your Git email address".into()),
                fix_cmd: Some("git config --global user.email \"you@example.com\"".into()),
            });
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
}

fn check_t9(_issues: &mut Vec<Issue>) {
    ui::section("T9 External Drive");

    let t9_path = Path::new("/Volumes/T9");

    if t9_path.exists() {
        println!("  {} T9 mounted at /Volumes/T9", "✓".green());

        // Check if refs symlink points to T9
        if let Some(home) = dirs::home_dir() {
            let refs_path = home.join("dev/refs");
            if refs_path.is_symlink()
                && let Ok(target) = std::fs::read_link(&refs_path)
                && target.to_string_lossy().contains("T9")
            {
                println!("  {} refs symlinked to T9", "✓".green());
            }
        }

        // Check free space
        // Note: Would need sys-info crate for proper disk space check
        println!(
            "  {} {}",
            "ℹ".blue(),
            "Run 'bossa t9 stats' for detailed info".dimmed()
        );
    } else {
        println!(
            "  {} T9 not mounted {}",
            "○".dimmed(),
            "(optional)".dimmed()
        );
        // Not an issue - T9 is optional
    }

    // T9 is optional, no issues to report
}

fn check_brew(issues: &mut Vec<Issue>) {
    ui::section("Homebrew Health");

    if !runner::command_exists("brew") {
        println!("  {} Homebrew not installed", "✗".red());
        // Already reported in check_commands if brew is missing
        return;
    }

    println!("  {} Homebrew installed", "✓".green());

    // brew doctor exits non-zero when there are warnings, so combine stdout+stderr
    let doctor_output = runner::run_capture("brew", &["doctor"]);
    let output = match doctor_output {
        Ok(o) => o,
        Err(e) => format!("{e}"),
    };

    if output.contains("ready to brew") || output.is_empty() {
        println!("  {} No issues detected", "✓".green());
        return;
    }

    // Parse warnings into individual items
    let warnings = parse_brew_warnings(&output);

    if warnings.is_empty() {
        // Unparseable output — fall back to generic issue
        println!("  {} Some issues detected", "⚠".yellow());
        issues.push(Issue {
            category: "Homebrew Health",
            summary: "Homebrew reported issues".into(),
            detail: Some(first_lines(&output, 5)),
            fix: Some("Run brew doctor for full output".into()),
            fix_cmd: Some("brew doctor".into()),
        });
        return;
    }

    for warning in &warnings {
        println!("  {} {}", "⚠".yellow(), warning.summary);
    }

    for warning in warnings {
        issues.push(Issue {
            category: "Homebrew Health",
            summary: warning.summary,
            detail: warning.detail,
            fix: warning.fix,
            fix_cmd: warning.fix_cmd,
        });
    }
}

struct BrewWarning {
    summary: String,
    detail: Option<String>,
    fix: Option<String>,
    fix_cmd: Option<String>,
}

fn parse_brew_warnings(output: &str) -> Vec<BrewWarning> {
    let mut warnings = Vec::new();

    // Split on "Warning:" boundaries
    for chunk in output.split("Warning: ").skip(1) {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }

        let first_line = chunk.lines().next().unwrap_or("").trim();

        if first_line.contains("deprecated or disabled") {
            // Extract formula names
            let formulae: Vec<&str> = chunk
                .lines()
                .filter(|l| l.starts_with("  "))
                .map(str::trim)
                .collect();
            if formulae.is_empty() {
                continue;
            }
            let list = formulae.join(", ");
            warnings.push(BrewWarning {
                summary: format!("Deprecated/disabled formulae: {list}"),
                detail: Some("These formulae are no longer maintained; find replacements".into()),
                fix: Some(format!("Uninstall or replace: {list}")),
                fix_cmd: Some(format!("brew uninstall {}", formulae.join(" "))),
            });
        } else if first_line.contains("unlinked kegs") {
            let kegs: Vec<&str> = chunk
                .lines()
                .filter(|l| l.starts_with("  "))
                .map(str::trim)
                .collect();
            if kegs.is_empty() {
                continue;
            }
            let list = kegs.join(", ");
            warnings.push(BrewWarning {
                summary: format!("Unlinked kegs: {list}"),
                detail: Some("Unlinked kegs can cause build failures for dependents".into()),
                fix: Some(format!("Link the kegs: {list}")),
                fix_cmd: Some(format!("brew link {}", kegs.join(" "))),
            });
        } else if first_line.contains("not readable") {
            let formulae: Vec<&str> = chunk
                .lines()
                .filter(|l| l.starts_with("  "))
                .map(|l| {
                    // "tap/formula: long error message" → just the formula name
                    l.trim().split(':').next().unwrap_or(l.trim())
                })
                .collect();
            if formulae.is_empty() {
                continue;
            }
            let list = formulae.join(", ");
            warnings.push(BrewWarning {
                summary: format!("Unreadable formulae: {list}"),
                detail: Some("These formulae have broken Ruby definitions".into()),
                fix: Some("Untap or reinstall the affected taps".into()),
                fix_cmd: None,
            });
        } else {
            // Generic warning — keep first line as summary
            warnings.push(BrewWarning {
                summary: first_line.to_string(),
                detail: None,
                fix: Some("Run brew doctor for details".into()),
                fix_cmd: Some("brew doctor".into()),
            });
        }
    }

    warnings
}

fn first_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().take(n).collect();
    let result = lines.join("\n");
    let total = s.lines().count();
    if total > n {
        format!("{result}\n... and {} more lines", total - n)
    } else {
        result
    }
}
