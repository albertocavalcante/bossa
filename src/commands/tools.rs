//! Tool installation and management commands.
//!
//! This module provides commands for installing development tools from various sources:
//! - HTTP URLs pointing to tar.gz archives
//! - Container images (via podman/docker)
//! - GitHub releases
//! - Cargo (crates.io or git)
//! - npm/pnpm global packages
//!
//! Tools can be installed imperatively via CLI or declaratively via config.toml.
//! Tools can declare dependencies on other tools, which are installed first.

use crate::Context;
use crate::cli::ToolsCommand;
use crate::schema::{
    BossaConfig, ContainerMeta, InstalledTool, ToolDefinition, ToolSource, ToolsConfig,
};
use crate::ui;
use anyhow::{Context as _, Result, bail};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Maximum download size (500 MB).
const MAX_DOWNLOAD_SIZE: u64 = 500 * 1024 * 1024;

// =============================================================================
// Dependency Resolution (Topological Sort)
// =============================================================================

/// Sort tools by dependencies using Kahn's algorithm (topological sort).
/// Returns tools in order such that dependencies come before dependents.
fn sort_by_dependencies<'a>(
    tools: Vec<(&'a String, &'a ToolDefinition)>,
) -> Result<Vec<(&'a String, &'a ToolDefinition)>> {
    // Build adjacency list and in-degree count
    let tool_names: HashSet<&str> = tools.iter().map(|(name, _)| name.as_str()).collect();

    // Calculate in-degree for each tool (number of dependencies pointing to it)
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for (name, def) in &tools {
        in_degree.entry(name.as_str()).or_insert(0);
        for dep in &def.depends {
            // Only count dependencies that are in our tool set
            if tool_names.contains(dep.as_str()) {
                *in_degree.entry(name.as_str()).or_insert(0) += 1;
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .push(name.as_str());
            }
        }
    }

    // Start with tools that have no dependencies (in-degree = 0)
    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|&(_, degree)| *degree == 0)
        .map(|(&name, _)| name)
        .collect();
    queue.sort_unstable(); // Deterministic order

    let mut sorted: Vec<(&String, &ToolDefinition)> = Vec::new();
    let mut processed = 0;

    while let Some(name) = queue.pop() {
        // Find the original (name, def) pair
        if let Some(&(orig_name, def)) = tools.iter().find(|(n, _)| n.as_str() == name) {
            sorted.push((orig_name, def));
            processed += 1;

            // Reduce in-degree for dependents
            if let Some(deps) = dependents.get(name) {
                for dep in deps {
                    if let Some(degree) = in_degree.get_mut(dep) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push(dep);
                        }
                    }
                }
            }
        }
    }

    // Check for cycles
    if processed != tools.len() {
        let unprocessed: Vec<_> = in_degree
            .iter()
            .filter(|&(_, d)| *d > 0)
            .map(|(&n, _)| n)
            .collect();
        bail!(
            "Circular dependency detected among tools: {}",
            unprocessed.join(", ")
        );
    }

    Ok(sorted)
}

/// Check if all dependencies of a tool are satisfied (installed).
fn check_dependencies(
    def: &ToolDefinition,
    state: &ToolsConfig,
    tools_to_install: &HashSet<&str>,
) -> Result<Vec<String>> {
    let mut missing = Vec::new();

    for dep in &def.depends {
        // Check if dependency is already installed
        let is_installed = state
            .get(dep)
            .is_some_and(|t| PathBuf::from(&t.install_path).exists());

        // Check if dependency is in the list of tools to be installed
        let will_be_installed = tools_to_install.contains(dep.as_str());

        // Check if dependency is available on the system (e.g., npm, pnpm)
        let is_system_available = Command::new(dep)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());

        if !is_installed && !will_be_installed && !is_system_available {
            missing.push(dep.clone());
        }
    }

    Ok(missing)
}

/// Run the tools command.
pub fn run(ctx: &Context, cmd: ToolsCommand) -> Result<()> {
    match cmd {
        ToolsCommand::Install {
            name,
            url,
            binary,
            path,
            install_dir,
            force,
        } => install_from_url(
            ctx,
            &name,
            &url,
            binary.as_deref(),
            path.as_deref(),
            install_dir.as_deref(),
            force,
        ),
        ToolsCommand::InstallContainer {
            name,
            image,
            package,
            binary_path,
            package_manager,
            runtime,
            dependencies,
            pre_install,
            keep_container,
            install_dir,
            force,
        } => install_from_container(
            ctx,
            &name,
            &image,
            package.as_deref(),
            &binary_path,
            package_manager.as_deref(),
            &runtime,
            dependencies.as_deref(),
            pre_install.as_deref(),
            keep_container,
            install_dir.as_deref(),
            force,
        ),
        ToolsCommand::Apply {
            tools,
            dry_run,
            force,
        } => apply(ctx, &tools, dry_run, force),
        ToolsCommand::List { all } => list(ctx, all),
        ToolsCommand::Status { name } => status(ctx, &name),
        ToolsCommand::Uninstall { name } => uninstall(ctx, &name),
        ToolsCommand::Outdated { tools, json } => outdated(ctx, &tools, json),
    }
}

// =============================================================================
// Apply Command (Declarative)
// =============================================================================

/// Apply tools from config file.
fn apply(ctx: &Context, filter_tools: &[String], dry_run: bool, force: bool) -> Result<()> {
    let config = BossaConfig::load()?;
    let mut state = ToolsConfig::load()?;

    // Validate all definitions
    config.tools.validate()?;

    // Filter tools if specified
    let tools_to_apply: Vec<_> = if filter_tools.is_empty() {
        config.tools.enabled_tools().collect()
    } else {
        let filter_set: HashSet<_> = filter_tools
            .iter()
            .map(std::string::String::as_str)
            .collect();
        config
            .tools
            .enabled_tools()
            .filter(|(name, _)| filter_set.contains(name.as_str()))
            .collect()
    };

    if tools_to_apply.is_empty() {
        if filter_tools.is_empty() {
            ui::info("No tools defined in config. Add tools to [tools] section.");
        } else {
            ui::warn("No matching tools found in config.");
        }
        return Ok(());
    }

    // Sort tools by dependencies (topological sort)
    let tools_to_apply = sort_by_dependencies(tools_to_apply)?;

    // Collect names of tools we'll install for dependency checking
    let tools_to_install: HashSet<&str> = tools_to_apply
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();

    if !ctx.quiet {
        ui::header("Applying Tools");
        println!();
    }

    let mut installed = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for (name, def) in tools_to_apply {
        // Check dependencies are satisfied
        let missing_deps = check_dependencies(def, &state, &tools_to_install)?;
        if !missing_deps.is_empty() {
            ui::error(&format!(
                "  ✗ {} missing dependencies: {}",
                name,
                missing_deps.join(", ")
            ));
            failed += 1;
            continue;
        }
        // Check platform availability
        if !def.is_available_for_current_platform() {
            if !ctx.quiet {
                ui::dim(&format!("  ⊘ {name} (not available for this platform)"));
            }
            skipped += 1;
            continue;
        }

        // Check if already installed
        let is_installed = state
            .get(name)
            .is_some_and(|t| PathBuf::from(&t.install_path).exists());

        if is_installed && !force {
            if !ctx.quiet {
                ui::dim(&format!("  ✓ {name} (already installed)"));
            }
            skipped += 1;
            continue;
        }

        if dry_run {
            ui::info(&format!("  Would install: {} ({})", name, def.description));
            continue;
        }

        if !ctx.quiet {
            ui::info(&format!("  Installing {name}..."));
        }

        match install_from_definition(ctx, name, def, &config.tools) {
            Ok(installed_tool) => {
                state.insert(name.clone(), installed_tool);
                state.save()?;

                if !ctx.quiet {
                    ui::success(&format!("  ✓ {name} installed"));
                    if let Some(ref msg) = def.post_install {
                        println!();
                        ui::dim(msg);
                        println!();
                    }
                }
                installed += 1;
            }
            Err(e) => {
                ui::error(&format!("  ✗ {name} failed: {e}"));
                failed += 1;
            }
        }
    }

    if !ctx.quiet && !dry_run {
        println!();
        ui::header("Summary");
        ui::kv("Installed", &installed.to_string());
        ui::kv("Skipped", &skipped.to_string());
        if failed > 0 {
            ui::kv("Failed", &failed.to_string());
        }
    }

    if failed > 0 {
        bail!("{failed} tool(s) failed to install");
    }

    Ok(())
}

