//! Service resource - restart macOS services

use anyhow::{Context, Result};
use std::process::Command;

use super::{ApplyContext, ApplyResult, Resource, ResourceState, SudoRequirement};

/// A macOS service to restart
#[derive(Debug, Clone)]
pub struct Service {
    pub name: String,
}

impl Service {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }

    /// Check if service is running
    fn is_running(&self) -> bool {
        Command::new("pgrep")
            .args(["-x", &self.name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Restart the service via killall
    fn restart(&self) -> Result<bool> {
        let status = Command::new("killall")
            .arg(&self.name)
            .status()
            .context("Failed to execute killall")?;

        Ok(status.success())
    }
}

impl Resource for Service {
    fn id(&self) -> String {
        self.name.clone()
    }

    fn description(&self) -> String {
        format!("Restart {}", self.name)
    }

    fn resource_type(&self) -> &'static str {
        "service"
    }

    fn sudo_requirement(&self) -> SudoRequirement {
        SudoRequirement::None
    }

    fn current_state(&self) -> Result<ResourceState> {
        // Services are always in a "running" or "not running" state
        // We treat "needs restart" as the desired outcome
        Ok(ResourceState::Present { details: None })
    }

    fn desired_state(&self) -> ResourceState {
        // After apply, service should have been restarted
        ResourceState::Present {
            details: Some("restarted".to_string()),
        }
    }

    fn needs_apply(&self) -> Result<bool> {
        // Services marked for restart always need to be restarted
        Ok(true)
    }

    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped {
                reason: "Dry run".to_string(),
            });
        }

        if self.restart()? {
            Ok(ApplyResult::Modified)
        } else {
            Ok(ApplyResult::Skipped {
                reason: format!("{} was not running", self.name),
            })
        }
    }

    fn can_parallelize(&self) -> bool {
        false // Services should be restarted sequentially
    }
}
