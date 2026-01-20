use anyhow::Result;
use std::fs;

use crate::cli::{ConfigCommand, ConfigFormatArg, ConfigTarget};
use crate::config::{
    config_dir, find_config_file, ConfigFormat, RefsConfig, WorkspacesConfig,
};
use crate::ui;
use crate::Context;

pub fn run(_ctx: &Context, cmd: ConfigCommand) -> Result<()> {
    match cmd {
        ConfigCommand::Show => show(),
        ConfigCommand::Convert { config, format, keep } => convert(config, format, keep),
        ConfigCommand::Validate => validate(),
        ConfigCommand::Dir => dir(),
    }
}

fn show() -> Result<()> {
    ui::header("Configuration Files");

    let dir = config_dir()?;
    println!();
    ui::kv("Config directory", &dir.display().to_string());
    println!();

    // Check refs config
    ui::info("refs");
    match find_config_file(&dir, "refs") {
        Some((path, format)) => {
            ui::dim(&format!(
                "  {} ({})",
                path.display(),
                format.extension().to_uppercase()
            ));
        }
        None => {
            ui::dim("  Not found (refs.toml or refs.json)");
        }
    }

    // Check workspaces config
    ui::info("workspaces");
    match find_config_file(&dir, "workspaces") {
        Some((path, format)) => {
            ui::dim(&format!(
                "  {} ({})",
                path.display(),
                format.extension().to_uppercase()
            ));
        }
        None => {
            ui::dim("  Not found (workspaces.toml or workspaces.json)");
        }
    }

    println!();
    ui::dim("TOML files are preferred over JSON when both exist.");
    ui::dim("Use 'bossa config convert' to switch formats.");

    Ok(())
}

fn convert(target: ConfigTarget, format_arg: ConfigFormatArg, keep: bool) -> Result<()> {
    let target_format = match format_arg {
        ConfigFormatArg::Json => ConfigFormat::Json,
        ConfigFormatArg::Toml => ConfigFormat::Toml,
    };

    ui::header(&format!("Converting to {}", target_format.extension().to_uppercase()));

    match target {
        ConfigTarget::Refs => convert_refs(target_format, keep)?,
        ConfigTarget::Workspaces => convert_workspaces(target_format, keep)?,
        ConfigTarget::All => {
            convert_refs(target_format, keep)?;
            convert_workspaces(target_format, keep)?;
        }
    }

    println!();
    ui::success("Conversion complete!");
    Ok(())
}

fn convert_refs(target_format: ConfigFormat, keep: bool) -> Result<()> {
    let dir = config_dir()?;

    match RefsConfig::load_with_format() {
        Ok((config, source_format)) => {
            if source_format == target_format {
                ui::warn(&format!(
                    "refs already in {} format",
                    target_format.extension().to_uppercase()
                ));
                return Ok(());
            }

            let source_path = dir.join(format!("refs.{}", source_format.extension()));
            let target_path = dir.join(format!("refs.{}", target_format.extension()));

            // Save in new format
            config.save_as(target_format)?;
            ui::success(&format!("Created {}", target_path.display()));

            // Remove original unless --keep
            if !keep && source_path.exists() {
                fs::remove_file(&source_path)?;
                ui::dim(&format!("Removed {}", source_path.display()));
            }
        }
        Err(e) => {
            ui::warn(&format!("Could not load refs config: {}", e));
        }
    }

    Ok(())
}

fn convert_workspaces(target_format: ConfigFormat, keep: bool) -> Result<()> {
    let dir = config_dir()?;

    match WorkspacesConfig::load_with_format() {
        Ok((config, source_format)) => {
            if source_format == target_format {
                ui::warn(&format!(
                    "workspaces already in {} format",
                    target_format.extension().to_uppercase()
                ));
                return Ok(());
            }

            let source_path = dir.join(format!("workspaces.{}", source_format.extension()));
            let target_path = dir.join(format!("workspaces.{}", target_format.extension()));

            // Save in new format
            config.save_as(target_format)?;
            ui::success(&format!("Created {}", target_path.display()));

            // Remove original unless --keep
            if !keep && source_path.exists() {
                fs::remove_file(&source_path)?;
                ui::dim(&format!("Removed {}", source_path.display()));
            }
        }
        Err(e) => {
            ui::warn(&format!("Could not load workspaces config: {}", e));
        }
    }

    Ok(())
}

fn validate() -> Result<()> {
    ui::header("Validating Configuration Files");

    let mut all_valid = true;

    // Validate refs
    print!("refs: ");
    match RefsConfig::load_with_format() {
        Ok((config, format)) => {
            println!(
                "{} ({} repos)",
                format.extension().to_uppercase(),
                config.repositories.len()
            );
        }
        Err(e) => {
            ui::error(&format!("Invalid - {}", e));
            all_valid = false;
        }
    }

    // Validate workspaces
    print!("workspaces: ");
    match WorkspacesConfig::load_with_format() {
        Ok((config, format)) => {
            println!(
                "{} ({} workspaces)",
                format.extension().to_uppercase(),
                config.workspaces.len()
            );
        }
        Err(e) => {
            ui::error(&format!("Invalid - {}", e));
            all_valid = false;
        }
    }

    println!();
    if all_valid {
        ui::success("All configuration files are valid!");
    } else {
        ui::error("Some configuration files have issues.");
    }

    Ok(())
}

fn dir() -> Result<()> {
    let dir = config_dir()?;

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()?;
    }

    ui::info(&format!("Opened {}", dir.display()));
    Ok(())
}
