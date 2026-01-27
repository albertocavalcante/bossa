//! Tool installation and management commands.
//!
//! This module provides commands for installing development tools from various sources:
//! - HTTP URLs pointing to tar.gz archives
//! - Container images (via podman/docker)
//! - GitHub releases
//!
//! Tools can be installed imperatively via CLI or declaratively via config.toml.

use crate::cli::ToolsCommand;
use crate::schema::{
    BossaConfig, ContainerMeta, InstalledTool, ToolDefinition, ToolSource, ToolsConfig,
};
use crate::ui;
use crate::Context;
use anyhow::{bail, Context as _, Result};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

/// Maximum download size (500 MB).
const MAX_DOWNLOAD_SIZE: u64 = 500 * 1024 * 1024;

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
        let filter_set: HashSet<_> = filter_tools.iter().map(|s| s.as_str()).collect();
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

    if !ctx.quiet {
        ui::header("Applying Tools");
        println!();
    }

    let mut installed = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for (name, def) in tools_to_apply {
        // Check platform availability
        if !def.is_available_for_current_platform() {
            if !ctx.quiet {
                ui::dim(&format!("  ⊘ {} (not available for this platform)", name));
            }
            skipped += 1;
            continue;
        }

        // Check if already installed
        let is_installed = state.get(name).is_some_and(|t| {
            PathBuf::from(&t.install_path).exists()
        });

        if is_installed && !force {
            if !ctx.quiet {
                ui::dim(&format!("  ✓ {} (already installed)", name));
            }
            skipped += 1;
            continue;
        }

        if dry_run {
            ui::info(&format!("  Would install: {} ({})", name, def.description));
            continue;
        }

        if !ctx.quiet {
            ui::info(&format!("  Installing {}...", name));
        }

        match install_from_definition(ctx, name, def, &config.tools) {
            Ok(installed_tool) => {
                state.insert(name.clone(), installed_tool);
                state.save()?;

                if !ctx.quiet {
                    ui::success(&format!("  ✓ {} installed", name));
                    if let Some(ref msg) = def.post_install {
                        println!();
                        ui::dim(msg);
                        println!();
                    }
                }
                installed += 1;
            }
            Err(e) => {
                ui::error(&format!("  ✗ {} failed: {}", name, e));
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
        bail!("{} tool(s) failed to install", failed);
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
            let image = def.image.as_ref().context("Image required for container source")?;
            let container_binary_path = def
                .binary_path
                .as_ref()
                .context("binary_path required for container source")?;

            let runtime = def
                .runtime
                .as_deref()
                .unwrap_or(&tools_section.runtime);

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
                all_packages.first().map(|s| s.as_str()),
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
                bail!("Container command failed:\n{}", run_result.stderr.trim_end());
            }

            let local_binary_path = install_path.join(&binary_name);
            copy_from_container(runtime, &container_name, container_binary_path, &local_binary_path)?;

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
            let repo = def.repo.as_ref().context("repo required for github-release source")?;
            let version = def
                .version
                .as_ref()
                .context("version required for github-release source")?;

            // Build download URL from repo and asset pattern
            let asset_pattern = def.asset.as_ref().map(|a| def.expand_template(a, name));

            let url = if let Some(asset) = asset_pattern {
                format!(
                    "https://github.com/{}/releases/download/{}/{}",
                    repo, version, asset
                )
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
                format!("https://crates.io/crates/{}", crate_name)
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
            ui::warn(&format!("Reinstalling '{}' (--force)", name));
        }
    }

    // Determine installation directory
    let install_path = get_install_dir(install_dir)?;
    fs::create_dir_all(&install_path)?;

    if !ctx.quiet {
        ui::info(&format!("Installing {} from {}", name, url));
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
        ui::info(&format!("Extracting binary '{}'...", binary_name));
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
        ui::success(&format!("Tool '{}' installed successfully!", name));
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
        bail!("Runtime must be 'podman' or 'docker', got '{}'", runtime);
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
            ui::warn(&format!("Reinstalling '{}' (--force)", name));
        }
    }

    // Determine installation directory
    let local_install_dir = get_install_dir(install_dir)?;
    fs::create_dir_all(&local_install_dir)?;

    // Extract binary name from path
    let binary_name = PathBuf::from(binary_path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_string());

    if !ctx.quiet {
        ui::header(&format!("Installing {} from container", name));
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
        ui::info(&format!("Creating container from {}...", image));
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
        ui::info(&format!("Container '{}' kept for debugging", container_name));
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
            package: package.map(|s| s.to_string()),
            binary_path: binary_path.to_string(),
            package_manager: Some(pkg_manager),
            runtime: runtime.to_string(),
        }),
    };
    config.insert(name.to_string(), installed_tool);
    config.save()?;

    if !ctx.quiet {
        ui::success(&format!("Tool '{}' installed successfully!", name));
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
        ui::info("Use 'bossa tools install', 'bossa tools install-container', or 'bossa tools apply' to install tools.");
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

        println!("  {} {}{}", status, name, in_config);
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
                        println!("  {} {}", status, name);
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
        bail!("Tool '{}' not found (not installed and not defined in config)", name);
    }

    ui::header(&format!("Tool: {}", name));
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

        if exists
            && let Ok(metadata) = fs::metadata(&binary_path)
        {
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
        }
    }

    Ok(())
}

