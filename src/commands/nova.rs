//! Nova command - bootstrap a new machine natively
//!
//! "bossa nova" - the vision: new machine → brew install bossa → bossa nova → done

use anyhow::{Context, Result};
use colored::Colorize;

use crate::Context as AppContext;
use crate::cli::NovaArgs;
use crate::config;
use crate::engine::planner::ExecutionPlanExt;
use crate::engine::{self, ExecuteOptions, ExecutionPlan};
use crate::resource::{
    BrewPackage, DefaultValue as ResDefaultValue, DockApp, DockFolder, FileHandler, GHExtension,
    MacOSDefault, PnpmPackage, Symlink, VSCodeExtension,
};
use crate::runner;
use crate::schema::{BossaConfig, DefaultValue as SchemaDefaultValue};
use crate::sudo::SudoConfig;
use crate::ui;

pub fn run(ctx: &AppContext, args: NovaArgs) -> Result<()> {
    ui::banner();

    if args.list_stages {
        list_stages();
        return Ok(());
    }

    ui::header("Bossa Nova - System Bootstrap");
    println!();

    // Load config
    let config = load_config()?;

    // Build execution plan
    let plan = build_plan(ctx, &config, &args)?;

    if plan.is_empty() {
        ui::success("Nothing to do - system is already configured!");
        return Ok(());
    }

    // Show what we're going to do
    println!(
        "  {} resources to apply ({} unprivileged, {} privileged)",
        plan.total_resources().to_string().bold(),
        plan.unprivileged.len().to_string().green(),
        plan.privileged.len().to_string().yellow()
    );
    println!();

    // Execute
    let opts = ExecuteOptions {
        dry_run: args.dry_run,
        jobs: args.jobs.map_or(4, |j| j as usize),
        yes: args.yes,
        verbose: ctx.verbose > 0,
    };

    let summary = engine::execute(plan, opts)?;

    if !summary.is_success() {
        anyhow::bail!("{} resource(s) failed to apply", summary.failed);
    }

    Ok(())
}

/// Install Homebrew if it's not already present.
///
/// Returns `Ok(())` immediately if brew is already on `$PATH` (idempotent).
/// Otherwise prompts the user for confirmation (skipped when `yes` is true)
/// and runs the official Homebrew install script.
pub(crate) fn install_homebrew(yes: bool) -> Result<()> {
    if runner::command_exists("brew") {
        return Ok(());
    }

    ui::info("Homebrew is not installed.");

    if !yes {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt("Install Homebrew now?")
            .default(true)
            .interact()
            .context("Failed to read confirmation")?;

        if !confirmed {
            anyhow::bail!("Homebrew installation declined — cannot continue without brew");
        }
    }

    ui::info("Installing Homebrew...");

    let status = runner::run(
        "/bin/bash",
        &[
            "-c",
            "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)",
        ],
    )
    .context("Failed to run Homebrew install script")?;

    if !status.success() {
        anyhow::bail!("Homebrew install script exited with status {status}");
    }

    // Verify brew is now reachable
    if !runner::command_exists("brew") {
        anyhow::bail!(
            "Homebrew install script succeeded but `brew` is still not found on $PATH.\n\
             You may need to add Homebrew to your PATH and restart your shell."
        );
    }

    ui::success("Homebrew installed successfully!");
    Ok(())
}

fn load_config() -> Result<BossaConfig> {
    let config_dir = config::config_dir()?;

    // Try to load unified config
    match config::load_config::<BossaConfig>(&config_dir, "config") {
        Ok((config, _format)) => Ok(config),
        Err(_) => {
            // Return default config if none exists
            ui::warn("No config found at ~/.config/bossa/config.toml");
            ui::info("Run 'bossa add' commands or create config manually");
            Ok(BossaConfig::default())
        }
    }
}

fn build_plan(ctx: &AppContext, config: &BossaConfig, args: &NovaArgs) -> Result<ExecutionPlan> {
    let mut plan = ExecutionPlan::new();

    // Convert schema::SudoConfig to sudo::SudoConfig
    let sudo_config = SudoConfig {
        casks: config.sudo.casks.clone(),
        defaults: config.sudo.defaults.clone(),
        operations: config.sudo.operations.clone(),
    };

    // Determine which stages to run
    let stages = determine_stages(args);

    // Stage: homebrew (eager — must run before any brew resources are built)
    if stages.contains(&"homebrew") {
        install_homebrew(args.yes)?;
    }

    // Stage: defaults
    if stages.contains(&"defaults") {
        add_defaults_resources(&mut plan, config, &sudo_config)?;
    }

    // Stage: packages (brew)
    if stages.contains(&"packages") {
        add_brew_resources(&mut plan, config, &sudo_config)?;
    }

    // Stage: cellar (sync homebrew packages to external SSD)
    if stages.contains(&"cellar")
        && let Err(e) = super::cellar::sync_for_nova(ctx)
    {
        ui::warn(&format!("Cellar stage failed: {e} — continuing"));
    }

    // Stage: dotfiles (must run before symlinks — stow depends on ~/.dotfiles)
    if stages.contains(&"dotfiles")
        && let Err(e) = super::dotfiles::sync_for_nova(config)
    {
        ui::warn(&format!("Dotfiles stage failed: {e} — continuing"));
    }

    // Stage: symlinks
    if stages.contains(&"symlinks") {
        add_symlink_resources(&mut plan, config)?;
    }

    // Stage: dock
    if stages.contains(&"dock") {
        add_dock_resources(&mut plan, config)?;
    }

    // Stage: handlers
    if stages.contains(&"handlers") {
        add_handler_resources(&mut plan, config)?;
    }

    // Stage: ecosystem (pnpm, gh, vscode)
    if stages.contains(&"ecosystem") {
        add_ecosystem_resources(&mut plan, config)?;
    }

    Ok(plan)
}

