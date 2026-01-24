//! Resource trait for declarative state management
//!
//! A Resource represents something that can be in a certain state,
//! and can be changed to reach a desired state.

use crate::context::ApplyContext;
use crate::types::{ApplyResult, ResourceState, SudoRequirement};
use anyhow::Result;
use std::fmt;

/// Core trait for declarative resources
///
/// Every resource in the system implements this trait, which provides:
/// - Identity (id, description, type)
/// - State detection (current vs desired)
/// - State convergence (apply)
/// - Privilege requirements
///
/// # Example
///
/// ```ignore
/// use declarative::{Resource, ResourceState, ApplyResult, ApplyContext};
///
/// #[derive(Debug)]
/// struct FileResource {
///     path: String,
///     content: String,
/// }
///
/// impl Resource for FileResource {
///     fn id(&self) -> String {
///         self.path.clone()
///     }
///
///     fn description(&self) -> String {
///         format!("Ensure file exists at {}", self.path)
///     }
///
///     fn resource_type(&self) -> &'static str {
///         "file"
///     }
///
///     fn current_state(&self) -> Result<ResourceState> {
///         if std::path::Path::new(&self.path).exists() {
///             Ok(ResourceState::Present { details: None })
///         } else {
///             Ok(ResourceState::Absent)
///         }
///     }
///
///     fn desired_state(&self) -> ResourceState {
///         ResourceState::Present { details: None }
///     }
///
///     fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
///         if ctx.dry_run {
///             return Ok(ApplyResult::Skipped { reason: "Dry run".into() });
///         }
///         std::fs::write(&self.path, &self.content)?;
///         Ok(ApplyResult::Created)
///     }
/// }
/// ```
pub trait Resource: Send + Sync + fmt::Debug {
    /// Unique identifier for this resource
    ///
    /// This should be stable and uniquely identify the resource
    /// within its type. Examples:
    /// - "ripgrep" for a brew package
    /// - "com.apple.finder.ShowPathbar" for a macOS default
    /// - "~/.config/starship.toml" for a symlink
    fn id(&self) -> String;

    /// Human-readable description of what this resource does
    fn description(&self) -> String;

    /// Resource type category
    ///
    /// Used for grouping and filtering. Examples:
    /// - "brew_formula", "brew_cask", "brew_tap"
    /// - "macos_default"
    /// - "symlink"
    fn resource_type(&self) -> &'static str;

    /// Whether this resource requires elevated privileges
    ///
    /// Return `SudoRequirement::Required` with a reason if sudo is needed.
    /// This is typically determined by configuration, not hardcoded.
    fn sudo_requirement(&self) -> SudoRequirement {
        SudoRequirement::None
    }

    /// Detect the current state of this resource
    ///
    /// This should query the system to determine what state
    /// the resource is currently in.
    fn current_state(&self) -> Result<ResourceState>;

    /// Get the desired state for this resource
    ///
    /// This is typically derived from configuration.
    fn desired_state(&self) -> ResourceState;

    /// Check if the resource needs changes to reach desired state
    ///
    /// Default implementation compares current and desired states.
    fn needs_apply(&self) -> Result<bool> {
        let current = self.current_state()?;
        let desired = self.desired_state();
        Ok(current != desired)
    }

    /// Apply changes to reach the desired state
    ///
    /// This method should:
    /// 1. Check if already in desired state (return NoChange)
    /// 2. Respect ctx.dry_run (return Skipped if true)
    /// 3. Make the necessary changes
    /// 4. Return the appropriate ApplyResult
    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult>;

    /// Whether this resource can be applied in parallel with others
    ///
    /// Override to return false for resources that have ordering
    /// dependencies or shared state concerns.
    fn can_parallelize(&self) -> bool {
        true
    }
}

/// A boxed resource for type-erased storage
pub type BoxedResource = Box<dyn Resource>;

/// Extension trait for working with boxed resources
pub trait ResourceExt {
    /// Check if the resource requires sudo based on its requirement
    fn requires_sudo(&self) -> bool;
}

impl<R: Resource + ?Sized> ResourceExt for R {
    fn requires_sudo(&self) -> bool {
        matches!(self.sudo_requirement(), SudoRequirement::Required { .. })
    }
}
