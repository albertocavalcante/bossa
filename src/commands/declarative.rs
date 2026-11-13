//! Canonical configuration projection and terminal presentation for verified setup.

use anyhow::{Result, ensure};
use bossa::setup::{self, Setup};
use declarative::{ExecutionPlan, ResourceState};
use std::sync::Arc;
use std::time::Duration;

use crate::{Context, ui};

fn selected(target: Option<&str>, required: bool) -> Result<setup::SelectedSetup> {
    let config = crate::config::load_bossa_config(required)?;
    setup::select_configured(
        &config,
        target,
        crate::paths::environment(),
        &std::env::current_dir()?,
    )
}

fn plan(selected: setup::SelectedSetup) -> Result<ExecutionPlan> {
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
    for observation in setup::observe(&plan) {
        match observation.state {
            Ok(ResourceState::Present { .. }) => {
                println!("  ✓ {}: selected postconditions verified", observation.id);
            }
            Ok(state) => {
                if !ctx.quiet {
                    println!("  → {}: {state:?}", observation.id);
                }
            }
            Err(error) => {
                eprintln!("  Unavailable {}: {error:#}", observation.id);
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