/// Map user-facing stage aliases to internal canonical names.
fn normalize_stage(name: &str) -> &str {
    match name {
        "essential" | "brew" => "packages",
        "stow" => "symlinks",
        "pnpm" => "ecosystem",
        other => other,
    }
}

fn determine_stages(args: &NovaArgs) -> Vec<&'static str> {
    // If --only specified, use only those
    if let Some(ref only) = args.only {
        let only_set: Vec<&str> = only.split(',').map(|s| normalize_stage(s.trim())).collect();
        return IMPLEMENTED_STAGES
            .iter()
            .filter(|&&s| only_set.contains(&s))
            .copied()
            .collect();
    }

    // If --skip specified, remove those
    if let Some(ref skip) = args.skip {
        let skip_set: Vec<&str> = skip.split(',').map(|s| normalize_stage(s.trim())).collect();
        return IMPLEMENTED_STAGES
            .iter()
            .filter(|&&s| !skip_set.contains(&s))
            .copied()
            .collect();
    }

    IMPLEMENTED_STAGES.to_vec()
}

fn add_defaults_resources(
    plan: &mut ExecutionPlan,
    config: &BossaConfig,
    sudo_config: &SudoConfig,
) -> Result<()> {
    for (domain_key, value) in &config.defaults.settings {
        let res_value = convert_default_value(value);

        let mut resource = MacOSDefault::from_domain_key(domain_key, res_value)
            .with_context(|| format!("Invalid default key: {domain_key}"))?;

        // Check if this default requires sudo
        if sudo_config.default_requires_sudo(domain_key) {
            resource = resource.with_sudo(true);
        }

        plan.add_resource(Box::new(resource), sudo_config);
    }

    // Add restart services
    for service in &config.defaults.restart.services {
        plan.add_restart_service(service.clone());
    }

    Ok(())
}

fn add_brew_resources(
    plan: &mut ExecutionPlan,
    config: &BossaConfig,
    sudo_config: &SudoConfig,
) -> Result<()> {
    let brew = &config.packages.brew;

    // Taps first
    for tap in &brew.taps {
        let resource = BrewPackage::tap(tap);
        plan.add_resource(Box::new(resource), sudo_config);
    }

    // Essential formulas (with retry - TODO: implement retry in executor)
    for pkg in &brew.essential.packages {
        let resource = BrewPackage::formula(pkg);
        plan.add_resource(Box::new(resource), sudo_config);
    }

    // Regular formulas
    for formula in &brew.formulas {
        let resource = BrewPackage::formula(formula);
        plan.add_resource(Box::new(resource), sudo_config);
    }

    // Casks (check sudo allowlist)
    for cask in &brew.casks {
        let mut resource = BrewPackage::cask(cask);
        if sudo_config.cask_requires_sudo(cask) {
            resource = resource.with_sudo(true);
        }
        plan.add_resource(Box::new(resource), sudo_config);
    }

    // Fonts (also casks)
    for font in &brew.fonts {
        let resource = BrewPackage::cask(font);
        plan.add_resource(Box::new(resource), sudo_config);
    }

    Ok(())
}

fn add_symlink_resources(plan: &mut ExecutionPlan, config: &BossaConfig) -> Result<()> {
    let symlinks_opt = &config.symlinks;

    // Check if symlinks config exists
    let symlinks = match symlinks_opt {
        Some(s) => s,
        None => return Ok(()), // No symlinks configured
    };

    if symlinks.source.is_empty() || symlinks.packages.is_empty() {
        return Ok(());
    }

    let source_base = crate::paths::expand(&symlinks.source)
        .to_string_lossy()
        .to_string();
    let target_base = crate::paths::expand(&symlinks.target)
        .to_string_lossy()
        .to_string();

    for package in &symlinks.packages {
        let package_source = std::path::Path::new(&source_base).join(package);

        // Walk the package directory and create symlinks
        if package_source.exists() {
            walk_and_create_symlinks(
                plan,
                &package_source,
                &package_source,
                std::path::Path::new(&target_base),
                &symlinks.ignore,
            )?;
        }
    }

    Ok(())
}