/// Install a tool from its declarative definition.
fn install_from_definition(
    ctx: &Context,
    name: &str,
    def: &ToolDefinition,
    tools_section: &crate::schema::ToolsSection,
) -> Result<InstalledTool> {
    let install_dir = def
        .install_dir
        .as_deref()
        .unwrap_or(&tools_section.install_dir);
    let install_path = get_install_dir(Some(install_dir))?;
    fs::create_dir_all(&install_path)?;

    let binary_name = def.effective_binary(name);

    match def.source {
        ToolSource::Http => {
            // Use platform-specific URL if available
            let url = def
                .get_effective_url(name)
                .context("URL required for HTTP source (provide 'url' or 'base_url' + 'path')")?;

            let archive_path = def
                .archive_path
                .as_ref()
                .map(|p| def.expand_template(p, name));

            let data = download_file(&url)?;

            // Use platform-specific archive type if available
            let archive_type = def.get_effective_archive_type();

            // Determine how to handle the downloaded data based on archive_type
            let binary_data = if archive_type == "binary" {
                // Direct binary download - no extraction needed
                data
            } else {
                // Extract from archive based on type
                match archive_type.as_str() {
                    "zip" => extract_zip(&data, &binary_name, archive_path.as_deref())?,
                    _ => extract_targz(&data, &binary_name, archive_path.as_deref())?,
                }
            };

            let binary_path = install_binary(&binary_data, &binary_name, &install_path)?;

            Ok(InstalledTool {
                url,
                binary: binary_name,
                install_path: binary_path.to_string_lossy().to_string(),
                installed_at: chrono::Utc::now().to_rfc3339(),
                source: "http".to_string(),
                container: None,
            })
        }

        ToolSource::Container => {
            let image = def
                .image
                .as_ref()
                .context("Image required for container source")?;
            let container_binary_path = def
                .binary_path
                .as_ref()
                .context("binary_path required for container source")?;

            let runtime = def.runtime.as_deref().unwrap_or(&tools_section.runtime);

            check_runtime_available(runtime)?;

            let container_name = format!("bossa-tools-{}-{}", name, std::process::id());
            let pkg_manager = def
                .package_manager
                .clone()
                .unwrap_or_else(|| detect_package_manager(image));

            // Combine package and packages
            let all_packages: Vec<String> = def
                .package
                .iter()
                .cloned()
                .chain(def.packages.iter().cloned())
                .collect();

            let install_script = build_install_script(
                all_packages.first().map(std::string::String::as_str),
                &pkg_manager,
                if all_packages.len() > 1 {
                    Some(&all_packages[1..])
                } else {
                    None
                },
                def.pre_install.as_deref(),
                container_binary_path,
            );

            let run_result = run_container(runtime, image, &container_name, &install_script, ctx)?;

            if !run_result.success {
                let _ = remove_container(runtime, &container_name);
                bail!(
                    "Container command failed:\n{}",
                    run_result.stderr.trim_end()
                );
            }

            let local_binary_path = install_path.join(&binary_name);
            copy_from_container(
                runtime,
                &container_name,
                container_binary_path,
                &local_binary_path,
            )?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&local_binary_path)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&local_binary_path, perms)?;
            }

            remove_container(runtime, &container_name)?;

            Ok(InstalledTool {
                url: image.clone(),
                binary: binary_name,
                install_path: local_binary_path.to_string_lossy().to_string(),
                installed_at: chrono::Utc::now().to_rfc3339(),
                source: "container".to_string(),
                container: Some(ContainerMeta {
                    image: image.clone(),
                    package: def.package.clone(),
                    binary_path: container_binary_path.clone(),
                    package_manager: Some(pkg_manager),
                    runtime: runtime.to_string(),
                }),
            })
        }

        ToolSource::GithubRelease => {
            let repo = def
                .repo
                .as_ref()
                .context("repo required for github-release source")?;
            let version = def
                .version
                .as_ref()
                .context("version required for github-release source")?;

            // Build download URL from repo and asset pattern
            let asset_pattern = def.asset.as_ref().map(|a| def.expand_template(a, name));

            let url = if let Some(asset) = asset_pattern {
                format!("https://github.com/{repo}/releases/download/{version}/{asset}")
            } else {
                bail!("asset pattern required for github-release source");
            };

            let archive_path = def
                .archive_path
                .as_ref()
                .map(|p| def.expand_template(p, name));

            let data = download_file(&url)?;

            // Determine how to handle the downloaded data based on archive_type
            let binary_data = if def.is_binary_download() {
                data
            } else {
                let ext = def.effective_extension();
                match ext.as_str() {
                    "zip" => extract_zip(&data, &binary_name, archive_path.as_deref())?,
                    _ => extract_targz(&data, &binary_name, archive_path.as_deref())?,
                }
            };

            let binary_path = install_binary(&binary_data, &binary_name, &install_path)?;

            Ok(InstalledTool {
                url,
                binary: binary_name,
                install_path: binary_path.to_string_lossy().to_string(),
                installed_at: chrono::Utc::now().to_rfc3339(),
                source: "github-release".to_string(),
                container: None,
            })
        }

        ToolSource::Cargo => {
            // Check cargo is available
            check_cargo_available()?;

            // Build the cargo install command
            let cargo_result = run_cargo_install(def, &install_path, ctx)?;

            if !cargo_result.success {
                bail!("cargo install failed:\n{}", cargo_result.stderr.trim_end());
            }

            // Find the installed binary
            let binary_full_path = install_path.join(&binary_name);
            if !binary_full_path.exists() {
                bail!(
                    "Binary '{}' not found at {} after cargo install. Check the 'binary' field.",
                    binary_name,
                    binary_full_path.display()
                );
            }

            // Build source URL for tracking
            let source_url = if let Some(ref git) = def.git {
                git.clone()
            } else if let Some(ref crate_name) = def.crate_name {
                format!("https://crates.io/crates/{crate_name}")
            } else {
                "cargo".to_string()
            };

            Ok(InstalledTool {
                url: source_url,
                binary: binary_name,
                install_path: binary_full_path.to_string_lossy().to_string(),
                installed_at: chrono::Utc::now().to_rfc3339(),
                source: "cargo".to_string(),
                container: None,
            })
        }

        ToolSource::Npm => {
            // Determine package manager: prefer pnpm, fall back to npm
            let (pm, pm_name) = detect_npm_package_manager();

            // Get package name (defaults to tool name)
            let npm_package = def.npm_package.as_deref().unwrap_or(name);

            // Run npm/pnpm install
            let npm_result = run_npm_install(
                &pm,
                npm_package,
                def.version.as_deref(),
                def.needs_scripts,
                ctx,
            )?;

            if !npm_result.success {
                bail!(
                    "{} install failed:\n{}",
                    pm_name,
                    npm_result.stderr.trim_end()
                );
            }

            // Find where npm/pnpm installed the binary
            let binary_path = find_npm_binary(&pm, &binary_name)?;

            Ok(InstalledTool {
                url: format!("npm:{npm_package}"),
                binary: binary_name,
                install_path: binary_path.to_string_lossy().to_string(),
                installed_at: chrono::Utc::now().to_rfc3339(),
                source: "npm".to_string(),
                container: None,
            })
        }
    }
}

