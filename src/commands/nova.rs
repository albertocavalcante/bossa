//! Nova command - bootstrap a new machine natively
//!
//! "bossa nova" - the vision: new machine → brew install bossa → bossa nova → done

use anyhow::{Context, Result, bail};
use colored::Colorize;

use crate::Context as AppContext;
use crate::cli::NovaArgs;
use crate::config;
use crate::engine::{self, ExecuteOptions, ExecutionPlan};
use crate::resource::{
    BrewPackage, DefaultValue as ResDefaultValue, DockApps, DockFolder, FileHandler, GHExtension,
    MacOSDefault, PnpmPackage, Resource, Symlink, VSCodeExtension,
};
use crate::schema::{BossaConfig, DefaultValue as SchemaDefaultValue};
use crate::sudo::SudoConfig;
use crate::ui;

pub fn run(ctx: &AppContext, args: NovaArgs) -> Result<()> {
    if !ctx.quiet {
        ui::banner();
    }

    if args.list_stages {
        list_stages();
        return Ok(());
    }

    if !ctx.quiet {
        ui::header("Bossa Nova - System Bootstrap");
        println!();
    }

    // Load config
    let config = load_config()?;

    // Build execution plan
    let stages = determine_stages(&args)?;
    check_prerequisites(&config, &stages)?;
    let plan = build_plan(&config, &stages)?;

    if plan.is_empty() {
        if !ctx.quiet {
            ui::success("No resources configured in the selected stages");
        }
        return Ok(());
    }

    // Show what we're going to do
    if !ctx.quiet {
        println!(
            "  {} resources to apply ({} unprivileged, {} privileged)",
            plan.total_resources().to_string().bold(),
            plan.unprivileged.len().to_string().green(),
            plan.privileged.len().to_string().yellow()
        );
        println!();
    }

    // Execute
    let opts = ExecuteOptions {
        dry_run: args.dry_run,
        jobs: args.jobs.map_or(4, |j| j as usize),
        yes: args.yes,
        verbose: ctx.verbose > 0,
    };

    let summary = engine::execute(plan, opts, ctx.quiet)?;

    if !summary.is_success() {
        anyhow::bail!(
            "Selected configuration incomplete: {} failed, {} blocked, {} cancelled; inspect the action diagnostics",
            summary.failed,
            summary.blocked,
            summary.cancelled
        );
    }

    Ok(())
}

fn load_config() -> Result<BossaConfig> {
    config::load_bossa_config(true)
}

/// Preflight Homebrew, cellar and dotfiles before constructing resources.
/// Unsupported synchronization is blocked until it has an explicit safe executor.
fn check_prerequisites(config: &BossaConfig, stages: &[&str]) -> Result<()> {
    let brew = &config.packages.brew;
    let packages_configured = !brew.taps.is_empty()
        || !brew.essential.packages.is_empty()
        || !brew.formulas.is_empty()
        || !brew.casks.is_empty()
        || !brew.fonts.is_empty();
    let needs_brew =
        stages.contains(&"homebrew") || (stages.contains(&"packages") && packages_configured);
    let mut blockers = Vec::new();
    if needs_brew {
        crate::runner::observe("brew", ["--version"])
            .context("Nova blocked: Homebrew is unavailable; install it explicitly and retry; verified bootstrap acquisition is not implemented")?;
    }
    if !config.sudo.operations.is_empty() {
        blockers.push("custom sudo.operations are not implemented; use the explicit cask/default allowlists instead of an ignored privilege policy");
    }
    if stages.contains(&"cellar") && !config.cellar.path.is_empty() {
        blockers.push("cellar: Nova synchronization is blocked until preservation and recovery are implemented; omit this stage to preview other resources");
    }
    if stages.contains(&"dotfiles") && config.dotfiles.is_some() {
        blockers.push("dotfiles: Nova synchronization is blocked until acquisition and dependent symlinks have a safe executor; omit this stage to preview other resources");
    }
    if !blockers.is_empty() {
        bail!("Nova blocked before changes:\n  {}", blockers.join("\n  "));
    }
    Ok(())
}

