//! Core types for declarative resource management

use serde::{Deserialize, Serialize};
use std::process::Output;

/// Requirement level for sudo/elevated privileges
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SudoRequirement {
    /// No elevated privileges needed
    None,
    /// Elevated privileges required with a reason
    Required { reason: String },
}

impl Default for SudoRequirement {
    fn default() -> Self {
        Self::None
    }
}

/// Current or desired state of a resource
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl ResourceState {
    /// Check if state represents presence
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present { .. })
    }

    /// Check if state represents absence
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

/// Result of applying a resource
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl ApplyResult {
    /// Check if the result represents success (no failure)
    pub fn is_success(&self) -> bool {
        !matches!(self, Self::Failed { .. })
    }

    /// Check if the result represents a change
    pub fn is_change(&self) -> bool {
        matches!(self, Self::Created | Self::Modified | Self::Removed)
    }
}

/// Summary of execution results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecuteSummary {
    pub created: usize,
    pub modified: usize,
    pub removed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub no_change: usize,
}

impl ExecuteSummary {
    /// Total number of actual changes made
    pub fn total_changes(&self) -> usize {
        self.created + self.modified + self.removed
    }

    /// Check if execution was fully successful (no failures)
    pub fn is_success(&self) -> bool {
        self.failed == 0
    }

    /// Total number of resources processed
    pub fn total(&self) -> usize {
        self.created + self.modified + self.removed + self.skipped + self.failed + self.no_change
    }

    /// Merge another summary into this one
    pub fn merge(&mut self, other: &ExecuteSummary) {
        self.created += other.created;
        self.modified += other.modified;
        self.removed += other.removed;
        self.skipped += other.skipped;
        self.failed += other.failed;
        self.no_change += other.no_change;
    }

    /// Add a result to the summary
    pub fn add_result(&mut self, result: &ApplyResult) {
        match result {
            ApplyResult::NoChange => self.no_change += 1,
            ApplyResult::Created => self.created += 1,
            ApplyResult::Modified => self.modified += 1,
            ApplyResult::Removed => self.removed += 1,
            ApplyResult::Failed { .. } => self.failed += 1,
            ApplyResult::Skipped { .. } => self.skipped += 1,
        }
    }
}

/// Options for execution
#[derive(Debug, Clone)]
pub struct ExecuteOptions {
    /// Don't make changes, just show what would happen
    pub dry_run: bool,
    /// Number of parallel jobs for unprivileged operations
    pub jobs: usize,
    /// Verbose output
    pub verbose: bool,
}

impl Default for ExecuteOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            jobs: 4,
            verbose: false,
        }
    }
}

/// Output from a privileged command
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub success: bool,
}

impl From<Output> for CommandOutput {
    fn from(output: Output) -> Self {
        Self {
            stdout: output.stdout,
            stderr: output.stderr,
            success: output.status.success(),
        }
    }
}

impl CommandOutput {
    /// Get stdout as a string
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    /// Get stderr as a string
    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }
}