// =============================================================================
// Imperative Install Commands
// =============================================================================

/// Install a tool from a URL.
fn install_from_url(
    ctx: &Context,
    name: &str,
    url: &str,
    binary: Option<&str>,
    archive_path: Option<&str>,
    install_dir: Option<&str>,
    force: bool,
) -> Result<()> {
    let binary_name = binary.unwrap_or(name);

    // Validate URL
    if !url.ends_with(".tar.gz") && !url.ends_with(".tgz") {
        bail!("URL must point to a .tar.gz archive");
    }

    // Load existing config
    let mut config = ToolsConfig::load()?;

    // Check if already installed
    if let Some(existing) = config.get(name) {
        if !force {
            bail!(
                "Tool '{}' is already installed at {}. Use --force to reinstall.",
                name,
                existing.install_path
            );
        }
        if !ctx.quiet {
            ui::warn(&format!("Reinstalling '{name}' (--force)"));
        }
    }

    // Determine installation directory
    let install_path = get_install_dir(install_dir)?;
    fs::create_dir_all(&install_path)?;

    if !ctx.quiet {
        ui::info(&format!("Installing {name} from {url}"));
    }

    // Download the archive
    if !ctx.quiet {
        ui::info("Downloading archive...");
    }
    let data = download_file(url)?;
    if !ctx.quiet {
        ui::info(&format!(
            "Downloaded {}",
            ui::format_size(data.len() as u64)
        ));
    }

    // Extract the binary
    if !ctx.quiet {
        ui::info(&format!("Extracting binary '{binary_name}'..."));
    }
    let binary_data = extract_targz(&data, binary_name, archive_path)?;
    if !ctx.quiet {
        ui::info(&format!(
            "Extracted {}",
            ui::format_size(binary_data.len() as u64)
        ));
    }

    // Install the binary
    let binary_path = install_binary(&binary_data, binary_name, &install_path)?;
    if !ctx.quiet {
        ui::success(&format!("Installed {} to {}", name, binary_path.display()));
    }

    // Save to config
    let installed_tool = InstalledTool {
        url: url.to_string(),
        binary: binary_name.to_string(),
        install_path: binary_path.to_string_lossy().to_string(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        source: "http".to_string(),
        container: None,
    };
    config.insert(name.to_string(), installed_tool);
    config.save()?;

    if !ctx.quiet {
        ui::success(&format!("Tool '{name}' installed successfully!"));
    }

    Ok(())
}

/// Install a tool from a container image.
#[allow(clippy::too_many_arguments)]
fn install_from_container(
    ctx: &Context,
    name: &str,
    image: &str,
    package: Option<&str>,
    binary_path: &str,
    package_manager: Option<&str>,
    runtime: &str,
    dependencies: Option<&[String]>,
    pre_install: Option<&str>,
    keep_container: bool,
    install_dir: Option<&str>,
    force: bool,
) -> Result<()> {
    // Validate runtime
    if runtime != "podman" && runtime != "docker" {
        bail!("Runtime must be 'podman' or 'docker', got '{runtime}'");
    }

    // Check if runtime is available
    check_runtime_available(runtime)?;

    // Load existing config
    let mut config = ToolsConfig::load()?;

    // Check if already installed
    if let Some(existing) = config.get(name) {
        if !force {
            bail!(
                "Tool '{}' is already installed at {}. Use --force to reinstall.",
                name,
                existing.install_path
            );
        }
        if !ctx.quiet {
            ui::warn(&format!("Reinstalling '{name}' (--force)"));
        }
    }

    // Determine installation directory
    let local_install_dir = get_install_dir(install_dir)?;
    fs::create_dir_all(&local_install_dir)?;

    // Extract binary name from path
    let binary_name = PathBuf::from(binary_path)
        .file_name()
        .and_then(|n| n.to_str())
        .map_or_else(|| name.to_string(), std::string::ToString::to_string);

    if !ctx.quiet {
        ui::header(&format!("Installing {name} from container"));
        ui::kv("Image", image);
        if let Some(pkg) = package {
            ui::kv("Package", pkg);
        }
        ui::kv("Binary path", binary_path);
        ui::kv("Runtime", runtime);
        println!();
    }

    // Generate a unique container name
    let container_name = format!("bossa-tools-{}-{}", name, std::process::id());

    // Detect or use specified package manager
    let pkg_manager = if let Some(pm) = package_manager {
        pm.to_string()
    } else {
        detect_package_manager(image)
    };

    // Build the container command
    let install_script = build_install_script(
        package,
        &pkg_manager,
        dependencies,
        pre_install,
        binary_path,
    );

    if !ctx.quiet {
        ui::info(&format!("Creating container from {image}..."));
    }

    // Run container with install script
    let run_result = run_container(runtime, image, &container_name, &install_script, ctx)?;

    if !run_result.success {
        // Clean up container on failure
        if !keep_container {
            let _ = remove_container(runtime, &container_name);
        }
        bail!(
            "Container command failed:\n{}",
            run_result.stderr.trim_end()
        );
    }

    if !ctx.quiet {
        ui::info("Extracting binary from container...");
    }

    // Copy binary from container
    let local_binary_path = local_install_dir.join(&binary_name);
    copy_from_container(runtime, &container_name, binary_path, &local_binary_path)?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&local_binary_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&local_binary_path, perms)?;
    }

    // Clean up container
    if !keep_container {
        if !ctx.quiet {
            ui::info("Cleaning up container...");
        }
        remove_container(runtime, &container_name)?;
    } else if !ctx.quiet {
        ui::info(&format!("Container '{container_name}' kept for debugging"));
    }

    if !ctx.quiet {
        ui::success(&format!(
            "Installed {} to {}",
            name,
            local_binary_path.display()
        ));
    }

    // Save to config
    let installed_tool = InstalledTool {
        url: image.to_string(),
        binary: binary_name,
        install_path: local_binary_path.to_string_lossy().to_string(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        source: "container".to_string(),
        container: Some(ContainerMeta {
            image: image.to_string(),
            package: package.map(std::string::ToString::to_string),
            binary_path: binary_path.to_string(),
            package_manager: Some(pkg_manager),
            runtime: runtime.to_string(),
        }),
    };
    config.insert(name.to_string(), installed_tool);
    config.save()?;

    if !ctx.quiet {
        ui::success(&format!("Tool '{name}' installed successfully!"));
    }

    Ok(())
}

// =============================================================================
// List, Status, Uninstall
// =============================================================================

