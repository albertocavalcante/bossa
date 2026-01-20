use anyhow::Result;

use crate::cli::T9Command;
use crate::runner;
use crate::ui;
use crate::Context;

pub fn run(_ctx: &Context, cmd: T9Command) -> Result<()> {
    match cmd {
        T9Command::Status => status(),
        T9Command::Config => config(),
        T9Command::Clean => clean(),
        T9Command::Stats => stats(),
        T9Command::Verify => verify(),
    }
}

fn status() -> Result<()> {
    ui::header("T9 Repository Status");
    runner::run_script("t9-repos", &["status"])?;
    Ok(())
}

fn config() -> Result<()> {
    ui::header("Configuring T9 Repos for exFAT");
    runner::run_script("t9-repos", &["config"])?;
    Ok(())
}

fn clean() -> Result<()> {
    ui::header("Cleaning T9 Metadata");
    ui::info("Removing ._ and .DS_Store files...");
    runner::run_script("t9-repos", &["clean-metadata"])?;
    ui::success("Metadata cleaned");
    Ok(())
}

fn stats() -> Result<()> {
    ui::header("T9 Statistics");
    runner::run_script("t9-repos", &["stats"])?;
    Ok(())
}

fn verify() -> Result<()> {
    ui::header("Verifying T9 Setup");
    runner::run_script("t9-repos", &["verify"])?;
    Ok(())
}
