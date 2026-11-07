//! GitHub CLI extension resource

use anyhow::{Context, Result, bail};

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
        validate_name(&self.name)?;
        installed_from_inventory(
            &crate::runner::observe("gh", ["extension", "list"])?,
            &self.name,
        )
    }

    /// Install the extension
    fn install(&self, _ctx: &ApplyContext) -> Result<()> {
        let output = crate::runner::capture("gh", ["extension", "install", &self.name])
            .context("Failed to run gh extension install")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("gh extension install failed: {}", stderr.trim());
        }

        Ok(())
    }
}

fn validate_name(name: &str) -> Result<()> {
    super::native::package_name(name, "GitHub extension")?;
    let pieces: Vec<_> = name.split('/').collect();
    if pieces.len() != 2
        || pieces.iter().any(|piece| {
            piece.is_empty()
                || !piece.starts_with(|value: char| value.is_ascii_alphanumeric())
                || piece
                    .chars()
                    .any(|value| !value.is_ascii_alphanumeric() && !"._-".contains(value))
        })
    {
        bail!(
            "GitHub extensions require owner/repository names; ref and URL specifications are not supported"
        );
    }
    Ok(())
}

fn installed_from_inventory(bytes: &[u8], wanted: &str) -> Result<bool> {
    let text = std::str::from_utf8(bytes).context("Invalid GitHub extension inventory encoding")?;
    let mut found = false;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let repository = line
            .split_whitespace()
            .find_map(|field| {
                let field = field
                    .strip_prefix("https://github.com/")
                    .unwrap_or(field)
                    .trim_end_matches(".git");
                validate_name(field).is_ok().then_some(field)
            })
            .context("Unsupported GitHub extension inventory format")?;
        found |= repository.eq_ignore_ascii_case(wanted);
    }
    Ok(found)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_repository_inventory_and_supported_specs() {
        let inventory = b"gh tool\towner/gh-tool\tv1\n";
        assert!(installed_from_inventory(inventory, "owner/gh-tool").unwrap());
        assert!(!installed_from_inventory(inventory, "owner/gh-too").unwrap());
        assert!(installed_from_inventory(b"unknown", "owner/gh-tool").is_err());
        for name in [
            "../tool",
            "owner/tool@v1",
            "https://github.com/owner/tool",
            "--flag/tool",
        ] {
            assert!(validate_name(name).is_err());
        }
    }
}