/// List installed tools.
fn list(_ctx: &Context, show_all: bool) -> Result<()> {
    let state = ToolsConfig::load()?;
    let config = BossaConfig::load().ok();

    // Collect defined tools from config
    let defined_tools: HashSet<_> = config
        .as_ref()
        .map(|c| c.tools.definitions.keys().cloned().collect())
        .unwrap_or_default();

    if state.tools.is_empty() && (!show_all || defined_tools.is_empty()) {
        ui::info("No tools installed yet.");
        ui::info(
            "Use 'bossa tools install', 'bossa tools install-container', or 'bossa tools apply' to install tools.",
        );
        return Ok(());
    }

    ui::header("Installed Tools");
    println!();

    for (name, tool) in &state.tools {
        let exists = PathBuf::from(&tool.install_path).exists();
        let status = if exists { "✓" } else { "✗ (missing)" };
        let in_config = if defined_tools.contains(name) {
            " [config]"
        } else {
            ""
        };

        println!("  {status} {name}{in_config}");
        ui::kv("    Binary", &tool.binary);
        ui::kv("    Path", &tool.install_path);
        ui::kv("    Source", &tool.source);

        if let Some(ref container) = tool.container {
            ui::kv("    Image", &container.image);
            if let Some(ref pkg) = container.package {
                ui::kv("    Package", pkg);
            }
        } else {
            ui::kv("    URL", &tool.url);
        }

        ui::kv("    Installed", &tool.installed_at);
        println!();
    }

    // Show tools from config that aren't installed
    if show_all {
        let installed_names: HashSet<_> = state.tools.keys().cloned().collect();
        let not_installed: Vec<_> = defined_tools
            .iter()
            .filter(|name| !installed_names.contains(*name))
            .collect();

        if !not_installed.is_empty() {
            ui::header("Defined in Config (Not Installed)");
            println!();

            if let Some(ref cfg) = config {
                for name in not_installed {
                    if let Some(def) = cfg.tools.get(name) {
                        let status = if def.enabled { "○" } else { "○ (disabled)" };
                        println!("  {status} {name}");
                        if !def.description.is_empty() {
                            ui::kv("    Description", &def.description);
                        }
                        ui::kv("    Source", &format!("{:?}", def.source).to_lowercase());
                        println!();
                    }
                }
            }

            ui::info("Run 'bossa tools apply' to install these tools.");
        }
    }

    Ok(())
}

/// Show status of a specific tool.
fn status(_ctx: &Context, name: &str) -> Result<()> {
    let state = ToolsConfig::load()?;
    let config = BossaConfig::load().ok();

    let installed = state.get(name);
    let defined = config.as_ref().and_then(|c| c.tools.get(name));

    if installed.is_none() && defined.is_none() {
        bail!("Tool '{name}' not found (not installed and not defined in config)");
    }

    ui::header(&format!("Tool: {name}"));
    println!();

    // Show installed status
    if let Some(tool) = installed {
        let binary_path = PathBuf::from(&tool.install_path);
        let exists = binary_path.exists();

        if exists {
            ui::success("Status: Installed");
        } else {
            ui::error("Status: Missing (binary not found)");
        }

        println!();
        ui::kv("Binary", &tool.binary);
        ui::kv("Install Path", &tool.install_path);
        ui::kv("Source Type", &tool.source);
        ui::kv("Installed At", &tool.installed_at);

        if let Some(ref container) = tool.container {
            println!();
            ui::header("Container Details");
            ui::kv("Image", &container.image);
            if let Some(ref pkg) = container.package {
                ui::kv("Package", pkg);
            }
            ui::kv("Binary Path", &container.binary_path);
            if let Some(ref pm) = container.package_manager {
                ui::kv("Package Manager", pm);
            }
            ui::kv("Runtime", &container.runtime);
        } else {
            ui::kv("Source URL", &tool.url);
        }

        if exists && let Ok(metadata) = fs::metadata(&binary_path) {
            ui::kv("File Size", &ui::format_size(metadata.len()));
        }
    } else {
        ui::warn("Status: Not installed");
    }

    // Show config definition
    if let Some(def) = defined {
        println!();
        ui::header("Config Definition");
        if !def.description.is_empty() {
            ui::kv("Description", &def.description);
        }
        ui::kv("Source", &format!("{:?}", def.source).to_lowercase());
        ui::kv("Enabled", if def.enabled { "yes" } else { "no" });

        if let Some(ref v) = def.version {
            ui::kv("Version", v);
        }

        match def.source {
            ToolSource::Http => {
                if let Some(ref url) = def.url {
                    ui::kv("URL", &def.expand_version(url));
                }
            }
            ToolSource::Container => {
                if let Some(ref image) = def.image {
                    ui::kv("Image", image);
                }
                if let Some(ref pkg) = def.package {
                    ui::kv("Package", pkg);
                }
                if let Some(ref bp) = def.binary_path {
                    ui::kv("Binary Path", bp);
                }
            }
            ToolSource::GithubRelease => {
                if let Some(ref repo) = def.repo {
                    ui::kv("Repo", repo);
                }
                if let Some(ref asset) = def.asset {
                    ui::kv("Asset", &def.expand_version(asset));
                }
            }
            ToolSource::Cargo => {
                if let Some(ref crate_name) = def.crate_name {
                    ui::kv("Crate", crate_name);
                }
                if let Some(ref git) = def.git {
                    ui::kv("Git", git);
                }
                if !def.features.is_empty() {
                    ui::kv("Features", &def.features.join(", "));
                }
                if def.all_features {
                    ui::kv("All Features", "yes");
                }
                if def.locked {
                    ui::kv("Locked", "yes");
                }
            }
            ToolSource::Npm => {
                if let Some(ref npm_package) = def.npm_package {
                    ui::kv("Package", npm_package);
                }
                if def.needs_scripts {
                    ui::kv("Needs Scripts", "yes");
                }
            }
        }

        // Show dependencies if any
        if !def.depends.is_empty() {
            ui::kv("Dependencies", &def.depends.join(", "));
        }
    }

    Ok(())
}

/// Uninstall a tool.
fn uninstall(ctx: &Context, name: &str) -> Result<()> {
    let mut config = ToolsConfig::load()?;

    let tool = config
        .remove(name)
        .context(format!("Tool '{name}' not found"))?;

    // Remove the binary file
    let binary_path = PathBuf::from(&tool.install_path);
    if binary_path.exists() {
        fs::remove_file(&binary_path)?;
        if !ctx.quiet {
            ui::info(&format!("Removed binary: {}", binary_path.display()));
        }
    } else if !ctx.quiet {
        ui::warn(&format!("Binary not found at: {}", binary_path.display()));
    }

    // Save config
    config.save()?;

    if !ctx.quiet {
        ui::success(&format!("Tool '{name}' uninstalled successfully!"));
    }

    Ok(())
}

// =============================================================================
// Outdated Command
// =============================================================================

use colored::Colorize;