/// Uninstall a tool.
fn uninstall(ctx: &Context, name: &str) -> Result<()> {
    let mut config = ToolsConfig::load()?;

    let tool = config
        .remove(name)
        .context(format!("Tool '{}' not found", name))?;

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
        ui::success(&format!("Tool '{}' uninstalled successfully!", name));
    }

    Ok(())
}

// =============================================================================
// Helper functions - General
// =============================================================================

/// Get the installation directory.
fn get_install_dir(custom_dir: Option<&str>) -> Result<PathBuf> {
    if let Some(dir) = custom_dir {
        let expanded = shellexpand::tilde(dir);
        return Ok(PathBuf::from(expanded.as_ref()));
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
    let target_with_path = archive_path
        .map(|p| format!("{}/{}", p.trim_matches('/'), binary_name))
        .unwrap_or_else(|| binary_name.to_string());

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let path_str = path.to_string_lossy();

        // Check if this entry matches our target
        let is_match = path_str.ends_with(&format!("/{}", binary_name))
            || path_str == binary_name
            || path_str.ends_with(&format!("/{}", target_with_path))
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
        "Binary '{}' not found in archive. Use --path/archive_path to specify the directory inside the archive.",
        binary_name
    )
}

/// Extract a specific binary from a zip archive.
fn extract_zip(data: &[u8], binary_name: &str, archive_path: Option<&str>) -> Result<Vec<u8>> {
    use std::io::Cursor;

    let reader = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)?;

    // Build target paths to search for
    let target_with_path = archive_path
        .map(|p| format!("{}/{}", p.trim_matches('/'), binary_name))
        .unwrap_or_else(|| binary_name.to_string());

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let path_str = file.name();

        // Check if this entry matches our target
        let is_match = path_str.ends_with(&format!("/{}", binary_name))
            || path_str == binary_name
            || path_str.ends_with(&format!("/{}", target_with_path))
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
        "Binary '{}' not found in zip archive. Use archive_path to specify the directory inside the archive.",
        binary_name
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
            "'{}' is installed but returned an error. Check your {} installation.",
            runtime,
            runtime
        ),
        Err(_) => bail!(
            "'{}' not found. Please install {} first.\n\
             On macOS: brew install {}\n\
             On Fedora: sudo dnf install {}",
            runtime,
            runtime,
            runtime,
            runtime
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
        script.push_str(&format!("{}\n", pre));
    }

    // Install packages if specified
    if let Some(pkg) = package {
        let all_packages: Vec<&str> = std::iter::once(pkg)
            .chain(
                dependencies
                    .map(|d| d.iter().map(|s| s.as_str()))
                    .into_iter()
                    .flatten(),
            )
            .collect();

        let packages_str = all_packages.join(" ");

        let install_cmd = match pkg_manager {
            "apk" => format!("apk add --no-cache {}", packages_str),
            "apt" => format!(
                "apt-get update && apt-get install -y --no-install-recommends {}",
                packages_str
            ),
            "microdnf" => format!("microdnf install -y {} && microdnf clean all", packages_str),
            "dnf" => format!("dnf install -y {} && dnf clean all", packages_str),
            "yum" => format!("yum install -y {} && yum clean all", packages_str),
            other => format!("{} install -y {}", other, packages_str),
        };

        script.push_str(&format!("{}\n", install_cmd));
    }

    // Verify binary exists
    script.push_str(&format!(
        "test -f {} || (echo 'Binary not found: {}' && exit 1)\n",
        binary_path, binary_path
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
        ui::dim(&format!("Running script in container:\n{}", script));
    }

    let output = Command::new(runtime)
        .args(["run", "--name", container_name, image, "sh", "-c", script])
        .output()
        .context(format!("Failed to run {} container", runtime))?;

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
    local_path: &PathBuf,
) -> Result<()> {
    let source = format!("{}:{}", container_name, container_path);

    let output = Command::new(runtime)
        .args(["cp", &source, &local_path.to_string_lossy()])
        .output()
        .context(format!(
            "Failed to copy file from container using {}",
            runtime
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
        .context(format!("Failed to remove container using {}", runtime))?;

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
    let cargo_root = install_root
        .parent()
        .unwrap_or(install_root);
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
        let script =
            build_install_script(Some("main"), "apt", Some(&deps), None, "/usr/bin/main");
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
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
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
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
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
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
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
}
