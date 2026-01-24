//! Diff computation and display - bossa-specific UI

use crate::resource::Resource;
use colored::Colorize;
use declarative::{ResourceDiff, ResourceState};

/// Compute diffs for all resources
pub fn compute_diffs(resources: &[Box<dyn Resource>]) -> Vec<ResourceDiff> {
    resources
        .iter()
        .filter_map(|r| ResourceDiff::from_resource(r.as_ref()).ok().flatten())
        .collect()
}

/// Display a list of diffs in a user-friendly format
pub fn display_diff(diffs: &[ResourceDiff]) {
    if diffs.is_empty() {
        println!();
        println!("  {} No changes needed", "✓".green());
        return;
    }

    // Group by resource type
    let mut by_type: std::collections::HashMap<&str, Vec<&ResourceDiff>> =
        std::collections::HashMap::new();
    for diff in diffs {
        by_type
            .entry(diff.resource_type.as_str())
            .or_default()
            .push(diff);
    }

    println!();
    println!(
        "┌─ {} ─────────────────────────────────────────┐",
        "Configuration Diff".bold()
    );
    println!("│");

    for (resource_type, type_diffs) in &by_type {
        let type_name = match *resource_type {
            "brew_formula" => "Packages (brew formulas)",
            "brew_cask" => "Packages (brew casks)",
            "brew_tap" => "Packages (brew taps)",
            "macos_default" => "Defaults (macOS)",
            "symlink" => "Symlinks",
            "service" => "Services",
            _ => resource_type,
        };
        println!("│ {}", type_name.bold());

        for diff in type_diffs {
            let symbol = match (&diff.current, &diff.desired) {
                (ResourceState::Absent, ResourceState::Present { .. }) => "+".green(),
                (ResourceState::Present { .. }, ResourceState::Absent) => "-".red(),
                (ResourceState::Modified { .. }, _) | (_, ResourceState::Modified { .. }) => {
                    "~".yellow()
                }
                _ => "?".dimmed(),
            };

            let sudo_indicator = if diff.requires_sudo {
                " [sudo]".red().to_string()
            } else {
                String::new()
            };

            let state_desc = match (&diff.current, &diff.desired) {
                (ResourceState::Absent, ResourceState::Present { details }) => {
                    format!(
                        "(not installed){}",
                        details
                            .as_ref()
                            .map(|d| format!(" → {}", d))
                            .unwrap_or_default()
                    )
                }
                (
                    ResourceState::Present { details: from },
                    ResourceState::Present { details: to },
                ) => {
                    format!(
                        "{} → {}",
                        from.as_deref().unwrap_or("current"),
                        to.as_deref().unwrap_or("desired")
                    )
                }
                (ResourceState::Present { .. }, ResourceState::Absent) => {
                    "(will remove)".to_string()
                }
                _ => String::new(),
            };

            println!(
                "│   {} {:<30} {}{}",
                symbol,
                diff.resource_id,
                state_desc.dimmed(),
                sudo_indicator
            );
        }
        println!("│");
    }

    // Summary
    let sudo_count = diffs.iter().filter(|d| d.requires_sudo).count();
    let regular_count = diffs.len() - sudo_count;

    println!("├─────────────────────────────────────────────────────┤");
    println!(
        "│ Summary: {} changes ({} unprivileged, {} require sudo)",
        diffs.len().to_string().bold(),
        regular_count.to_string().green(),
        sudo_count.to_string().red()
    );
    println!("└─────────────────────────────────────────────────────┘");
}

/// Display the sudo boundary warning
pub fn display_sudo_boundary(privileged_diffs: &[ResourceDiff]) {
    if privileged_diffs.is_empty() {
        return;
    }

    println!();
    println!(
        "┌─ {} ─────────────────────────────────────────┐",
        "Privilege Boundary".yellow().bold()
    );
    println!("│");
    println!(
        "│  {}  The following {} operations require sudo:",
        "⚠".yellow(),
        privileged_diffs.len()
    );
    println!("│");

    for diff in privileged_diffs.iter().take(10) {
        println!("│  • {}", diff.description);
    }

    if privileged_diffs.len() > 10 {
        println!("│  • ... and {} more", privileged_diffs.len() - 10);
    }

    println!("│");
    println!("│  Sudo will be requested once and released immediately after.");
    println!("│");
    println!("└─────────────────────────────────────────────────────────────┘");
}
