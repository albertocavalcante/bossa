//! Scoped sudo context with deterministic allowlist
//!
//! Sudo is never requested for the entire process. Instead:
//! 1. Config defines which operations need sudo (allowlist)
//! 2. All changes are computed first (no sudo needed)
//! 3. Sudo is acquired once for privileged batch
//! 4. Sudo is released immediately after

#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use declarative::{CommandOutput, SudoClassifier, SudoProvider};
use serde::{Deserialize, Serialize};
use std::process::{Command, Output};

/// Configuration for sudo allowlist
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SudoConfig {
    /// Casks that require sudo (e.g., ["docker", "1password"])
    #[serde(default)]
    pub casks: Vec<String>,

    /// macOS defaults that require sudo
    #[serde(default)]
    pub defaults: Vec<String>,

    /// Custom operations requiring sudo
    #[serde(default)]
    pub operations: Vec<String>,
}

impl SudoConfig {
    /// Check if a cask requires sudo
    pub fn cask_requires_sudo(&self, name: &str) -> bool {
        self.casks.iter().any(|c| c == name)
    }

    /// Check if a default requires sudo
    pub fn default_requires_sudo(&self, domain_key: &str) -> bool {
        self.defaults.iter().any(|d| d == domain_key)
    }

    /// Check if an operation requires sudo
    pub fn operation_requires_sudo(&self, op: &str) -> bool {
        self.operations.iter().any(|o| o == op)
    }
}

/// Implement SudoClassifier for SudoConfig
impl SudoClassifier for SudoConfig {
    fn requires_sudo(&self, resource_type: &str, resource_id: &str) -> bool {
        match resource_type {
            "brew_cask" => self.cask_requires_sudo(resource_id),
            "macos_default" => self.default_requires_sudo(resource_id),
            _ => false,
        }
    }
}

/// Scoped sudo context - automatically invalidates on drop
pub struct SudoContext {
    validated: bool,
}

impl SudoContext {
    /// Acquire sudo privileges with a reason shown to user
    pub fn acquire(reason: &str) -> Result<Self> {
        // Prompt user with reason
        eprintln!();
        eprintln!("  Sudo required: {}", reason);
        eprintln!();

        // Validate sudo (will prompt for password)
        let status = Command::new("sudo")
            .args(["-v"])
            .status()
            .context("Failed to execute sudo")?;

        if !status.success() {
            bail!("Failed to acquire sudo privileges");
        }

        Ok(Self { validated: true })
    }

    /// Check if sudo is currently valid (without prompting)
    pub fn is_valid() -> bool {
        Command::new("sudo")
            .args(["-n", "true"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Run a command with sudo (internal)
    fn run_internal(&self, cmd: &str, args: &[&str]) -> Result<Output> {
        if !self.validated {
            bail!("Sudo context not validated");
        }

        let output = Command::new("sudo")
            .arg(cmd)
            .args(args)
            .output()
            .with_context(|| format!("Failed to execute: sudo {} {:?}", cmd, args))?;

        Ok(output)
    }

    /// Run a command with sudo and capture output
    pub fn run_capture(&self, cmd: &str, args: &[&str]) -> Result<String> {
        let output = self.run_internal(cmd, args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Command failed: {}", stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Run a command with sudo, returning success/failure
    pub fn run_status(&self, cmd: &str, args: &[&str]) -> Result<bool> {
        let output = self.run_internal(cmd, args)?;
        Ok(output.status.success())
    }
}

/// Implement SudoProvider for SudoContext
impl SudoProvider for SudoContext {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
        let output = self.run_internal(cmd, args)?;
        Ok(output.into())
    }
}

impl Drop for SudoContext {
    fn drop(&mut self) {
        // Invalidate sudo timestamp to release privileges
        let _ = Command::new("sudo").args(["-k"]).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sudo_config_defaults() {
        let config = SudoConfig::default();
        assert!(config.casks.is_empty());
        assert!(config.defaults.is_empty());
        assert!(config.operations.is_empty());
    }

    #[test]
    fn test_cask_requires_sudo() {
        let config = SudoConfig {
            casks: vec!["docker".to_string(), "1password".to_string()],
            ..Default::default()
        };
        assert!(config.cask_requires_sudo("docker"));
        assert!(config.cask_requires_sudo("1password"));
        assert!(!config.cask_requires_sudo("raycast"));
    }

    #[test]
    fn test_sudo_classifier() {
        let config = SudoConfig {
            casks: vec!["docker".to_string()],
            defaults: vec!["com.apple.system".to_string()],
            ..Default::default()
        };

        assert!(config.requires_sudo("brew_cask", "docker"));
        assert!(!config.requires_sudo("brew_cask", "raycast"));
        assert!(config.requires_sudo("macos_default", "com.apple.system"));
        assert!(!config.requires_sudo("brew_formula", "ripgrep"));
    }
}
