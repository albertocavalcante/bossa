//! Preview an explicitly supplied fixture config; pass --apply to consent to effects.
use anyhow::{Context, Result, bail};
use bossa_sdk::{
    AppEnvironment, ConfigFormat, ProcessLimits, ResourceApplyOptions, SystemRunner,
    apply_resources, decode_bossa_config, setup,
};
use declarative::{AutoDecline, CommandOutput, NoProgress, SudoProvider};
use std::{path::PathBuf, sync::Arc, time::Duration};

struct NoSudo;
impl SudoProvider for NoSudo {
    fn run(&self, _: &str, _: &[&str]) -> Result<CommandOutput> {
        bail!("This example does not support privileged resources")
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let file = PathBuf::from(
        args.next()
            .context("usage: configured_setup CONFIG.toml WORKDIR [--apply]")?,
    );
    let cwd = PathBuf::from(args.next().context("missing WORKDIR")?);
    let apply = match args.next() {
        None => false,
        Some(value) if value == "--apply" && args.next().is_none() => true,
        _ => bail!("unknown arguments"),
    };
    let environment = AppEnvironment::capture();
    let config = decode_bossa_config(ConfigFormat::Toml, &std::fs::read(file)?)?.desired;
    let selected = setup::select_configured(&config, None, &environment, &cwd)?;
    let runner = setup::git_runner(SystemRunner::new(environment.variables().clone()));
    let observer = runner.clone().with_limits(ProcessLimits {
        timeout: Duration::from_secs(15),
        max_output_bytes: 8 * 1024 * 1024,
    });
    let plan = setup::plan(
        selected.specs,
        Arc::new(observer),
        Arc::new(runner.clone()),
        cwd.clone(),
    )?;
    for observation in setup::observe(&plan) {
        println!("{}: {:?}", observation.id, observation.state?);
    }
    let report = apply_resources(
        plan,
        ResourceApplyOptions {
            dry_run: !apply,
            yes: apply,
            ..Default::default()
        },
        &runner,
        &cwd,
        || Ok(NoSudo),
        &mut NoProgress,
        &mut AutoDecline,
    )?;
    if !report.is_success() {
        bail!("Selected setup incomplete");
    }
    Ok(())
}