fn build_plan(config: &BossaConfig, stages: &[&str]) -> Result<ExecutionPlan> {
    let mut plan = ExecutionPlan::new();
    let sudo_config = SudoConfig {
        casks: config.sudo.casks.clone(),
        defaults: config.sudo.defaults.clone(),
        operations: config.sudo.operations.clone(),
    };

    // Stage: defaults
    if stages.contains(&"defaults") {
        add_defaults_resources(&mut plan, config, &sudo_config)?;
    }

    // Stage: packages (brew)
    if stages.contains(&"packages") {
        add_brew_resources(&mut plan, config, &sudo_config)?;
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

fn determine_stages(args: &NovaArgs) -> Result<Vec<&'static str>> {
    let parse = |selection: &str| -> Result<Vec<&'static str>> {
        selection
            .split(',')
            .map(|name| {
                let name = normalize_stage(name.trim());
                IMPLEMENTED_STAGES
                    .iter()
                    .copied()
                    .find(|stage| *stage == name)
                    .with_context(|| format!("Unknown or unsupported Nova stage: {name}"))
            })
            .collect()
    };
    if let Some(only) = &args.only {
        let selected = parse(only)?;
        return Ok(IMPLEMENTED_STAGES
            .iter()
            .copied()
            .filter(|stage| selected.contains(stage))
            .collect());
    }
    if let Some(skip) = &args.skip {
        let skipped = parse(skip)?;
        return Ok(IMPLEMENTED_STAGES
            .iter()
            .copied()
            .filter(|stage| !skipped.contains(stage))
            .collect());
    }
    Ok(IMPLEMENTED_STAGES.to_vec())
}

fn add_defaults_resources(
    plan: &mut ExecutionPlan,
    config: &BossaConfig,
    sudo_config: &SudoConfig,
) -> Result<()> {
    for (domain_key, value) in &config.defaults.settings {
        let res_value = convert_default_value(value)
            .with_context(|| format!("Unsupported default {domain_key}"))?;

        let mut resource = MacOSDefault::from_domain_key(domain_key, res_value)
            .with_context(|| format!("Invalid default key: {domain_key}"))?;

        // Check if this default requires sudo
        if sudo_config.default_requires_sudo(domain_key) {
            resource = resource.with_sudo(true);
        }

        plan.add_resource(Box::new(resource), sudo_config);
    }

    let parents = plan
        .unprivileged
        .iter()
        .chain(&plan.privileged)
        .filter(|resource| resource.resource_type() == "macos_default")
        .map(|resource| declarative::ResourceKey {
            resource_type: resource.resource_type().into(),
            id: resource.id(),
        })
        .collect::<Vec<_>>();
    for service in &config.defaults.restart.services {
        if !parents.is_empty() {
            plan.add_post_action_for(service.clone(), parents.clone());
        }
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
        let resource = BrewPackage::cask(font).with_sudo(sudo_config.cask_requires_sudo(font));
        plan.add_resource(Box::new(resource), sudo_config);
    }

    Ok(())
}

fn add_symlink_resources(plan: &mut ExecutionPlan, config: &BossaConfig) -> Result<()> {
    let Some(symlinks) = &config.symlinks else {
        return Ok(());
    };
    if symlinks.source.is_empty() || symlinks.target.is_empty() {
        bail!("Symlink source and target must be explicit nonempty paths");
    }
    let source = crate::paths::expand(&symlinks.source);
    let target = crate::paths::expand(&symlinks.target);
    if !source.is_absolute() || !target.is_absolute() {
        bail!("Symlink source and target must be absolute after expansion");
    }
    let mut count = 0;
    for package in &symlinks.packages {
        let mut components = std::path::Path::new(package).components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
        {
            bail!("Symlink package must be a single directory name: {package}");
        }
        let base = source.join(package);
        let metadata = std::fs::symlink_metadata(&base).with_context(|| {
            format!(
                "Configured symlink package is unavailable: {}",
                base.display()
            )
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!(
                "Symlink package must be a directory, not a directory link: {}",
                base.display()
            );
        }
        let mut pending = vec![(base.clone(), 0_usize)];
        while let Some((directory, depth)) = pending.pop() {
            if depth > 64 {
                bail!("Symlink package exceeds directory nesting limit");
            }
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(&directory)? {
                count += 1;
                if count > 100_000 {
                    bail!("Symlink discovery exceeds entry limit");
                }
                entries.push(entry?);
            }
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let path = entry.path();
                if symlinks.ignore.iter().any(|pattern| {
                    entry.file_name() == std::ffi::OsStr::new(pattern) || path.ends_with(pattern)
                }) {
                    continue;
                }
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    pending.push((path, depth + 1));
                } else if kind.is_file() || kind.is_symlink() {
                    let destination = target.join(path.strip_prefix(&base)?);
                    plan.add_resource(
                        Box::new(Symlink::new(&path, destination)),
                        &SudoConfig::default(),
                    );
                } else {
                    bail!(
                        "Unsupported filesystem object in symlink package: {}",
                        path.display()
                    );
                }
            }
        }
    }
    Ok(())
}

