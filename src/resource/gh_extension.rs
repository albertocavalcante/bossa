//! GitHub CLI extension resource

use anyhow::{Context, Result, bail};
use std::process::Command;

use super::{ApplyContext, ApplyResult, Resource, ResourceState};

/// A GitHub CLI extension
#[derive(Debug, Clone)]
pub struct GHExtension {
    pub name: String,
}

impl GHExtension {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }

    /// Check if extension is installed
    fn is_installed(&self) -> Result<bool> {
        let output = Command::new("gh")
            .args(["extension", "list"])
            .output()
            .context("Failed to run gh extension list")?;

        if !output.status.success() {
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.contains(&self.name))
    }

    /// Install the extension
    fn install(&self, _ctx: &ApplyContext) -> Result<()> {
        let output = Command::new("gh")
            .args(["extension", "install", &self.name])
            .output()
            .context("Failed to run gh extension install")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("gh extension install failed: {}", stderr.trim());
        }

        Ok(())
    }
}

impl Resource for GHExtension {
    fn id(&self) -> String {
        format!("gh:{}", self.name)
    }

    fn description(&self) -> String {
        format!("Install GitHub CLI extension {}", self.name)
    }

    fn resource_type(&self) -> &'static str {
        "gh_extension"
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
