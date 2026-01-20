//! pnpm global package resource

use anyhow::{Context, Result, bail};
use std::process::Command;

use super::{ApplyContext, ApplyResult, Resource, ResourceState};

/// A pnpm global package
#[derive(Debug, Clone)]
pub struct PnpmPackage {
    pub name: String,
}

impl PnpmPackage {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }

    /// Check if package is installed globally
    fn is_installed(&self) -> Result<bool> {
        let output = Command::new("pnpm")
            .args(["list", "-g", "--depth=0"])
            .output()
            .context("Failed to run pnpm list")?;

        if !output.status.success() {
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.contains(&self.name))
    }

    /// Install the package globally
    fn install(&self, _ctx: &ApplyContext) -> Result<()> {
        let output = Command::new("pnpm")
            .args(["add", "-g", &self.name])
            .output()
            .context("Failed to run pnpm add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("pnpm add failed: {}", stderr.trim());
        }

        Ok(())
    }
}

impl Resource for PnpmPackage {
    fn id(&self) -> String {
        format!("pnpm:{}", self.name)
    }

    fn description(&self) -> String {
        format!("Install pnpm global package {}", self.name)
    }

    fn resource_type(&self) -> &'static str {
        "pnpm_package"
    }

    fn current_state(&self) -> Result<ResourceState> {
        if self.is_installed()? {
            Ok(ResourceState::Present { details: None })
        } else {
            Ok(ResourceState::Absent)
        }
    }

    fn desired_state(&self) -> ResourceState {
        ResourceState::Present { details: None }
    }

    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped {
                reason: "Dry run".to_string(),
            });
        }

        if self.is_installed()? {
            return Ok(ApplyResult::NoChange);
        }

        self.install(ctx)?;
        Ok(ApplyResult::Created)
    }
}
