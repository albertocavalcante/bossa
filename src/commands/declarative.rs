//! Canonical configuration projection and terminal presentation for verified setup.

use anyhow::{Context as _, Result, ensure};
use bossa::setup::{
    self, Checkout, Link, NamedSetup, Setup, Storage, StorageKind, Workspace, Worktree,
};
use declarative::{ExecutionPlan, ResourceState};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::{Context, ui};

struct Selected {
    groups: Vec<String>,
    specs: Vec<NamedSetup>,
}

fn absolute(path: PathBuf, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn selected(target: Option<&str>, required: bool) -> Result<Selected> {
    let config = crate::config::load_bossa_config(required)?;
    let cwd = std::env::current_dir()?;
    let mut groups: Vec<_> = config
        .collections
        .keys()
        .map(|name| ("collections", name.as_str()))
        .chain(
            config
                .workspaces
                .repos
                .iter()
                .map(|repo| ("workspaces", repo.name.as_str())),
        )
        .chain(config.storage.keys().map(|name| ("storage", name.as_str())))
        .collect();
    groups.sort_unstable();
    let (domain, name) = match target {
        None => (None, None),
        Some(domain @ ("collections" | "workspaces" | "storage")) => (Some(domain), None),
        Some(target) => {
            if let Some((domain @ ("collections" | "workspaces" | "storage"), name)) =
                target.split_once('.')
            {
                ensure!(
                    !name.is_empty() && groups.contains(&(domain, name)),
                    "Unknown setup selector: {target}"
                );
                (Some(domain), Some(name))
            } else {
                let found: Vec<_> = groups.iter().filter(|(_, name)| *name == target).collect();
                ensure!(
                    found.len() == 1,
                    "Unknown or ambiguous setup selector: {target}; use collections.NAME, workspaces.NAME or storage.NAME"
                );
                (Some(found[0].0), Some(found[0].1))
            }
        }
    };
    let matches = |kind, id: &str| domain.is_none_or(|d| d == kind) && name.is_none_or(|n| n == id);
    let names = groups
        .iter()
        .filter(|(kind, id)| matches(*kind, id))
        .map(|(kind, id)| format!("{kind}.{id}"))
        .collect();
    let mut specs = Vec::new();
    let mut collections: Vec<_> = config.collections.iter().collect();
    collections.sort_by_key(|(name, _)| *name);
    for (name, collection) in collections {
        if !matches("collections", name) {
            continue;
        }
        collection.clone.validate()?;
        let root = absolute(crate::paths::expand(&collection.path), &cwd);
        for repo in &collection.repos {
            specs.push(NamedSetup {
                name: format!("{name}/{}", repo.name),
                setup: Setup::Checkout(Checkout {
                    url: repo.url.clone(),
                    path: root.join(&repo.name),
                    branch: repo.default_branch.clone(),
                    depth: collection.clone.depth,
                    single_branch: collection.clone.single_branch,
                    options: collection.clone.options.clone(),
                }),
            });
        }
    }
    if config
        .workspaces
        .repos
        .iter()
        .any(|workspace| matches("workspaces", &workspace.name))
    {
        let root = absolute(
            if crate::paths::environment()
                .variable("BOSSA_WORKSPACES_DIR")
                .is_some()
            {
                crate::paths::workspaces_dir()?
            } else {
                config
                    .workspaces
                    .expanded_root(crate::paths::environment())?
            },
            &cwd,
        );
        let physical_root = setup::resolve_location(&root)?;
        for workspace in &config.workspaces.repos {
            if !matches("workspaces", &workspace.name) {
                continue;
            }
            let bare = workspace.bare_path(&root);
            let worktrees: Vec<_> = workspace
                .worktrees
                .iter()
                .map(|branch| Worktree {
                    branch: branch.clone(),
                    path: workspace.worktree_path(&root, branch),
                })
                .collect();
            for path in std::iter::once(&bare).chain(worktrees.iter().map(|w| &w.path)) {
                ensure!(
                    setup::resolve_location(path)?.starts_with(&physical_root),
                    "Workspace destination resolves outside the configured root: {}",
                    path.display()
                );
            }
            specs.push(NamedSetup {
                name: workspace.name.clone(),
                setup: Setup::Workspace(Workspace {
                    url: workspace.url.clone(),
                    bare,
                    worktrees,
                }),
            });
        }
    }
    let mut storages: Vec<_> = config.storage.iter().collect();
    storages.sort_by_key(|(name, _)| *name);
    for (name, storage) in storages {
        if !matches("storage", name) {
            continue;
        }
        let mount = absolute(storage.expanded_mount(crate::paths::environment())?, &cwd);
        let mut links = Vec::new();
        for link in &storage.symlinks {
            let source = if link.to.contains("{mount}") {
                let text = mount
                    .to_str()
                    .context("A storage placeholder requires a UTF-8 mount path")?;
                crate::paths::expand(&link.to.replace("{mount}", text))
            } else {
                crate::paths::expand(&link.to)
            };
            links.push(Link {
                source: absolute(source, &cwd),
                target: absolute(crate::paths::expand(&link.from), &cwd),
            });
        }
        specs.push(NamedSetup {
            name: name.clone(),
            setup: Setup::Storage(Storage {
                mount,
                links,
                kind: match storage.storage_type {
                    crate::schema::StorageType::Internal => StorageKind::Internal,
                    crate::schema::StorageType::External => StorageKind::External,
                    crate::schema::StorageType::Network => StorageKind::Network,
                },
            }),
        });
    }
    Ok(Selected {
        groups: names,
        specs,
    })
}

fn plan(selected: Selected) -> Result<ExecutionPlan> {
    if selected
        .specs
        .iter()
        .any(|spec| !matches!(spec.setup, Setup::Storage(_)))
    {
        crate::runner::require_command_at("git", &std::env::current_dir()?)?;
    }
    let runner = setup::git_runner(crate::runner::native_runner().clone());
    let observer = runner.clone().with_limits(bossa::ProcessLimits {
        timeout: Duration::from_secs(15),
        max_output_bytes: 8 * 1024 * 1024,
    });
    setup::plan(
        selected.specs,
        Arc::new(observer),
        Arc::new(runner),
        std::env::current_dir()?,
    )
}

pub fn status(ctx: &Context, target: Option<&str>) -> Result<()> {
    ui::header("Bossa Status");
    let selected = selected(target, false)?;
    if selected.groups.is_empty() {
        ui::dim("No resources configured");
    }
    for group in &selected.groups {
        println!("  {group}");
    }
    let plan = plan(selected)?;
    let mut unavailable = false;
    for resource in &plan.unprivileged {
        match resource.current_state() {
            Ok(ResourceState::Present { .. }) => {
                println!("  ✓ {}: selected postconditions verified", resource.id());
            }
            Ok(state) => {
                if !ctx.quiet {
                    println!("  → {}: {state:?}", resource.id());
                }
            }
            Err(error) => {
                eprintln!("  Unavailable {}: {error:#}", resource.id());
                unavailable = true;
            }
        }
    }
    ensure!(!unavailable, "Selected setup observation unavailable");
    Ok(())
}

pub fn diff(ctx: &Context, target: Option<&str>) -> Result<()> {
    run_apply(ctx, target, true, 1, false, false)
}

pub fn apply(
    ctx: &Context,
    target: Option<&str>,
    dry_run: bool,
    jobs: usize,
    yes: bool,
) -> Result<()> {
    run_apply(ctx, target, dry_run, jobs, yes, true)
}

fn run_apply(
    ctx: &Context,
    target: Option<&str>,
    dry_run: bool,
    jobs: usize,
    yes: bool,
    required: bool,
) -> Result<()> {
    ui::header(if dry_run {
        "Configuration Preview"
    } else {
        "Applying Configuration"
    });
    let selected = selected(target, required)?;
    if selected.groups.is_empty() {
        ui::dim("No resources configured");
    }
    for group in &selected.groups {
        if !ctx.quiet {
            println!("  {group}");
        }
    }
    let summary = crate::engine::executor::execute(
        plan(selected)?,
        bossa::ResourceApplyOptions {
            dry_run,
            jobs,
            yes,
            verbose: ctx.verbose > 0,
        },
        ctx.quiet,
    )?;
    ensure!(
        summary.is_success(),
        "Selected setup incomplete; possible changes are retained, no automatic rollback attempted"
    );
    Ok(())
}
