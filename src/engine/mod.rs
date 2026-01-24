//! Execution engine for bossa
//!
//! This module provides bossa-specific execution logic on top of the
//! declarative crate's generic types.

#![allow(dead_code)]

// Re-export core types from declarative crate
pub use declarative::ExecutionPlan;

// Bossa-specific modules
pub mod differ;
pub mod executor;
pub mod planner;

pub use executor::{execute, ExecuteOptions};
pub use planner::ExecutionPlanExt;