fn add_dock_resources(plan: &mut ExecutionPlan, config: &BossaConfig) -> Result<()> {
    let dock = &config.dock;

    let mut preferences = Vec::new();
    if let Some(value) = dock.autohide {
        preferences.push(("autohide", ResDefaultValue::Bool(value)));
    }
    if let Some(value) = dock.tilesize {
        if value == 0 {
            bail!("Dock tilesize must be positive");
        }
        preferences.push(("tilesize", ResDefaultValue::Int(i64::from(value))));
    }
    if let Some(value) = &dock.minimize_effect {
        if !matches!(value.as_str(), "scale" | "genie") {
            bail!("Unsupported Dock minimize_effect");
        }
        preferences.push(("mineffect", ResDefaultValue::String(value.clone())));
    }
    if let Some(value) = dock.show_recents {
        preferences.push(("show-recents", ResDefaultValue::Bool(value)));
    }
    let mut parents = Vec::new();
    for (key, value) in preferences {
        let resource = MacOSDefault::new("com.apple.dock", key, value);
        parents.push(declarative::ResourceKey {
            resource_type: resource.resource_type().into(),
            id: resource.id(),
        });
        plan.add_resource(Box::new(resource), &SudoConfig::default());
    }

    // Add dock apps
    if !dock.apps.is_empty() {
        plan.add_resource(
            Box::new(DockApps {
                paths: dock.apps.clone(),
            }),
            &SudoConfig::default(),
        );
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
    if !dock.apps.is_empty() || !dock.folders.is_empty() || !parents.is_empty() {
        parents.extend(
            plan.unprivileged
                .iter()
                .chain(&plan.privileged)
                .filter(|resource| resource.resource_type().starts_with("dock_"))
                .map(|resource| declarative::ResourceKey {
                    resource_type: resource.resource_type().into(),
                    id: resource.id(),
                }),
        );
        plan.add_post_action_for("Dock".to_string(), parents);
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

fn convert_default_value(value: &SchemaDefaultValue) -> Result<ResDefaultValue> {
    Ok(match value {
        SchemaDefaultValue::Bool(b) => ResDefaultValue::Bool(*b),
        SchemaDefaultValue::Int(i) => ResDefaultValue::Int(*i),
        SchemaDefaultValue::Float(f) if f.is_finite() => ResDefaultValue::Float(*f),
        SchemaDefaultValue::Float(_) => {
            bail!("Non-finite defaults are not supported; no replacement was scheduled")
        }
        SchemaDefaultValue::String(s) => ResDefaultValue::String(s.clone()),
        SchemaDefaultValue::Array(_) => {
            bail!("Array defaults are not supported; no replacement was scheduled")
        }
    })
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

    for name in IMPLEMENTED_STAGES {
        let description = match *name {
            "homebrew" => "Existing Homebrew prerequisite (automatic installation blocked)",
            "packages" => "Configured Homebrew packages (aliases: brew, essential)",
            "cellar" => "Configured synchronization blocked pending preservation/recovery",
            "dotfiles" => "Configured synchronization blocked pending safe acquisition",
            "symlinks" => "Native dotfile symlinks (alias: stow)",
            "ecosystem" => "pnpm globals, gh and VS Code extensions (alias: pnpm)",
            "defaults" => "Scalar macOS defaults",
            "dock" => "Dock apps and folders",
            "handlers" => "File handlers",
            _ => unreachable!("Every supported stage has a description"),
        };
        println!("  {:<15} {}", name.bold(), description.dimmed());
    }
    for stage in NovaStage::all() {
        if !IMPLEMENTED_STAGES.contains(&normalize_stage(stage.name())) {
            println!(
                "  {:<15} {} (planned)",
                stage.name().bold(),
                stage.description().dimmed()
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
