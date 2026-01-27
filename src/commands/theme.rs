//! Theme management commands for GNOME/GTK.
//!
//! This module provides commands for managing desktop themes on Linux,
//! specifically for GNOME and GTK-based environments.

use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::process::Command;

use crate::Context as AppContext;
use crate::cli::ThemeCommand;
use crate::schema::BossaConfig;
use crate::ui;

/// Run the theme command.
pub fn run(ctx: &AppContext, cmd: ThemeCommand) -> Result<()> {
    // Theme commands only work on Linux with GNOME
    if !cfg!(target_os = "linux") {
        bail!("Theme commands are only available on Linux");
    }

    match cmd {
        ThemeCommand::List => list(ctx),
        ThemeCommand::Status => status(ctx),
        ThemeCommand::Apply { name, dry_run } => apply(ctx, &name, dry_run),
        ThemeCommand::Show { name } => show(ctx, &name),
    }
}

/// List available theme presets.
fn list(ctx: &AppContext) -> Result<()> {
    let config = BossaConfig::load()?;

    if config.themes.themes.is_empty() {
        ui::info("No themes defined in config. Add themes to [themes] section.");
        return Ok(());
    }

    if !ctx.quiet {
        ui::header("Available Themes");
        println!();
    }

    // Print table header
    println!(
        "  {:<20} {:<40} {:>8}",
        "Name".bold(),
        "Description".bold(),
        "Status".bold()
    );
    println!("  {} {} {}", "─".repeat(20), "─".repeat(40), "─".repeat(8));

    for (name, def) in config.themes.enabled_themes() {
        let status = if def.enabled {
            "enabled".green()
        } else {
            "disabled".dimmed()
        };

        let description = if def.description.len() > 38 {
            format!("{}...", &def.description[..35])
        } else {
            def.description.clone()
        };

        println!("  {:<20} {:<40} {:>8}", name, description.dimmed(), status);
    }

    println!();

    Ok(())
}

/// Show current theme status.
fn status(ctx: &AppContext) -> Result<()> {
    if !ctx.quiet {
        ui::header("Current Theme Status");
        println!();
    }

    // Get current settings using gsettings
    let gtk_theme = get_gsetting("org.gnome.desktop.interface", "gtk-theme")?;
    let icon_theme = get_gsetting("org.gnome.desktop.interface", "icon-theme")?;
    let cursor_theme = get_gsetting("org.gnome.desktop.interface", "cursor-theme")?;
    let shell_theme = get_gsetting("org.gnome.shell.extensions.user-theme", "name")
        .unwrap_or_else(|_| "(not set)".to_string());
    let wm_theme = get_gsetting("org.gnome.desktop.wm.preferences", "theme")?;
    let button_layout = get_gsetting("org.gnome.desktop.wm.preferences", "button-layout")?;

    println!("  {:<16} {}", "GTK Theme:".bold(), gtk_theme);
    println!("  {:<16} {}", "Shell Theme:".bold(), shell_theme);
    println!("  {:<16} {}", "WM Theme:".bold(), wm_theme);
    println!("  {:<16} {}", "Icons:".bold(), icon_theme);
    println!("  {:<16} {}", "Cursor:".bold(), cursor_theme);
    println!("  {:<16} {}", "Button Layout:".bold(), button_layout);

    println!();

    // Try to find matching preset
    let config = BossaConfig::load()?;
    let mut matched = false;

    for (name, def) in config.themes.enabled_themes() {
        if matches_current(def, &gtk_theme, &icon_theme, &cursor_theme, &shell_theme) {
            println!("  {} Matches preset: {}", "✓".green(), name.cyan());
            matched = true;
            break;
        }
    }

    if !matched && !config.themes.themes.is_empty() {
        println!(
            "  {} Current theme doesn't match any preset",
            "○".bright_black()
        );
    }

    Ok(())
}

/// Check if current settings match a theme definition.
fn matches_current(
    def: &crate::schema::ThemeDefinition,
    gtk: &str,
    icons: &str,
    cursor: &str,
    shell: &str,
) -> bool {
    let gtk_match = def.gtk.as_ref().is_none_or(|t| t == gtk);
    let icons_match = def.icons.as_ref().is_none_or(|t| t == icons);
    let cursor_match = def.cursor.as_ref().is_none_or(|t| t == cursor);
    let shell_match = def.shell.as_ref().is_none_or(|t| t == shell);

    gtk_match && icons_match && cursor_match && shell_match
}

