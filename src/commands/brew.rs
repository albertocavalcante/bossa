//! Homebrew package management commands using brewkit.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::PathBuf;

use crate::cli::BrewCommand;
use crate::progress;
use crate::ui;
use crate::Context as AppContext;

pub fn run(_ctx: &AppContext, cmd: BrewCommand) -> Result<()> {
    match cmd {
        BrewCommand::Apply {
            essential,
            dry_run,
            file,
        } => apply(essential, dry_run, file),
        BrewCommand::Capture { output } => capture(output),
        BrewCommand::Audit { file } => audit(file),
        BrewCommand::List { r#type } => list(r#type),
    }
}

/// Get the default Brewfile path.
fn default_brewfile_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join("dotfiles/Brewfile"))
        .unwrap_or_else(|| PathBuf::from("Brewfile"))
}

/// Get the Brewfile path, using the provided path or the default.
fn get_brewfile_path(file: Option<String>) -> PathBuf {
    file.map(PathBuf::from).unwrap_or_else(default_brewfile_path)
}

/// Format a package type with color.
fn colored_type(pkg_type: &brewkit::PackageType) -> colored::ColoredString {
    match pkg_type {
        brewkit::PackageType::Tap => "tap".blue(),
        brewkit::PackageType::Brew => "brew".green(),
        brewkit::PackageType::Cask => "cask".magenta(),
        brewkit::PackageType::Mas => "mas".yellow(),
        brewkit::PackageType::Vscode => "vscode".cyan(),
    }
}

/// Print a summary of package counts by type.
fn print_package_summary(brewfile: &brewkit::Brewfile) {
    let taps = brewfile.taps().len();
    let brews = brewfile.brews().len();
    let casks = brewfile.casks().len();
    let mas = brewfile.mas_apps().len();
    let vscode = brewfile.vscode_extensions().len();

    let mut parts = Vec::new();
    if taps > 0 {
        parts.push(format!("{} {}", taps, "taps".blue()));
    }
    if brews > 0 {
        parts.push(format!("{} {}", brews, "formulas".green()));
    }
    if casks > 0 {
        parts.push(format!("{} {}", casks, "casks".magenta()));
    }
    if mas > 0 {
        parts.push(format!("{} {}", mas, "mas apps".yellow()));
    }
    if vscode > 0 {
        parts.push(format!("{} {}", vscode, "vscode extensions".cyan()));
    }

    if !parts.is_empty() {
        println!("  {}", parts.join(", "));
    }
}

