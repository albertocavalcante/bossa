//! Apply context and provider traits
//!
//! These traits allow the declarative crate to be used without
//! depending on specific implementations of sudo, progress, etc.

use crate::types::{ApplyResult, CommandOutput};
use anyhow::Result;

/// Provider for elevated privilege operations
///
/// Implement this trait to provide sudo/admin capabilities.
/// The implementation handles privilege acquisition and release.
pub trait SudoProvider: Send + Sync {
    /// Run a command with elevated privileges
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput>;

    /// Run a command and return just success/failure
    fn run_status(&self, cmd: &str, args: &[&str]) -> Result<bool> {
        Ok(self.run(cmd, args)?.success)
    }

    /// Run a command and capture stdout
    fn run_capture(&self, cmd: &str, args: &[&str]) -> Result<String> {
        let output = self.run(cmd, args)?;
        if !output.success {
            anyhow::bail!("Command failed: {}", output.stderr_str().trim());
        }
        Ok(output.stdout_str())
    }
}

/// Classifier for determining which resources need elevated privileges
///
/// Implement this trait to define your privilege policy.
pub trait SudoClassifier: Send + Sync {
    /// Check if a resource requires elevated privileges
    ///
    /// # Arguments
    /// * `resource_type` - The type of resource (e.g., "brew_cask", "macos_default")
    /// * `resource_id` - The identifier of the resource
    fn requires_sudo(&self, resource_type: &str, resource_id: &str) -> bool;
}

/// Default classifier that never requires sudo
pub struct NoSudo;

impl SudoClassifier for NoSudo {
    fn requires_sudo(&self, _resource_type: &str, _resource_id: &str) -> bool {
        false
    }
}

/// Progress callback for execution operations
///
/// Implement this trait to receive progress updates during execution.
pub trait ProgressCallback: Send {
    /// Called when starting to apply a batch of resources
    fn on_batch_start(&mut self, count: usize, privileged: bool);

    /// Called when starting to apply a single resource
    fn on_resource_start(&mut self, id: &str, description: &str);

    /// Called when a resource application completes
    fn on_resource_complete(&mut self, id: &str, result: &ApplyResult);

    /// Called when a batch completes
    fn on_batch_complete(&mut self);
}

/// Confirmation callback for user interaction
///
/// Implement this trait to handle user confirmations.
pub trait ConfirmCallback: Send {
    /// Ask the user to confirm an action
    ///
    /// # Arguments
    /// * `prompt` - The confirmation prompt to show
    ///
    /// # Returns
    /// `true` if the user confirmed, `false` otherwise
    fn confirm(&mut self, prompt: &str) -> Result<bool>;
}

/// No-op progress callback
pub struct NoProgress;

impl ProgressCallback for NoProgress {
    fn on_batch_start(&mut self, _count: usize, _privileged: bool) {}
    fn on_resource_start(&mut self, _id: &str, _description: &str) {}
    fn on_resource_complete(&mut self, _id: &str, _result: &ApplyResult) {}
    fn on_batch_complete(&mut self) {}
}

/// Auto-confirm callback (always returns true)
pub struct AutoConfirm;

impl ConfirmCallback for AutoConfirm {
    fn confirm(&mut self, _prompt: &str) -> Result<bool> {
        Ok(true)
    }
}

/// Auto-decline callback (always returns false)
pub struct AutoDecline;

impl ConfirmCallback for AutoDecline {
    fn confirm(&mut self, _prompt: &str) -> Result<bool> {
        Ok(false)
    }
}

/// Context passed to resource apply operations
pub struct ApplyContext<'a> {
    /// Whether this is a dry run (no actual changes)
    pub dry_run: bool,
    /// Whether to output verbose information
    pub verbose: bool,
    /// Optional sudo provider for privileged operations
    pub sudo: Option<&'a dyn SudoProvider>,
}

impl<'a> ApplyContext<'a> {
    /// Create a new apply context
    pub fn new(dry_run: bool, verbose: bool) -> Self {
        Self {
            dry_run,
            verbose,
            sudo: None,
        }
    }

    /// Create a context with a sudo provider
    pub fn with_sudo(dry_run: bool, verbose: bool, sudo: &'a dyn SudoProvider) -> Self {
        Self {
            dry_run,
            verbose,
            sudo: Some(sudo),
        }
    }

    /// Get the sudo provider, or error if not available
    pub fn require_sudo(&self) -> Result<&dyn SudoProvider> {
        self.sudo
            .ok_or_else(|| anyhow::anyhow!("Sudo required but not available"))
    }
}
