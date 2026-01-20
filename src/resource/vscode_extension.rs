//! VS Code extension resource

use anyhow::{Context, Result, bail};
use std::process::Command;

use super::{ApplyContext, ApplyResult, Resource, ResourceState};

/// A VS Code extension
#[derive(Debug, Clone)]
pub struct VSCodeExtension {
    pub extension_id: String,
}

impl VSCodeExtension {
    pub fn new(extension_id: &str) -> Self {
        Self {
            extension_id: extension_id.to_string(),
        }
    }

    /// Check if extension is installed
    fn is_installed(&self) -> Result<bool> {
        let output = Command::new("code")
            .args(["--list-extensions"])
            .output()
            .context("Failed to run code --list-extensions")?;

        if !output.status.success() {
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().any(|line| line.trim() == self.extension_id))
    }

    /// Install the extension
    fn install(&self, _ctx: &ApplyContext) -> Result<()> {
        let output = Command::new("code")
            .args(["--install-extension", &self.extension_id, "--force"])
            .output()
            .context("Failed to run code --install-extension")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("code --install-extension failed: {}", stderr.trim());
        }

        Ok(())
    }
}

impl Resource for VSCodeExtension {
    fn id(&self) -> String {
        format!("vscode:{}", self.extension_id)
    }

    fn description(&self) -> String {
        format!("Install VS Code extension {}", self.extension_id)
    }

    fn resource_type(&self) -> &'static str {
        "vscode_extension"
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
