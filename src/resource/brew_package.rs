//! Homebrew package resource

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::process::Command;

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

#[derive(Deserialize)]
struct BrewInfo {
    installed: Vec<BrewInstalled>,
}

#[derive(Deserialize)]
struct BrewInstalled {
    version: String,
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
        match self.package_type {
            BrewPackageType::Tap => {
                let output = Command::new("brew")
                    .args(["tap"])
                    .output()
                    .context("Failed to run brew tap")?;
                let taps = String::from_utf8_lossy(&output.stdout);
                Ok(taps.lines().any(|t| t.trim() == self.name))
            }
            BrewPackageType::Formula | BrewPackageType::Cask => {
                let type_flag = match self.package_type {
                    BrewPackageType::Cask => "--cask",
                    _ => "--formula",
                };

                let output = Command::new("brew")
                    .args(["info", "--json=v2", type_flag, &self.name])
                    .output()
                    .context("Failed to run brew info")?;

                if !output.status.success() {
                    return Ok(false);
                }

                // Parse JSON to check if installed
                let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;

                let installed = match self.package_type {
                    BrewPackageType::Cask => json["casks"]
                        .as_array()
                        .and_then(|arr| arr.first())
                        .and_then(|c| c["installed"].as_str())
                        .is_some(),
                    _ => json["formulae"]
                        .as_array()
                        .and_then(|arr| arr.first())
                        .and_then(|f| f["installed"].as_array())
                        .is_some_and(|arr| !arr.is_empty()),
                };

                Ok(installed)
            }
        }
    }

    /// Install the package
    fn install(&self, ctx: &ApplyContext) -> Result<()> {
        let args: Vec<&str> = match self.package_type {
            BrewPackageType::Tap => vec!["tap", &self.name],
            BrewPackageType::Formula => vec!["install", "--formula", &self.name],
            BrewPackageType::Cask => vec!["install", "--cask", &self.name],
        };

        let (success, stderr) = if self.requires_sudo {
            let output = ctx
                .sudo
                .ok_or_else(|| anyhow::anyhow!("Sudo required but not available"))?
                .run("brew", &args)?;
            (output.success, output.stderr_str())
        } else {
            let output = Command::new("brew")
                .args(&args)
                .output()
                .context("Failed to run brew install")?;
            (
                output.status.success(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            )
        };

        if !success {
            bail!("brew install failed: {}", stderr.trim());
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
