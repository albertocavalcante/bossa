//! Locations command - manage logical path locations

use anyhow::{Context, Result};
use colored::Colorize;

use crate::Context as AppContext;
use crate::cli::LocationsCommand;
use crate::schema::BossaConfig;
use crate::ui;

pub fn run(_ctx: &AppContext, cmd: LocationsCommand) -> Result<()> {
    match cmd {
        LocationsCommand::List => list(),
        LocationsCommand::Add { name, path } => add(&name, &path),
        LocationsCommand::Remove { name } => remove(&name),
        LocationsCommand::Show { name } => show(&name),
        LocationsCommand::Alias { path, location } => alias(&path, &location),
    }
}

fn list() -> Result<()> {
    let config = BossaConfig::load()?;

    ui::header("Locations");

    if config.locations.paths.is_empty() {
        println!("{}", "No locations configured.".dimmed());
        println!();
        println!("Add one with: bossa locations add <name> <path>");
        return Ok(());
    }

    for (name, path) in &config.locations.paths {
        let resolved = crate::paths::resolve(path, &config.locations);
        let exists = resolved.exists();
        let icon = if exists { "✓".green() } else { "✗".red() };

        println!("  {} {} = {}", icon, name.bold(), resolved.display());
        if path != &resolved.to_string_lossy().to_string() {
            println!("      {}", format!("({path})").dimmed());
        }
    }

    if !config.locations.aliases.is_empty() {
        println!();
        ui::header("Aliases");
        for (alias_path, location_name) in &config.locations.aliases {
            println!("  {} → {}", alias_path.dimmed(), location_name);
        }
    }

    Ok(())
}

fn add(name: &str, path: &str) -> Result<()> {
    let mut config = BossaConfig::load()?;

    // Validate name
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        anyhow::bail!("Location name must be alphanumeric (got: {name})");
    }

    config
        .locations
        .paths
        .insert(name.to_string(), path.to_string());
    config.save()?;

    let resolved = crate::paths::resolve(path, &config.locations);
    println!(
        "{} Added location: {} = {}",
        "✓".green(),
        name.bold(),
        resolved.display()
    );

    Ok(())
}

fn remove(name: &str) -> Result<()> {
    let mut config = BossaConfig::load()?;

    if config.locations.paths.remove(name).is_none() {
        anyhow::bail!("Location '{name}' not found");
    }

    // Also remove any aliases pointing to this location
    config.locations.aliases.retain(|_, loc| loc != name);

    config.save()?;

    println!("{} Removed location: {}", "✓".green(), name);

    Ok(())
}

fn show(name: &str) -> Result<()> {
    let config = BossaConfig::load()?;

    let path = config
        .locations
        .paths
        .get(name)
        .with_context(|| format!("Location '{name}' not found"))?;

    let resolved = crate::paths::resolve(path, &config.locations);
    println!("{}", resolved.display());

    Ok(())
}

fn alias(path: &str, location: &str) -> Result<()> {
    let mut config = BossaConfig::load()?;

    // Verify location exists
    if !config.locations.paths.contains_key(location) {
        anyhow::bail!(
            "Location '{location}' not found. Add it first with: bossa locations add {location} <path>"
        );
    }

    config
        .locations
        .aliases
        .insert(path.to_string(), location.to_string());
    config.save()?;

    println!("{} Added alias: {} → {}", "✓".green(), path, location);

    Ok(())
}