/// Version check result for a tool
#[derive(Debug, Clone, serde::Serialize)]
struct VersionInfo {
    name: String,
    source: String,
    current: Option<String>,
    latest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl VersionInfo {
    fn is_outdated(&self) -> bool {
        match (&self.current, &self.latest) {
            (Some(current), Some(latest)) => {
                // Normalize versions for comparison
                let c = current.trim_start_matches('v');
                let l = latest.trim_start_matches('v');
                c != l
            }
            _ => false,
        }
    }

    fn status_icon(&self) -> colored::ColoredString {
        if self.error.is_some() {
            "?".yellow()
        } else if self.is_outdated() {
            "↑".yellow()
        } else {
            "✓".green()
        }
    }
}

/// Check for outdated tools
fn outdated(ctx: &Context, filter_tools: &[String], as_json: bool) -> Result<()> {
    let state = ToolsConfig::load()?;
    let config = BossaConfig::load().ok();

    // Collect tools to check
    let tools_to_check: Vec<(&String, Option<&ToolDefinition>)> = if filter_tools.is_empty() {
        // Check all installed tools
        state
            .tools
            .keys()
            .map(|name| {
                let def = config.as_ref().and_then(|c| c.tools.get(name));
                (name, def)
            })
            .collect()
    } else {
        // Check specified tools only
        filter_tools
            .iter()
            .filter_map(|name| {
                let def = config.as_ref().and_then(|c| c.tools.get(name));
                if state.tools.contains_key(name) || def.is_some() {
                    Some((name, def))
                } else {
                    if !ctx.quiet {
                        ui::warn(&format!("Tool '{name}' not found"));
                    }
                    None
                }
            })
            .collect()
    };

    if tools_to_check.is_empty() {
        if !ctx.quiet {
            ui::info("No tools to check.");
        }
        return Ok(());
    }

    if !ctx.quiet && !as_json {
        ui::header("Checking for updates");
        println!();
    }

    let mut results: Vec<VersionInfo> = Vec::new();

    for (name, def) in tools_to_check {
        let info = check_tool_version(name, def, &state);
        results.push(info);
    }

    // Output results
    if as_json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        // Print table header
        println!(
            "  {:<20} {:<12} {:<15} {:<15}",
            "Tool".bold(),
            "Source".bold(),
            "Current".bold(),
            "Latest".bold()
        );
        println!(
            "  {} {} {} {}",
            "─".repeat(20),
            "─".repeat(12),
            "─".repeat(15),
            "─".repeat(15)
        );

        let mut outdated_count = 0;
        let mut error_count = 0;

        for info in &results {
            let current = info.current.as_deref().unwrap_or("-");
            let latest = if let Some(ref err) = info.error {
                err.dimmed().to_string()
            } else {
                info.latest.as_deref().unwrap_or("-").to_string()
            };

            let latest_display = if info.is_outdated() {
                latest.yellow().to_string()
            } else {
                latest.dimmed().to_string()
            };

            println!(
                "  {:<20} {:<12} {:<15} {} {}",
                info.name,
                info.source.dimmed(),
                current,
                info.status_icon(),
                latest_display
            );

            if info.is_outdated() {
                outdated_count += 1;
            }
            if info.error.is_some() {
                error_count += 1;
            }
        }

        println!();

        // Summary
        if outdated_count > 0 {
            println!(
                "  {} {} tool(s) can be updated",
                "↑".yellow(),
                outdated_count
            );
            println!("  Run {} to update", "bossa tools apply --force".cyan());
        } else if error_count == 0 {
            println!("  {} All tools are up to date", "✓".green());
        }

        if error_count > 0 {
            println!(
                "  {} {} tool(s) could not be checked",
                "?".yellow(),
                error_count
            );
        }
    }

    Ok(())
}

/// Check version for a single tool
fn check_tool_version(
    name: &str,
    def: Option<&ToolDefinition>,
    _state: &ToolsConfig,
) -> VersionInfo {
    // Try to get current version from the binary
    let current = get_current_version(name);

    // Try to get latest version based on source
    let (source, latest, error) = if let Some(def) = def {
        match def.source {
            ToolSource::GithubRelease => {
                if let Some(ref repo) = def.repo {
                    match get_github_latest_release(repo) {
                        Ok(v) => ("github".to_string(), Some(v), None),
                        Err(e) => ("github".to_string(), None, Some(e.to_string())),
                    }
                } else {
                    (
                        "github".to_string(),
                        None,
                        Some("no repo defined".to_string()),
                    )
                }
            }
            ToolSource::Cargo => {
                if let Some(ref crate_name) = def.crate_name {
                    match get_crates_io_latest(crate_name) {
                        Ok(v) => ("cargo".to_string(), Some(v), None),
                        Err(e) => ("cargo".to_string(), None, Some(e.to_string())),
                    }
                } else if let Some(ref git) = def.git {
                    // For git sources, try to get latest tag
                    match get_git_latest_tag(git) {
                        Ok(v) => ("git".to_string(), Some(v), None),
                        Err(e) => ("git".to_string(), None, Some(e.to_string())),
                    }
                } else {
                    (
                        "cargo".to_string(),
                        None,
                        Some("no crate defined".to_string()),
                    )
                }
            }
            ToolSource::Http => {
                // For HTTP sources, we can't easily check for updates
                ("http".to_string(), def.version.clone(), None)
            }
            ToolSource::Container => ("container".to_string(), None, Some("n/a".to_string())),
            ToolSource::Npm => {
                let npm_package = def.npm_package.as_deref().unwrap_or(name);
                match get_npm_latest(npm_package) {
                    Ok(v) => ("npm".to_string(), Some(v), None),
                    Err(e) => ("npm".to_string(), None, Some(e.to_string())),
                }
            }
        }
    } else {
        // No definition, try to infer from installed tool
        ("unknown".to_string(), None, Some("no config".to_string()))
    };

    VersionInfo {
        name: name.to_string(),
        source,
        current,
        latest,
        error,
    }
}

/// Get current version by running the binary with --version
fn get_current_version(name: &str) -> Option<String> {
    let output = Command::new(name).arg("--version").output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse version from common formats:
    // "tool 1.2.3" or "tool version 1.2.3" or "1.2.3"
    extract_version(&stdout)
}

/// Extract version number from a string
fn extract_version(s: &str) -> Option<String> {
    // Try to find a semver-like pattern
    let re = regex::Regex::new(r"v?(\d+\.\d+(?:\.\d+)?(?:-[\w.]+)?)").ok()?;
    re.captures(s.lines().next()?)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Get latest release version from GitHub
fn get_github_latest_release(repo: &str) -> Result<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");

    let agent = ureq::Agent::new_with_defaults();
    let mut response = agent
        .get(&url)
        .header("User-Agent", "bossa-tools")
        .header("Accept", "application/vnd.github.v3+json")
        .call()
        .context("Failed to fetch GitHub releases")?;

    let body: serde_json::Value = response
        .body_mut()
        .read_json()
        .context("Failed to parse GitHub response")?;

    body["tag_name"]
        .as_str()
        .map(|s| s.trim_start_matches('v').to_string())
        .context("No tag_name in release")
}

/// Get latest version from crates.io
fn get_crates_io_latest(crate_name: &str) -> Result<String> {
    let url = format!("https://crates.io/api/v1/crates/{crate_name}");

    let agent = ureq::Agent::new_with_defaults();
    let mut response = agent
        .get(&url)
        .header("User-Agent", "bossa-tools")
        .call()
        .context("Failed to fetch crates.io")?;

    let body: serde_json::Value = response
        .body_mut()
        .read_json()
        .context("Failed to parse crates.io response")?;

    body["crate"]["newest_version"]
        .as_str()
        .map(std::string::ToString::to_string)
        .context("No newest_version in response")
}

/// Get latest tag from a git repository
fn get_git_latest_tag(repo_url: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["ls-remote", "--tags", "--sort=-v:refname", repo_url])
        .output()
        .context("Failed to run git ls-remote")?;

    if !output.status.success() {
        bail!("git ls-remote failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse the first (newest) tag
    // Format: "sha\trefs/tags/v1.2.3" or "sha\trefs/tags/v1.2.3^{}"
    for line in stdout.lines() {
        if let Some(tag_ref) = line.split('\t').nth(1) {
            let tag = tag_ref
                .trim_start_matches("refs/tags/")
                .trim_end_matches("^{}");

            // Skip non-version tags
            if tag
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == 'v')
            {
                return Ok(tag.trim_start_matches('v').to_string());
            }
        }
    }

    bail!("No version tags found")
}

// =============================================================================
// Helper functions - General
// =============================================================================

/// Get the installation directory.
fn get_install_dir(custom_dir: Option<&str>) -> Result<PathBuf> {
    if let Some(dir) = custom_dir {
        return Ok(crate::paths::expand(dir));
    }

    dirs::home_dir()
        .map(|h| h.join(".local").join("bin"))
        .context("Cannot determine home directory")
}

/// Download a file from a URL.
fn download_file(url: &str) -> Result<Vec<u8>> {
    let agent = ureq::Agent::new_with_defaults();

    let mut response = agent
        .get(url)
        .header("User-Agent", "bossa-tools")
        .call()
        .context("Failed to download file")?;

    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_SIZE)
        .read_to_vec()
        .context("Failed to read response body")?;

    Ok(bytes)
}

