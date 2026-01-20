//! File handler association resource (duti)

use anyhow::{Context, Result, bail};
use std::process::Command;

use super::{ApplyContext, ApplyResult, Resource, ResourceState};

/// A file handler association
#[derive(Debug, Clone)]
pub struct FileHandler {
    pub bundle_id: String,
    pub uti: String,
}

impl FileHandler {
    pub fn new(bundle_id: &str, uti: &str) -> Self {
        Self {
            bundle_id: bundle_id.to_string(),
            uti: uti.to_string(),
        }
    }

    /// Check if handler is set
    fn is_set(&self) -> Result<bool> {
        let output = Command::new("duti")
            .args(["-x", &self.uti])
            .output()
            .context("Failed to run duti")?;

        if !output.status.success() {
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.contains(&self.bundle_id))
    }

    /// Set the handler
    fn set_handler(&self, _ctx: &ApplyContext) -> Result<()> {
        let output = Command::new("duti")
            .args(["-s", &self.bundle_id, &self.uti, "all"])
            .output()
            .context("Failed to run duti")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("duti set failed: {}", stderr.trim());
        }

        Ok(())
    }
}

impl Resource for FileHandler {
    fn id(&self) -> String {
        format!("handler:{}:{}", self.bundle_id, self.uti)
    }

    fn description(&self) -> String {
        format!("Set {} as handler for {}", self.bundle_id, self.uti)
    }

    fn resource_type(&self) -> &'static str {
        "file_handler"
    }

    fn current_state(&self) -> Result<ResourceState> {
        if self.is_set()? {
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

        if self.is_set()? {
            return Ok(ApplyResult::NoChange);
        }

        self.set_handler(ctx)?;
        Ok(ApplyResult::Created)
    }
}