/// Apply a theme preset.
fn apply(ctx: &AppContext, name: &str, dry_run: bool) -> Result<()> {
    let config = BossaConfig::load()?;

    let def = config
        .themes
        .get(name)
        .context(format!("Theme '{}' not found", name))?;

    if !def.enabled {
        bail!("Theme '{}' is disabled", name);
    }

    // Check requirements
    if !def.requires.is_empty() {
        let missing: Vec<_> = def
            .requires
            .iter()
            .filter(|req| !is_tool_installed(req))
            .collect();

        if !missing.is_empty() {
            bail!(
                "Theme '{}' requires tools that are not installed: {}\n\
                 Run: bossa tools apply {}",
                name,
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }

    if !ctx.quiet {
        let mode = if dry_run {
            "Preview".yellow()
        } else {
            "Applying".green()
        };
        ui::header(&format!("{} theme: {}", mode, name));
        println!();

        if !def.description.is_empty() {
            println!("  {}", def.description.dimmed());
            println!();
        }
    }

    let mut changes = 0;

    // Apply GTK theme
    if let Some(ref gtk) = def.gtk {
        if !ctx.quiet {
            println!("  {} GTK theme: {}", icon(dry_run), gtk);
        }
        if !dry_run {
            set_gsetting("org.gnome.desktop.interface", "gtk-theme", gtk)?;
        }
        changes += 1;
    }

    // Apply Shell theme
    if let Some(ref shell) = def.shell {
        if !ctx.quiet {
            println!("  {} Shell theme: {}", icon(dry_run), shell);
        }
        if !dry_run {
            // Shell theme requires user-theme extension
            if set_gsetting("org.gnome.shell.extensions.user-theme", "name", shell).is_err() {
                ui::warn("    Shell theme requires 'user-theme' GNOME extension");
            }
        }
        changes += 1;
    }

    // Apply WM theme
    if let Some(ref wm) = def.wm {
        if !ctx.quiet {
            println!("  {} WM theme: {}", icon(dry_run), wm);
        }
        if !dry_run {
            set_gsetting("org.gnome.desktop.wm.preferences", "theme", wm)?;
        }
        changes += 1;
    }

    // Apply button layout
    if let Some(ref buttons) = def.wm_buttons {
        if !ctx.quiet {
            println!("  {} Button layout: {}", icon(dry_run), buttons);
        }
        if !dry_run {
            set_gsetting("org.gnome.desktop.wm.preferences", "button-layout", buttons)?;
        }
        changes += 1;
    }

    // Apply icon theme
    if let Some(ref icons) = def.icons {
        if !ctx.quiet {
            println!("  {} Icons: {}", icon(dry_run), icons);
        }
        if !dry_run {
            set_gsetting("org.gnome.desktop.interface", "icon-theme", icons)?;
        }
        changes += 1;
    }

    // Apply cursor theme
    if let Some(ref cursor) = def.cursor {
        if !ctx.quiet {
            println!("  {} Cursor: {}", icon(dry_run), cursor);
        }
        if !dry_run {
            set_gsetting("org.gnome.desktop.interface", "cursor-theme", cursor)?;
        }
        changes += 1;
    }

    // Terminal theme (informational only - requires manual setup)
    if let Some(ref terminal) = def.terminal
        && !ctx.quiet
    {
        println!(
            "  {} Terminal: {} {}",
            "ℹ".blue(),
            terminal,
            "(manual setup required)".dimmed()
        );
    }

    println!();

    if dry_run {
        println!(
            "{}",
            "Dry run complete. Run without --dry-run to apply.".dimmed()
        );
    } else {
        ui::success(&format!(
            "Theme '{}' applied ({} settings changed)",
            name, changes
        ));
    }

    Ok(())
}

/// Show details of a theme preset.
fn show(ctx: &AppContext, name: &str) -> Result<()> {
    let config = BossaConfig::load()?;

    let def = config
        .themes
        .get(name)
        .context(format!("Theme '{}' not found", name))?;

    if !ctx.quiet {
        ui::header(&format!("Theme: {}", name));
        println!();
    }

    if !def.description.is_empty() {
        ui::kv("Description", &def.description);
    }

    ui::kv("Enabled", if def.enabled { "yes" } else { "no" });
    println!();

    if let Some(ref gtk) = def.gtk {
        ui::kv("GTK Theme", gtk);
    }
    if let Some(ref shell) = def.shell {
        ui::kv("Shell Theme", shell);
    }
    if let Some(ref wm) = def.wm {
        ui::kv("WM Theme", wm);
    }
    if let Some(ref buttons) = def.wm_buttons {
        ui::kv("Button Layout", buttons);
    }
    if let Some(ref icons) = def.icons {
        ui::kv("Icons", icons);
    }
    if let Some(ref cursor) = def.cursor {
        ui::kv("Cursor", cursor);
    }
    if let Some(ref terminal) = def.terminal {
        ui::kv("Terminal", terminal);
    }

    if !def.requires.is_empty() {
        println!();
        ui::kv("Requires", &def.requires.join(", "));
    }

    Ok(())
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Get a gsettings value.
fn get_gsetting(schema: &str, key: &str) -> Result<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .context("Failed to run gsettings")?;

    if !output.status.success() {
        bail!(
            "gsettings failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // gsettings returns values with quotes, strip them
    let value = String::from_utf8_lossy(&output.stdout)
        .trim()
        .trim_matches('\'')
        .to_string();

    Ok(value)
}

/// Set a gsettings value.
fn set_gsetting(schema: &str, key: &str, value: &str) -> Result<()> {
    let output = Command::new("gsettings")
        .args(["set", schema, key, value])
        .output()
        .context("Failed to run gsettings")?;

    if !output.status.success() {
        bail!(
            "gsettings set failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

/// Check if a tool is installed (in PATH or as bossa tool).
fn is_tool_installed(name: &str) -> bool {
    // Check PATH
    Command::new("which")
        .arg(name)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Get the appropriate icon for dry_run mode.
fn icon(dry_run: bool) -> colored::ColoredString {
    if dry_run {
        "○".yellow()
    } else {
        "✓".green()
    }
}