/// Extract a specific binary from a tar.gz archive.
fn extract_targz(data: &[u8], binary_name: &str, archive_path: Option<&str>) -> Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let decoder = GzDecoder::new(data);
    let mut archive = Archive::new(decoder);

    // Build target paths to search for
    let target_with_path = archive_path.map_or_else(
        || binary_name.to_string(),
        |p| format!("{}/{}", p.trim_matches('/'), binary_name),
    );

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let path_str = path.to_string_lossy();

        // Check if this entry matches our target
        let is_match = path_str.ends_with(&format!("/{binary_name}"))
            || path_str == binary_name
            || path_str.ends_with(&format!("/{target_with_path}"))
            || path_str == target_with_path
            || path.file_name().is_some_and(|n| n == binary_name);

        if is_match {
            // Found the binary, read its contents
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents)?;
            return Ok(contents);
        }
    }

    bail!(
        "Binary '{binary_name}' not found in archive. Use --path/archive_path to specify the directory inside the archive."
    )
}

/// Extract a specific binary from a zip archive.
fn extract_zip(data: &[u8], binary_name: &str, archive_path: Option<&str>) -> Result<Vec<u8>> {
    use std::io::Cursor;

    let reader = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)?;

    // Build target paths to search for
    let target_with_path = archive_path.map_or_else(
        || binary_name.to_string(),
        |p| format!("{}/{}", p.trim_matches('/'), binary_name),
    );

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let path_str = file.name();

        // Check if this entry matches our target
        let is_match = path_str.ends_with(&format!("/{binary_name}"))
            || path_str == binary_name
            || path_str.ends_with(&format!("/{target_with_path}"))
            || path_str == target_with_path
            || std::path::Path::new(path_str)
                .file_name()
                .is_some_and(|n| n == binary_name);

        if is_match && !file.is_dir() {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;
            return Ok(contents);
        }
    }

    bail!(
        "Binary '{binary_name}' not found in zip archive. Use archive_path to specify the directory inside the archive."
    )
}

/// Install a binary to the specified directory.
fn install_binary(data: &[u8], name: &str, install_dir: &std::path::Path) -> Result<PathBuf> {
    let binary_path = install_dir.join(name);

    // Write the binary
    fs::write(&binary_path, data)?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&binary_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&binary_path, perms)?;
    }

    Ok(binary_path)
}

// =============================================================================
// Helper functions - Container
// =============================================================================

/// Check if the container runtime is available.
fn check_runtime_available(runtime: &str) -> Result<()> {
    let output = Command::new(runtime).arg("--version").output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        Ok(_) => bail!(
            "'{runtime}' is installed but returned an error. Check your {runtime} installation."
        ),
        Err(_) => bail!(
            "'{runtime}' not found. Please install {runtime} first.\n\
             On macOS: brew install {runtime}\n\
             On Fedora: sudo dnf install {runtime}"
        ),
    }
}

/// Detect package manager based on image name.
fn detect_package_manager(image: &str) -> String {
    let image_lower = image.to_lowercase();

    if image_lower.contains("alpine") {
        "apk".to_string()
    } else if image_lower.contains("debian") || image_lower.contains("ubuntu") {
        "apt".to_string()
    } else if image_lower.contains("ubi-minimal") || image_lower.contains("micro") {
        "microdnf".to_string()
    } else if image_lower.contains("fedora")
        || image_lower.contains("rhel")
        || image_lower.contains("centos")
        || image_lower.contains("ubi")
    {
        "dnf".to_string()
    } else {
        // Default to dnf for unknown images
        "dnf".to_string()
    }
}

/// Build the installation script to run inside the container.
fn build_install_script(
    package: Option<&str>,
    pkg_manager: &str,
    dependencies: Option<&[String]>,
    pre_install: Option<&str>,
    binary_path: &str,
) -> String {
    let mut script = String::from("set -e\n");

    // Pre-install command
    if let Some(pre) = pre_install {
        script.push_str(&format!("{pre}\n"));
    }

    // Install packages if specified
    if let Some(pkg) = package {
        let all_packages: Vec<&str> = std::iter::once(pkg)
            .chain(
                dependencies
                    .map(|d| d.iter().map(std::string::String::as_str))
                    .into_iter()
                    .flatten(),
            )
            .collect();

        let packages_str = all_packages.join(" ");

        let install_cmd = match pkg_manager {
            "apk" => format!("apk add --no-cache {packages_str}"),
            "apt" => format!(
                "apt-get update && apt-get install -y --no-install-recommends {packages_str}"
            ),
            "microdnf" => format!("microdnf install -y {packages_str} && microdnf clean all"),
            "dnf" => format!("dnf install -y {packages_str} && dnf clean all"),
            "yum" => format!("yum install -y {packages_str} && yum clean all"),
            other => format!("{other} install -y {packages_str}"),
        };

        script.push_str(&format!("{install_cmd}\n"));
    }

    // Verify binary exists
    script.push_str(&format!(
        "test -f {binary_path} || (echo 'Binary not found: {binary_path}' && exit 1)\n"
    ));

    script
}

struct ContainerRunResult {
    success: bool,
    #[allow(dead_code)]
    stdout: String,
    stderr: String,
}

