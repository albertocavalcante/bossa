//! # Declarative
//!
//! A framework for declarative resource management.
//!
//! This crate provides the core abstractions for declaring desired state,
//! detecting current state, and converging systems to match the desired state.
//!
//! ## Core Concepts
//!
//! - **Resource**: Something with state that can be managed (files, packages, settings)
//! - **ResourceState**: The current or desired state of a resource
//! - **ExecutionPlan**: A plan grouping resources by privilege level
//! - **Executor**: Applies resources with parallelism and privilege batching
//!
//! ## Example
//!
//! ```ignore
//! use declarative::{
//!     Resource, ResourceState, ApplyResult, ApplyContext,
//!     ExecutionPlan, ExecuteOptions, execute_simple,
//! };
//!
//! // Define a resource
//! #[derive(Debug)]
//! struct FileResource { path: String, content: String }
//!
//! impl Resource for FileResource {
//!     fn id(&self) -> String { self.path.clone() }
//!     fn description(&self) -> String { format!("File: {}", self.path) }
//!     fn resource_type(&self) -> &'static str { "file" }
//!
//!     fn current_state(&self) -> anyhow::Result<ResourceState> {
//!         if std::path::Path::new(&self.path).exists() {
//!             Ok(ResourceState::Present { details: None })
//!         } else {
//!             Ok(ResourceState::Absent)
//!         }
//!     }
//!
//!     fn desired_state(&self) -> ResourceState {
//!         ResourceState::Present { details: None }
//!     }
//!
//!     fn apply(&self, ctx: &mut ApplyContext) -> anyhow::Result<ApplyResult> {
//!         if ctx.dry_run {
//!             return Ok(ApplyResult::Skipped { reason: "Dry run".into() });
//!         }
//!         std::fs::write(&self.path, &self.content)?;
//!         Ok(ApplyResult::Created)
//!     }
//! }
//!
//! // Build and execute a plan
//! let mut plan = ExecutionPlan::new();
//! plan.unprivileged.push(Box::new(FileResource {
//!     path: "/tmp/test.txt".into(),
//!     content: "hello".into(),
//! }));
//!
//! let summary = execute_simple(plan, ExecuteOptions::default(), || {
//!     anyhow::bail!("No sudo needed")
//! })?;
//! ```
//!
//! ## Provider Traits
//!
//! The crate uses traits for dependency injection:
//!
//! - [`SudoProvider`]: Provides elevated privilege execution
//! - [`SudoClassifier`]: Determines which resources need privileges
//! - [`ProgressCallback`]: Receives progress updates
//! - [`ConfirmCallback`]: Handles user confirmations
//!
//! This allows the crate to be used without hard dependencies on
//! specific UI frameworks, sudo implementations, etc.

pub mod context;
pub mod diff;
pub mod executor;
pub mod planner;
pub mod resource;
pub mod types;

// Re-export main types at crate root
pub use context::{
    ApplyContext, AutoConfirm, AutoDecline, ConfirmCallback, NoProgress, NoSudo,
    ProgressCallback, SudoClassifier, SudoProvider,
};
pub use diff::{compute_diffs, group_by_type, DiffSummary, ResourceDiff};
pub use executor::{execute, execute_simple};
pub use planner::ExecutionPlan;
pub use resource::{BoxedResource, Resource, ResourceExt};
pub use types::{
    ApplyResult, CommandOutput, ExecuteOptions, ExecuteSummary, ResourceState, SudoRequirement,
};
