//! Explicit inputs and native adapters for Bossa application use cases.
//!
//! The CLI owns argument parsing, logging, terminal interaction, and exit handling.
//! This boundary is intentionally small while commands migrate incrementally.

mod app;
mod process;

pub use app::{AppEnvironment, AppPaths};
pub use process::{CommandRequest, IoMode, ProcessRunner, SystemRunner};