/// Create a brewkit client, with better error handling.
fn create_client() -> Result<brewkit::Client, String> {
    match brewkit::Client::new() {
        Ok(c) => Ok(c),
        Err(brewkit::Error::BrewNotFound) => {
            Err("Homebrew is not installed.\n\n  Install it with:\n    /bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"\n\n  Or visit: https://brew.sh".to_string())
        }
        Err(e) => Err(format!("Failed to initialize Homebrew client: {}", e)),
    }
}

fn apply(essential: bool, dry_run: bool, file: Option<String>) -> Result<()> {
    if essential {
        ui::header("Installing Essential Packages");
        ui::dim("Only taps and formulas will be installed (no casks, mas apps, or vscode extensions)");
    } else {
        ui::header("Installing All Packages");
    }

    let brewfile_path = get_brewfile_path(file);
    if !brewfile_path.exists() {
        ui::error(&format!(
            "Brewfile not found at {}",
            brewfile_path.display()
        ));
        println!();
        ui::info("Create a Brewfile or run 'bossa brew capture' to generate one from installed packages");
        return Ok(());
    }

    ui::dim(&format!("Using: {}", brewfile_path.display()));
    println!();

    // Create brewkit client
    let client = match create_client() {
        Ok(c) => c,
        Err(msg) => {
            ui::error(&msg);
            return Ok(());
        }
    };

    // Parse Brewfile
    let pb = progress::spinner("Parsing Brewfile...");
    let mut brewfile = client
        .parse_brewfile(&brewfile_path)
        .context("Failed to parse Brewfile")?;

    // Filter to essential packages if requested
    if essential {
        brewfile.packages.retain(|p| {
            matches!(
                p.package_type,
                brewkit::PackageType::Tap | brewkit::PackageType::Brew
            )
        });
    }

    progress::finish_success(
        &pb,
        &format!("Found {} packages", brewfile.packages.len()),
    );
    print_package_summary(&brewfile);
    println!();

    if dry_run {
        ui::info("Dry run - showing what would be installed:");
        println!();

        // Show what would be installed
        let audit_result = client.audit(&brewfile)?;

        if audit_result.missing.is_empty() {
            ui::success("All packages are already installed!");
        } else {
            ui::warn(&format!(
                "{} packages would be installed:",
                audit_result.missing.len()
            ));
            println!();
            for pkg in &audit_result.missing {
                println!("    {} {}", colored_type(&pkg.package_type), pkg.name);
            }
        }

        return Ok(());
    }

    // For essential mode, we need to write a temporary Brewfile
    let bundle_path = if essential {
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.join("bossa_essential_brewfile");
        client.write_brewfile(&brewfile, &temp_path)?;
        temp_path
    } else {
        brewfile_path.clone()
    };

    // Run brew bundle
    let pb = progress::spinner("Running brew bundle...");
    let result = client.bundle(&bundle_path)?;

    // Clean up temp file if we created one
    if essential {
        let _ = std::fs::remove_file(&bundle_path);
    }

    progress::finish_success(&pb, "Bundle complete");

    // Report results
    println!();

    if !result.installed.is_empty() {
        println!(
            "{} {} {}",
            "✓".green().bold(),
            "Installed:".green().bold(),
            result.installed.len()
        );
        for name in &result.installed {
            println!("    {}", name.green());
        }
        println!();
    }

    if !result.upgraded.is_empty() {
        println!(
            "{} {} {}",
            "→".blue().bold(),
            "Upgraded:".blue().bold(),
            result.upgraded.len()
        );
        for name in &result.upgraded {
            println!("    {}", name.blue());
        }
        println!();
    }

    if !result.skipped.is_empty() {
        println!(
            "  {} {}",
            "Already installed:".dimmed(),
            result.skipped.len()
        );
    }

    if !result.failed.is_empty() {
        println!();
        println!(
            "{} {} {}",
            "✗".red().bold(),
            "Failed:".red().bold(),
            result.failed.len()
        );
        for (name, err) in &result.failed {
            println!("    {} {}", name.red(), format!("- {}", err).dimmed());
        }
        println!();
    }

    // Summary
    println!();
    println!("{}", "─".repeat(50).dimmed());
    println!(
        "  {} installed, {} upgraded, {} skipped, {} failed",
        result.installed.len().to_string().green(),
        result.upgraded.len().to_string().blue(),
        result.skipped.len().to_string().dimmed(),
        if result.failed.is_empty() {
            result.failed.len().to_string().dimmed()
        } else {
            result.failed.len().to_string().red()
        }
    );

    if result.is_success() {
        println!();
        ui::success("Brew apply complete!");
    }

    Ok(())
}

fn capture(output: Option<String>) -> Result<()> {
    ui::header("Capturing Brew Packages");

    let output_path = output
        .map(PathBuf::from)
        .unwrap_or_else(default_brewfile_path);

    // Create brewkit client
    let client = match create_client() {
        Ok(c) => c,
        Err(msg) => {
            ui::error(&msg);
            return Ok(());
        }
    };

    let pb = progress::spinner("Capturing installed packages...");

    // Capture current state
    let brewfile = client.capture_brewfile()?;

    // Write to file
    client.write_brewfile(&brewfile, &output_path)?;

    progress::finish_success(
        &pb,
        &format!(
            "Captured {} packages to {}",
            brewfile.packages.len(),
            output_path.display()
        ),
    );

    // Summary with colors
    println!();
    print_package_summary(&brewfile);

    Ok(())
}

fn audit(file: Option<String>) -> Result<()> {
    ui::header("Brew Audit - Drift Detection");

    let brewfile_path = get_brewfile_path(file);
    if !brewfile_path.exists() {
        ui::error(&format!(
            "Brewfile not found at {}",
            brewfile_path.display()
        ));
        println!();
        ui::info("Create a Brewfile or specify one with --file");
        return Ok(());
    }

    ui::dim(&format!("Comparing against: {}", brewfile_path.display()));
    println!();

    // Create brewkit client
    let client = match create_client() {
        Ok(c) => c,
        Err(msg) => {
            ui::error(&msg);
            return Ok(());
        }
    };

    let pb = progress::spinner("Auditing packages...");

    // Parse Brewfile
    let brewfile = client.parse_brewfile(&brewfile_path)?;

    // Run audit
    let result = client.audit(&brewfile)?;

    progress::finish_success(&pb, "Audit complete");

    if !result.has_drift() {
        println!();
        ui::success("No drift detected - system matches Brewfile!");
        println!();
        println!(
            "  {} packages in sync",
            brewfile.packages.len().to_string().green()
        );
        return Ok(());
    }

    println!();

    // Report untracked packages
    if !result.untracked.is_empty() {
        println!(
            "{} {} ({})",
            "⚠".yellow(),
            "Untracked packages".yellow().bold(),
            result.untracked.len()
        );
        ui::dim("Installed but not in Brewfile:");
        println!();
        for pkg in &result.untracked {
            println!(
                "    {} {} {}",
                colored_type(&pkg.package_type),
                pkg.name,
                format!("({})", pkg.version).dimmed()
            );
        }
        println!();
    }

    // Report missing packages
    if !result.missing.is_empty() {
        println!(
            "{} {} ({})",
            "✗".red(),
            "Missing packages".red().bold(),
            result.missing.len()
        );
        ui::dim("In Brewfile but not installed:");
        println!();
        for pkg in &result.missing {
            println!("    {} {}", colored_type(&pkg.package_type), pkg.name);
        }
        println!();
    }

    // Report version mismatches
    if !result.mismatched.is_empty() {
        println!(
            "{} {} ({})",
            "≠".blue(),
            "Version mismatches".blue().bold(),
            result.mismatched.len()
        );
        println!();
        for (declared, installed) in &result.mismatched {
            println!(
                "    {} {} {} → {}",
                colored_type(&declared.package_type),
                declared.name,
                declared
                    .version
                    .as_deref()
                    .unwrap_or("?")
                    .to_string()
                    .dimmed(),
                installed.version.green()
            );
        }
        println!();
    }

    // Summary
    println!("{}", "─".repeat(50).dimmed());
    println!(
        "  {} untracked, {} missing, {} version mismatches",
        if result.untracked.is_empty() {
            "0".dimmed()
        } else {
            result.untracked.len().to_string().yellow()
        },
        if result.missing.is_empty() {
            "0".dimmed()
        } else {
            result.missing.len().to_string().red()
        },
        if result.mismatched.is_empty() {
            "0".dimmed()
        } else {
            result.mismatched.len().to_string().blue()
        }
    );

    // Suggestions
    println!();
    ui::info("To fix drift:");
    if !result.missing.is_empty() {
        println!("    Run {} to install missing packages", "bossa brew apply".cyan());
    }
    if !result.untracked.is_empty() {
        println!(
            "    Run {} to add untracked packages to Brewfile",
            "bossa brew capture".cyan()
        );
        println!(
            "    Or uninstall them with {}",
            "brew uninstall <package>".cyan()
        );
    }

    Ok(())
}

fn list(filter_type: Option<String>) -> Result<()> {
    ui::header("Installed Homebrew Packages");

    // Create brewkit client
    let client = match create_client() {
        Ok(c) => c,
        Err(msg) => {
            ui::error(&msg);
            return Ok(());
        }
    };

    // Parse filter type
    let filter = filter_type.as_ref().and_then(|t| {
        brewkit::PackageType::from_directive(t)
    });

    if let Some(ref ft) = filter_type
        && filter.is_none()
    {
        ui::error(&format!("Unknown package type: {}", ft));
        ui::info("Valid types: tap, brew, cask, mas, vscode");
        return Ok(());
    }

    let pb = progress::spinner("Fetching installed packages...");

    // Collect packages by type
    let types_to_list: Vec<brewkit::PackageType> = if let Some(t) = filter {
        vec![t]
    } else {
        vec![
            brewkit::PackageType::Tap,
            brewkit::PackageType::Brew,
            brewkit::PackageType::Cask,
            brewkit::PackageType::Mas,
            brewkit::PackageType::Vscode,
        ]
    };

    let mut total = 0;
    let mut by_type: Vec<(brewkit::PackageType, Vec<brewkit::InstalledPackage>)> = Vec::new();

    for pkg_type in types_to_list {
        match client.list_installed(pkg_type) {
            Ok(packages) => {
                total += packages.len();
                if !packages.is_empty() {
                    by_type.push((pkg_type, packages));
                }
            }
            Err(e) => {
                // Silently skip if the tool isn't available (e.g., mas, code)
                if !matches!(e, brewkit::Error::Other(_)) {
                    ui::warn(&format!("Could not list {}: {}", pkg_type, e));
                }
            }
        }
    }

    progress::finish_success(&pb, &format!("Found {} packages", total));
    println!();

    if by_type.is_empty() {
        ui::info("No packages found");
        return Ok(());
    }

    // Print packages grouped by type
    for (pkg_type, packages) in &by_type {
        let type_label = match pkg_type {
            brewkit::PackageType::Tap => "Taps".blue(),
            brewkit::PackageType::Brew => "Formulas".green(),
            brewkit::PackageType::Cask => "Casks".magenta(),
            brewkit::PackageType::Mas => "Mac App Store".yellow(),
            brewkit::PackageType::Vscode => "VS Code Extensions".cyan(),
        };

        println!("{} ({})", type_label.bold(), packages.len());
        println!();

        for pkg in packages {
            if pkg.version.is_empty() {
                println!("    {}", pkg.name);
            } else {
                println!("    {} {}", pkg.name, format!("({})", pkg.version).dimmed());
            }
        }
        println!();
    }

    // Summary
    println!("{}", "─".repeat(50).dimmed());
    let mut summary_parts = Vec::new();
    for (pkg_type, packages) in &by_type {
        let count = packages.len().to_string();
        let label = match pkg_type {
            brewkit::PackageType::Tap => format!("{} taps", count.blue()),
            brewkit::PackageType::Brew => format!("{} formulas", count.green()),
            brewkit::PackageType::Cask => format!("{} casks", count.magenta()),
            brewkit::PackageType::Mas => format!("{} mas", count.yellow()),
            brewkit::PackageType::Vscode => format!("{} vscode", count.cyan()),
        };
        summary_parts.push(label);
    }
    println!("  Total: {} ({})", total, summary_parts.join(", "));

    Ok(())
}