fn walk_and_create_symlinks(
    plan: &mut ExecutionPlan,
    base: &std::path::Path,
    current: &std::path::Path,
    target_base: &std::path::Path,
    ignore: &[String],
) -> Result<()> {
    use std::fs;

    if !current.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();

        // Skip ignored patterns
        if ignore.iter().any(|p| file_name == *p || path.ends_with(p)) {
            continue;
        }

        // Calculate relative path from base
        let relative = path.strip_prefix(base)?;
        let target = target_base.join(relative);

        if path.is_file() || path.is_symlink() {
            // Create symlink for files
            let resource = Symlink::new(&path, &target);
            plan.add_resource(Box::new(resource), &SudoConfig::default());
        } else if path.is_dir() {
            // Recurse into directories
            walk_and_create_symlinks(plan, base, &path, target_base, ignore)?;
        }
    }

    Ok(())
}

fn add_dock_resources(plan: &mut ExecutionPlan, config: &BossaConfig) -> Result<()> {
    let dock = &config.dock;

    // Add dock apps
    for (i, app) in dock.apps.iter().enumerate() {
        let resource = DockApp::new(app).at_position(i + 1);
        plan.add_resource(Box::new(resource), &SudoConfig::default());
    }

    // Add dock folders
    for folder in &dock.folders {
        let resource = DockFolder {
            path: folder.path.clone(),
            view: folder.view.clone(),
            display: folder.display.clone(),
            sort: folder.sort.clone(),
        };
        plan.add_resource(Box::new(resource), &SudoConfig::default());
    }

    // Restart Dock after changes
    if !dock.apps.is_empty() || !dock.folders.is_empty() {
        plan.add_restart_service("Dock".to_string());
    }

    Ok(())
}

fn add_handler_resources(plan: &mut ExecutionPlan, config: &BossaConfig) -> Result<()> {
    for (bundle_id, utis) in &config.handlers.handlers {
        for uti in utis {
            let resource = FileHandler::new(bundle_id, uti);
            plan.add_resource(Box::new(resource), &SudoConfig::default());
        }
    }
    Ok(())
}

fn add_ecosystem_resources(plan: &mut ExecutionPlan, config: &BossaConfig) -> Result<()> {
    // pnpm globals
    for pkg in &config.packages.pnpm.globals {
        let resource = PnpmPackage::new(pkg);
        plan.add_resource(Box::new(resource), &SudoConfig::default());
    }

    // gh extensions
    for ext in &config.packages.gh.extensions {
        let resource = GHExtension::new(ext);
        plan.add_resource(Box::new(resource), &SudoConfig::default());
    }

    // vscode extensions
    for ext in &config.packages.vscode.extensions {
        let resource = VSCodeExtension::new(ext);
        plan.add_resource(Box::new(resource), &SudoConfig::default());
    }

    Ok(())
}

fn convert_default_value(value: &SchemaDefaultValue) -> ResDefaultValue {
    match value {
        SchemaDefaultValue::Bool(b) => ResDefaultValue::Bool(*b),
        SchemaDefaultValue::Int(i) => ResDefaultValue::Int(*i),
        SchemaDefaultValue::Float(f) => ResDefaultValue::Float(*f),
        SchemaDefaultValue::String(s) => ResDefaultValue::String(s.clone()),
        SchemaDefaultValue::Array(_) => {
            // The resource layer has no Array variant; log a warning and skip
            log::warn!("Skipping array default value (not supported): {value:?}");
            ResDefaultValue::String(String::new())
        }
    }
}

/// Implemented stages (subset of NovaStage that have actual logic wired up)
const IMPLEMENTED_STAGES: &[&str] = &[
    "defaults",
    "homebrew",
    "packages",
    "cellar",
    "dotfiles",
    "symlinks",
    "dock",
    "handlers",
    "ecosystem",
];

fn list_stages() {
    use crate::cli::NovaStage;

    ui::header("Available Stages");
    println!();

    for stage in NovaStage::all() {
        let name = stage.name();
        let desc = stage.description();
        let is_implemented = IMPLEMENTED_STAGES.contains(&name);
        if is_implemented {
            println!("  {:<15} {}", name.bold(), desc.dimmed());
        } else {
            println!(
                "  {:<15} {} {}",
                name.bold(),
                desc.dimmed(),
                "(planned)".yellow()
            );
        }
    }

    println!();
    ui::section("Usage Examples");
    println!();
    println!("  {} Run all stages", "bossa nova".bold());
    println!(
        "  {} Skip specific stages",
        "bossa nova --skip=packages".bold()
    );
    println!(
        "  {} Run only specific stages",
        "bossa nova --only=defaults,symlinks".bold()
    );
    println!(
        "  {} Preview without changes",
        "bossa nova --dry-run".bold()
    );
}
