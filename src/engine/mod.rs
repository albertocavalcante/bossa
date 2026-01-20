//! Execution engine for bossa
//!
//! The engine orchestrates:
//! 1. Planning - Build resource graph from config
//! 2. Diffing - Compute current vs desired state
//! 3. Executing - Apply changes with parallelism and sudo batching

#![allow(dead_code)]

pub mod differ;
pub mod executor;
pub mod planner;

pub use executor::{ExecuteOptions, execute};
pub use planner::ExecutionPlan;
