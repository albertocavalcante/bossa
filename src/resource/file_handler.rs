//! File handler association resource (duti)

use anyhow::{Context, Result, bail};

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
        super::native::package_name(&self.bundle_id, "handler bundle identifier")?;
        super::native::package_name(&self.uti, "uniform type identifier")?;
        if !valid_identifier(&self.uti) || !valid_identifier(&self.bundle_id) {
            bail!("File handlers require a UTI, not an extension or URL scheme");
        }
        let output = crate::runner::observe("duti", ["-d", &self.uti])?;
        let handler = std::str::from_utf8(&output)
            .context("Invalid handler observation encoding")?
            .trim();
        if !valid_identifier(handler) {
            bail!("Unavailable default UTI handler observation");
        }
        Ok(handler.eq_ignore_ascii_case(&self.bundle_id))
    }

    /// Set the handler
    fn set_handler(&self, _ctx: &ApplyContext) -> Result<()> {
        let output = crate::runner::capture("duti", ["-s", &self.bundle_id, &self.uti, "all"])
            .context("Failed to run duti")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("duti set failed: {}", stderr.trim());
        }

        Ok(())
    }
}

fn valid_identifier(value: &str) -> bool {
    value.contains('.')
        && value.split('.').all(|part| {
            !part.is_empty()
                && !part.starts_with('-')
                && part
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric() || "_-".contains(value))
        })
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