/// Run a container with the given script.
fn run_container(
    runtime: &str,
    image: &str,
    container_name: &str,
    script: &str,
    ctx: &Context,
) -> Result<ContainerRunResult> {
    if ctx.verbose > 0 {
        ui::dim(&format!("Running script in container:\n{script}"));
    }

    let output = Command::new(runtime)
        .args(["run", "--name", container_name, image, "sh", "-c", script])
        .output()
        .context(format!("Failed to run {runtime} container"))?;

    Ok(ContainerRunResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Copy a file from a container to the local filesystem.
fn copy_from_container(
    runtime: &str,
    container_name: &str,
    container_path: &str,
    local_path: &Path,
) -> Result<()> {
    let source = format!("{container_name}:{container_path}");

    let output = Command::new(runtime)
        .args(["cp", &source, &local_path.to_string_lossy()])
        .output()
        .context(format!(
            "Failed to copy file from container using {runtime}"
        ))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to copy from container: {}", stderr.trim());
    }

    Ok(())
}

/// Remove a container.
fn remove_container(runtime: &str, container_name: &str) -> Result<()> {
    let output = Command::new(runtime)
        .args(["rm", "-f", container_name])
        .output()
        .context(format!("Failed to remove container using {runtime}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to remove container: {}", stderr.trim());
    }

    Ok(())
}

// =============================================================================
// Helper functions - Cargo
// =============================================================================

/// Check if cargo is available.
fn check_cargo_available() -> Result<()> {
    let output = Command::new("cargo").arg("--version").output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        Ok(_) => bail!("'cargo' is installed but returned an error. Check your Rust installation."),
        Err(_) => bail!(
            "'cargo' not found. Please install Rust first.\n\
             Visit: https://rustup.rs/\n\
             Or run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        ),
    }
}

struct CargoInstallResult {
    success: bool,
    #[allow(dead_code)]
    stdout: String,
    stderr: String,
}

/// Run cargo install with the given tool definition.
fn run_cargo_install(
    def: &ToolDefinition,
    install_root: &std::path::Path,
    ctx: &Context,
) -> Result<CargoInstallResult> {
    let mut args = vec!["install".to_string()];

    // Source: crate name or git
    if let Some(ref git) = def.git {
        args.push("--git".to_string());
        args.push(git.clone());

        // Git ref options (mutually exclusive, prefer in order: rev > tag > branch)
        if let Some(ref rev) = def.rev {
            args.push("--rev".to_string());
            args.push(rev.clone());
        } else if let Some(ref tag) = def.tag {
            args.push("--tag".to_string());
            args.push(tag.clone());
        } else if let Some(ref branch) = def.branch {
            args.push("--branch".to_string());
            args.push(branch.clone());
        }
    } else if let Some(ref crate_name) = def.crate_name {
        args.push(crate_name.clone());

        // Version (only for crates.io)
        if let Some(ref version) = def.version {
            args.push("--version".to_string());
            args.push(version.clone());
        }
    }

    // Binary name (if crate produces multiple binaries)
    if let Some(ref binary) = def.binary {
        args.push("--bin".to_string());
        args.push(binary.clone());
    }

    // Features
    if def.all_features {
        args.push("--all-features".to_string());
    } else if !def.features.is_empty() {
        args.push("--features".to_string());
        args.push(def.features.join(","));
    }

    // Locked builds
    if def.locked {
        args.push("--locked".to_string());
    }

    // Force reinstall
    args.push("--force".to_string());

    // Install root - cargo installs to <root>/bin/, so we use parent of install_dir
    // e.g., if install_dir is ~/.local/bin, we use ~/.local as root
    let cargo_root = install_root.parent().unwrap_or(install_root);
    args.push("--root".to_string());
    args.push(cargo_root.to_string_lossy().to_string());

    // Additional user-specified args
    for arg in &def.cargo_args {
        args.push(arg.clone());
    }

    if ctx.verbose > 0 {
        ui::dim(&format!("Running: cargo {}", args.join(" ")));
    }

    let output = Command::new("cargo")
        .args(&args)
        .output()
        .context("Failed to run cargo install")?;

    Ok(CargoInstallResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

// =============================================================================
// Helper functions - npm/pnpm
// =============================================================================

/// Detect available npm package manager (prefer pnpm > npm).
/// Returns (command, display_name).
fn detect_npm_package_manager() -> (String, &'static str) {
    // Try pnpm first (preferred)
    if Command::new("pnpm")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return ("pnpm".to_string(), "pnpm");
    }

    // Fall back to npm
    if Command::new("npm")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return ("npm".to_string(), "npm");
    }

    // Default to npm (will fail later with helpful error)
    ("npm".to_string(), "npm")
}

struct NpmInstallResult {
    success: bool,
    #[allow(dead_code)]
    stdout: String,
    stderr: String,
}

/// Run npm/pnpm global install.
fn run_npm_install(
    pm: &str,
    package: &str,
    version: Option<&str>,
    needs_scripts: bool,
    ctx: &Context,
) -> Result<NpmInstallResult> {
    // Build package spec with optional version
    let package_spec = if let Some(v) = version {
        format!("{package}@{v}")
    } else {
        package.to_string()
    };

    let mut args = vec!["install", "-g"];

    // pnpm uses different flags
    if pm == "pnpm" {
        // pnpm needs explicit permission for postinstall scripts
        if !needs_scripts {
            args.push("--ignore-scripts");
        }
    } else {
        // npm flags
        if !needs_scripts {
            args.push("--ignore-scripts");
        }
    }

    args.push(&package_spec);

    if ctx.verbose > 0 {
        ui::dim(&format!("Running: {} {}", pm, args.join(" ")));
    }

    let output = Command::new(pm)
        .args(&args)
        .output()
        .with_context(|| format!("Failed to run {pm} install"))?;

    Ok(NpmInstallResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Find where npm/pnpm installed a global binary.
fn find_npm_binary(pm: &str, binary_name: &str) -> Result<PathBuf> {
    // Get the global bin directory
    let bin_dir_output = Command::new(pm)
        .args(["config", "get", "prefix"])
        .output()
        .context("Failed to get npm prefix")?;

    if !bin_dir_output.status.success() {
        bail!("Failed to get {pm} prefix");
    }

    let prefix = String::from_utf8_lossy(&bin_dir_output.stdout)
        .trim()
        .to_string();

    // npm/pnpm install binaries to prefix/bin on Unix, prefix on Windows
    let bin_path = if cfg!(windows) {
        PathBuf::from(&prefix).join(binary_name)
    } else {
        PathBuf::from(&prefix).join("bin").join(binary_name)
    };

    // Also check pnpm's default location
    if !bin_path.exists() && pm == "pnpm" {
        // pnpm often uses ~/.local/share/pnpm
        if let Some(home) = dirs::home_dir() {
            let pnpm_path = home
                .join(".local")
                .join("share")
                .join("pnpm")
                .join(binary_name);
            if pnpm_path.exists() {
                return Ok(pnpm_path);
            }
        }
    }

    if bin_path.exists() {
        Ok(bin_path)
    } else {
        // Try which/where as fallback
        let which_cmd = if cfg!(windows) { "where" } else { "which" };
        let which_output = Command::new(which_cmd).arg(binary_name).output();

        if let Ok(output) = which_output
            && output.status.success()
        {
            let path = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }

        bail!(
            "Binary '{}' not found after {} install. Expected at: {}",
            binary_name,
            pm,
            bin_path.display()
        )
    }
}

/// Get latest version from npm registry.
fn get_npm_latest(package: &str) -> Result<String> {
    let url = format!("https://registry.npmjs.org/{package}/latest");

    let agent = ureq::Agent::new_with_defaults();
    let mut response = agent
        .get(&url)
        .header("User-Agent", "bossa-tools")
        .call()
        .context("Failed to fetch npm registry")?;

    let body: serde_json::Value = response
        .body_mut()
        .read_json()
        .context("Failed to parse npm response")?;

    body["version"]
        .as_str()
        .map(std::string::ToString::to_string)
        .context("No version in npm response")
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_install_dir_default() {
        let dir = get_install_dir(None).unwrap();
        assert!(dir.to_string_lossy().contains(".local/bin"));
    }

    #[test]
    fn test_get_install_dir_custom() {
        let dir = get_install_dir(Some("/tmp/test")).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/test"));
    }

    #[test]
    fn test_get_install_dir_tilde() {
        let dir = get_install_dir(Some("~/custom/bin")).unwrap();
        assert!(dir.to_string_lossy().contains("custom/bin"));
        assert!(!dir.to_string_lossy().contains('~'));
    }

    #[test]
    fn test_detect_package_manager_fedora() {
        assert_eq!(detect_package_manager("fedora:latest"), "dnf");
        assert_eq!(detect_package_manager("fedora:39"), "dnf");
    }

    #[test]
    fn test_detect_package_manager_alpine() {
        assert_eq!(detect_package_manager("alpine:latest"), "apk");
        assert_eq!(detect_package_manager("alpine:3.18"), "apk");
    }

    #[test]
    fn test_detect_package_manager_debian() {
        assert_eq!(detect_package_manager("debian:bookworm"), "apt");
        assert_eq!(detect_package_manager("ubuntu:22.04"), "apt");
    }

    #[test]
    fn test_detect_package_manager_ubi() {
        assert_eq!(
            detect_package_manager("registry.access.redhat.com/ubi9/ubi"),
            "dnf"
        );
        assert_eq!(
            detect_package_manager("registry.access.redhat.com/ubi9/ubi-minimal"),
            "microdnf"
        );
    }

    #[test]
    fn test_detect_package_manager_centos() {
        assert_eq!(detect_package_manager("centos:7"), "dnf");
        assert_eq!(
            detect_package_manager("quay.io/centos/centos:stream9"),
            "dnf"
        );
    }

    #[test]
    fn test_build_install_script_simple() {
        let script = build_install_script(Some("ripgrep"), "dnf", None, None, "/usr/bin/rg");
        assert!(script.contains("dnf install -y ripgrep"));
        assert!(script.contains("test -f /usr/bin/rg"));
    }

    #[test]
    fn test_build_install_script_with_deps() {
        let deps = vec!["dep1".to_string(), "dep2".to_string()];
        let script = build_install_script(Some("main"), "apt", Some(&deps), None, "/usr/bin/main");
        assert!(script.contains("apt-get install -y --no-install-recommends main dep1 dep2"));
    }

    #[test]
    fn test_build_install_script_with_pre_install() {
        let script = build_install_script(
            Some("ripgrep"),
            "dnf",
            None,
            Some("dnf config-manager --enable epel"),
            "/usr/bin/rg",
        );
        assert!(script.contains("dnf config-manager --enable epel"));
        assert!(script.contains("dnf install -y ripgrep"));
    }

    #[test]
    fn test_build_install_script_no_package() {
        let script = build_install_script(None, "dnf", None, None, "/usr/bin/existing");
        assert!(!script.contains("install"));
        assert!(script.contains("test -f /usr/bin/existing"));
    }

    #[test]
    fn test_build_install_script_microdnf() {
        let script = build_install_script(Some("jq"), "microdnf", None, None, "/usr/bin/jq");
        assert!(script.contains("microdnf install -y jq"));
        assert!(script.contains("microdnf clean all"));
    }

    #[test]
    fn test_build_install_script_apk() {
        let script = build_install_script(Some("curl"), "apk", None, None, "/usr/bin/curl");
        assert!(script.contains("apk add --no-cache curl"));
    }

    #[test]
    fn test_extract_targz_not_found() {
        // Create a minimal valid gzip stream with empty tar
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            // Add a dummy file
            let data = b"test content";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "other-file", &data[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let compressed = encoder.finish().unwrap();

        let result = extract_targz(&compressed, "nonexistent", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_extract_targz_found() {
        // Create a tar.gz with our target binary
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            let data = b"binary content";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "mydir/mytool", &data[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let compressed = encoder.finish().unwrap();

        let result = extract_targz(&compressed, "mytool", None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"binary content");
    }

    #[test]
    fn test_extract_targz_with_path() {
        // Create a tar.gz with nested structure
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            let data = b"nested binary";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "release-v1.0/bin/mytool", &data[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let compressed = encoder.finish().unwrap();

        let result = extract_targz(&compressed, "mytool", Some("release-v1.0/bin"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"nested binary");
    }

    #[test]
    fn test_extract_zip_found() {
        use std::io::Write;

        // Create a zip file with our target binary
        let mut buffer = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("mydir/mytool", options).unwrap();
            zip.write_all(b"zip binary content").unwrap();
            zip.finish().unwrap();
        }

        let result = extract_zip(&buffer, "mytool", None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"zip binary content");
    }

    #[test]
    fn test_extract_zip_not_found() {
        use std::io::Write;

        let mut buffer = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("other-file", options).unwrap();
            zip.write_all(b"test").unwrap();
            zip.finish().unwrap();
        }

        let result = extract_zip(&buffer, "nonexistent", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_extract_zip_with_path() {
        use std::io::Write;

        let mut buffer = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("release-v1.0/bin/mytool", options).unwrap();
            zip.write_all(b"nested zip binary").unwrap();
            zip.finish().unwrap();
        }

        let result = extract_zip(&buffer, "mytool", Some("release-v1.0/bin"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"nested zip binary");
    }

    #[test]
    fn test_extract_version_simple() {
        assert_eq!(extract_version("1.2.3"), Some("1.2.3".to_string()));
        assert_eq!(extract_version("v1.2.3"), Some("1.2.3".to_string()));
    }

    #[test]
    fn test_extract_version_with_name() {
        assert_eq!(extract_version("tool 1.2.3"), Some("1.2.3".to_string()));
        assert_eq!(
            extract_version("tool version 1.2.3"),
            Some("1.2.3".to_string())
        );
        assert_eq!(
            extract_version("ripgrep 14.1.0"),
            Some("14.1.0".to_string())
        );
    }

    #[test]
    fn test_extract_version_with_suffix() {
        assert_eq!(
            extract_version("1.2.3-beta"),
            Some("1.2.3-beta".to_string())
        );
        assert_eq!(
            extract_version("1.2.3-rc.1"),
            Some("1.2.3-rc.1".to_string())
        );
    }

    #[test]
    fn test_extract_version_two_part() {
        assert_eq!(extract_version("1.2"), Some("1.2".to_string()));
    }

    #[test]
    fn test_extract_version_no_version() {
        assert_eq!(extract_version("no version here"), None);
        assert_eq!(extract_version(""), None);
    }

    #[test]
    fn test_version_info_is_outdated() {
        let info = VersionInfo {
            name: "test".to_string(),
            source: "github".to_string(),
            current: Some("1.0.0".to_string()),
            latest: Some("1.1.0".to_string()),
            error: None,
        };
        assert!(info.is_outdated());

        let info = VersionInfo {
            name: "test".to_string(),
            source: "github".to_string(),
            current: Some("1.1.0".to_string()),
            latest: Some("1.1.0".to_string()),
            error: None,
        };
        assert!(!info.is_outdated());

        // Test with 'v' prefix
        let info = VersionInfo {
            name: "test".to_string(),
            source: "github".to_string(),
            current: Some("v1.1.0".to_string()),
            latest: Some("1.1.0".to_string()),
            error: None,
        };
        assert!(!info.is_outdated());
    }

    #[test]
    fn test_sort_by_dependencies_no_deps() {
        use crate::schema::ToolDefinition;

        let def_a = ToolDefinition::default();
        let def_b = ToolDefinition::default();

        let name_a = "tool_a".to_string();
        let name_b = "tool_b".to_string();

        let tools = vec![(&name_a, &def_a), (&name_b, &def_b)];

        let sorted = sort_by_dependencies(tools).unwrap();
        assert_eq!(sorted.len(), 2);
        // Without deps, order is determined by the algorithm (stable)
    }

    #[test]
    fn test_sort_by_dependencies_simple_chain() {
        use crate::schema::ToolDefinition;

        // bun depends on pnpm
        let def_bun = ToolDefinition {
            depends: vec!["pnpm".to_string()],
            ..Default::default()
        };

        let def_pnpm = ToolDefinition::default();

        let name_bun = "bun".to_string();
        let name_pnpm = "pnpm".to_string();

        // Input order: bun first, pnpm second
        let tools = vec![(&name_bun, &def_bun), (&name_pnpm, &def_pnpm)];

        let sorted = sort_by_dependencies(tools).unwrap();

        // pnpm should come before bun
        let names: Vec<_> = sorted.iter().map(|(n, _)| n.as_str()).collect();
        let pnpm_idx = names.iter().position(|n| *n == "pnpm").unwrap();
        let bun_idx = names.iter().position(|n| *n == "bun").unwrap();
        assert!(pnpm_idx < bun_idx, "pnpm should be installed before bun");
    }

    #[test]
    fn test_sort_by_dependencies_detects_cycle() {
        use crate::schema::ToolDefinition;

        // a depends on b, b depends on a = cycle
        let def_a = ToolDefinition {
            depends: vec!["b".to_string()],
            ..Default::default()
        };

        let def_b = ToolDefinition {
            depends: vec!["a".to_string()],
            ..Default::default()
        };

        let name_a = "a".to_string();
        let name_b = "b".to_string();

        let tools = vec![(&name_a, &def_a), (&name_b, &def_b)];

        let result = sort_by_dependencies(tools);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Circular dependency")
        );
    }

    #[test]
    fn test_detect_npm_package_manager() {
        // Just test that the function runs without panicking
        let (cmd, name) = detect_npm_package_manager();
        assert!(!cmd.is_empty());
        assert!(!name.is_empty());
    }
}
