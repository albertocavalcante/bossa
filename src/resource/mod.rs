//! Resource trait and types for declarative system configuration
//!
//! Every operation in bossa is modeled as a Resource with:
//! - State detection (current vs desired)
//! - Apply function (converge current → desired)
//! - Sudo requirements (deterministic, config-driven)

#![allow(dead_code)]

use anyhow::Result;
use std::fmt;

/// Requirement level for sudo privileges
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SudoRequirement {
    /// No sudo needed
    None,
    /// Sudo required with a reason
    Required { reason: String },
}

/// Current or desired state of a resource
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceState {
    /// Resource exists/is configured
    Present { details: Option<String> },
    /// Resource does not exist/is not configured
    Absent,
    /// Resource exists but differs from desired
    Modified { from: String, to: String },
    /// State cannot be determined
    Unknown,
}

/// Result of applying a resource
#[derive(Debug, Clone)]
pub enum ApplyResult {
    /// No changes needed
    NoChange,
    /// Resource was created
    Created,
    /// Resource was modified
    Modified,
    /// Resource was removed
    Removed,
    /// Apply failed
    Failed { error: String },
    /// Apply was skipped
    Skipped { reason: String },
}

/// Context passed to apply operations
pub struct ApplyContext<'a> {
    pub dry_run: bool,
    pub verbose: bool,
    pub sudo: Option<&'a crate::sudo::SudoContext>,
}

/// Core trait for all resources in bossa
pub trait Resource: Send + Sync + fmt::Debug {
    /// Unique identifier for this resource (e.g., "brew:ripgrep", "default:com.apple.finder.ShowPathbar")
    fn id(&self) -> String;

    /// Human-readable description
    fn description(&self) -> String;

    /// Resource type category (e.g., "brew_formula", "brew_cask", "macos_default", "symlink")
    fn resource_type(&self) -> &'static str;

    /// Whether this resource requires sudo (determined by config allowlist)
    fn sudo_requirement(&self) -> SudoRequirement {
        SudoRequirement::None
    }

    /// Detect current state of this resource
    fn current_state(&self) -> Result<ResourceState>;

    /// Get the desired state (from config)
    fn desired_state(&self) -> ResourceState;

    /// Check if resource needs changes
    fn needs_apply(&self) -> Result<bool> {
        let current = self.current_state()?;
        let desired = self.desired_state();
        Ok(current != desired)
    }

    /// Apply changes to reach desired state
    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult>;

    /// Whether this resource can be applied in parallel with others
    fn can_parallelize(&self) -> bool {
        true
    }
}

/// A diff between current and desired state
#[derive(Debug, Clone)]
pub struct ResourceDiff {
    pub resource_id: String,
    pub resource_type: String,
    pub description: String,
    pub current: ResourceState,
    pub desired: ResourceState,
    pub requires_sudo: bool,
}

impl ResourceDiff {
    pub fn from_resource(resource: &dyn Resource) -> Result<Option<Self>> {
        let current = resource.current_state()?;
        let desired = resource.desired_state();

        if current == desired {
            return Ok(None);
        }

        Ok(Some(Self {
            resource_id: resource.id(),
            resource_type: resource.resource_type().to_string(),
            description: resource.description(),
            current,
            desired,
            requires_sudo: matches!(
                resource.sudo_requirement(),
                SudoRequirement::Required { .. }
            ),
        }))
    }
}

// Re-export submodules
pub mod brew_package;
pub mod dock;
pub mod file_handler;
pub mod gh_extension;
pub mod macos_default;
pub mod pnpm_package;
pub mod service;
pub mod symlink;
pub mod vscode_extension;

pub use brew_package::BrewPackage;
pub use dock::{DockApp, DockFolder};
pub use file_handler::FileHandler;
pub use gh_extension::GHExtension;
pub use macos_default::{DefaultValue, MacOSDefault};
pub use pnpm_package::PnpmPackage;
pub use symlink::Symlink;
pub use vscode_extension::VSCodeExtension;
