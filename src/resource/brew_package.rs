//! Homebrew package resource

use anyhow::{Context, Result, bail};

use super::{ApplyContext, ApplyResult, Resource, ResourceState, SudoRequirement};

/// Type of brew package
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrewPackageType {
    Formula,
    Cask,
    Tap,
}

/// A Homebrew package
#[derive(Debug, Clone)]
pub struct BrewPackage {
    pub name: String,
    pub package_type: BrewPackageType,
    pub requires_sudo: bool,
}

impl BrewPackage {
    pub fn formula(name: &str) -> Self {
        Self {
            name: name.to_string(),
            package_type: BrewPackageType::Formula,
            requires_sudo: false,
        }
    }

    pub fn cask(name: &str) -> Self {
        Self {
            name: name.to_string(),
            package_type: BrewPackageType::Cask,
            requires_sudo: false, // Determined by config allowlist
        }
    }

    pub fn tap(name: &str) -> Self {
        Self {
            name: name.to_string(),
            package_type: BrewPackageType::Tap,
            requires_sudo: false,
        }
    }

    pub fn with_sudo(mut self, requires: bool) -> Self {
        self.requires_sudo = requires;
        self
    }

    /// Check if package is installed
    fn is_installed(&self) -> Result<bool> {
        if self.requires_sudo {
            bail!(
                "Privileged Homebrew installs are blocked until a scoped Homebrew-owner executor exists; brew must not run as root"
            );
        }
        super::native::package_name(&self.name, "Homebrew package")?;
        let pieces: Vec<_> = self.name.split('/').collect();
        if self.name != self.name.to_ascii_lowercase()
            || (pieces.len() >= 2
                && (pieces[1].starts_with("homebrew-") || pieces[1].starts_with("linuxbrew-")))
        {
            bail!(
                "Use canonical lowercase Homebrew tokens and owner/tap names without repository prefixes"
            );
        }
        let valid_count = if self.package_type == BrewPackageType::Tap {
            pieces.len() == 2
        } else {
            matches!(pieces.len(), 1 | 3)
        };
        if !valid_count
            || pieces.iter().any(|piece| {
                !piece.starts_with(|value: char| value.is_ascii_alphanumeric())
                    || piece
                        .chars()
                        .any(|value| !value.is_ascii_alphanumeric() && !"@+._-".contains(value))
            })
        {
            bail!("Unsupported Homebrew package specification");
        }
        super::brew_inventory::BrewPaths::native()?.installed(self.package_type, &self.name)
    }

    /// Install the package
    fn install(&self, _ctx: &ApplyContext) -> Result<()> {
        if self.requires_sudo {
            bail!("Privileged Homebrew installs are unsupported; no command was executed");
        }
        let args: Vec<&str> = match self.package_type {
            BrewPackageType::Tap => vec!["tap", &self.name],
            BrewPackageType::Formula => vec!["install", "--formula", &self.name],
            BrewPackageType::Cask => vec!["install", "--cask", &self.name],
        };

        let output = crate::runner::capture("brew", &args).context("Failed to run brew install")?;
        if !output.status.success() {
            bail!(
                "brew install failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        Ok(())
    }
}

impl Resource for BrewPackage {
    fn id(&self) -> String {
        self.name.clone()
    }

    fn description(&self) -> String {
        let type_str = match self.package_type {
            BrewPackageType::Formula => "formula",
            BrewPackageType::Cask => "cask",
            BrewPackageType::Tap => "tap",
        };
        format!("Install {} {} via brew", type_str, self.name)
    }

    fn resource_type(&self) -> &'static str {
        match self.package_type {
            BrewPackageType::Formula => "brew_formula",
            BrewPackageType::Cask => "brew_cask",
            BrewPackageType::Tap => "brew_tap",
        }
    }

    fn sudo_requirement(&self) -> SudoRequirement {
        if self.requires_sudo {
            SudoRequirement::Required {
                reason: format!("Installing {} requires sudo", self.name),
            }
        } else {
            SudoRequirement::None
        }
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
    fn unsupported_source_specs_fail_before_query_or_install() {
        for name in [
            "/tmp/formula.rb",
            "./formula.rb",
            "../formula.rb",
            "owner//token",
            "Node",
            "owner/homebrew-repo/token",
            "--option",
        ] {
            assert!(BrewPackage::formula(name).is_installed().is_err());
        }
    }
}
