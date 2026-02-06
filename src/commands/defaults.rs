use crate::Context;
use crate::cli::{DefaultsCommand, DefaultsType};
use crate::resource::{ApplyContext, ApplyResult, DefaultValue, MacOSDefault, Resource};
use anyhow::{Context as _, Result};
use colored::Colorize;

pub fn run(ctx: &Context, cmd: DefaultsCommand) -> Result<()> {
    match cmd {
        DefaultsCommand::Set {
            domain,
            key,
            value,
            r#type,
        } => set_default(ctx, &domain, &key, &value, r#type),
        DefaultsCommand::Read { domain, key } => read_default(ctx, &domain, key.as_deref()),
    }
}

fn set_default(
    ctx: &Context,
    domain: &str,
    key: &str,
    value: &str,
    type_hint: Option<DefaultsType>,
) -> Result<()> {
    // Parse the value
    let default_value = parse_value(value, type_hint)?;

    // Create the resource
    // Note: MacOSDefault::new takes ownership of default_value
    let resource = MacOSDefault::new(domain, key, default_value.clone());

    if !ctx.quiet {
        println!(
            "Setting {}.{} = {:?}",
            domain.bold(),
            key.bold(),
            default_value
        );
    }

    // Apply the change
    let mut apply_ctx = ApplyContext::new(false, ctx.verbose > 0);
    // TODO: Support sudo if needed. currently running as user.

    let result = resource.apply(&mut apply_ctx)?;

    if !ctx.quiet {
        match result {
            ApplyResult::Modified | ApplyResult::Created => {
                println!("{}", "  ✓ Updated".green());
            }
            ApplyResult::NoChange => {
                println!("{}", "  ✓ No change needed".dimmed());
            }
            ApplyResult::Skipped { reason } => {
                println!("  {} Skipped: {}", "○".yellow(), reason);
            }
            ApplyResult::Failed { error } => {
                println!("  {} Failed: {}", "✗".red(), error);
            }
            _ => {}
        }
    }

    // Special handling for Finder to make changes take effect immediately
    if domain == "com.apple.finder" {
        if !ctx.quiet {
            println!("{}", "  Restarting Finder to apply changes...".dimmed());
        }
        let _ = std::process::Command::new("killall").arg("Finder").output();
    }

    Ok(())
}

fn read_default(_ctx: &Context, domain: &str, key: Option<&str>) -> Result<()> {
    let mut args = vec!["read", domain];
    if let Some(k) = key {
        args.push(k);
    }

    let output = std::process::Command::new("defaults")
        .args(&args)
        .output()
        .context("Failed to run defaults read")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("defaults read failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");

    Ok(())
}

fn parse_value(value: &str, type_hint: Option<DefaultsType>) -> Result<DefaultValue> {
    match type_hint {
        Some(DefaultsType::Bool) => {
            let b = parse_bool(value).ok_or_else(|| anyhow::anyhow!("Invalid boolean: {value}"))?;
            Ok(DefaultValue::Bool(b))
        }
        Some(DefaultsType::Int) => {
            let i = value
                .parse::<i64>()
                .context(format!("Invalid integer: {value}"))?;
            Ok(DefaultValue::Int(i))
        }
        Some(DefaultsType::Float) => {
            let f = value
                .parse::<f64>()
                .context(format!("Invalid float: {value}"))?;
            Ok(DefaultValue::Float(f))
        }
        Some(DefaultsType::String) => Ok(DefaultValue::String(value.to_string())),
        None => {
            // Auto-detect
            if let Some(b) = parse_bool(value) {
                Ok(DefaultValue::Bool(b))
            } else if let Ok(i) = value.parse::<i64>() {
                Ok(DefaultValue::Int(i))
            } else if let Ok(f) = value.parse::<f64>() {
                Ok(DefaultValue::Float(f))
            } else {
                Ok(DefaultValue::String(value.to_string()))
            }
        }
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}
