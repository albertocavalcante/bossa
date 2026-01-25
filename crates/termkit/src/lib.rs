//! # termkit
//!
//! Terminal UI toolkit for building beautiful CLI applications.
//!
//! This crate provides reusable building blocks for CLI interfaces:
//! - **Messages**: Consistent info, warning, error, and success output
//! - **Layout**: Headers, sections, key-value pairs, step indicators
//! - **Progress**: Spinners, progress bars, multi-stage tracking
//! - **Formatting**: Human-readable sizes, path truncation
//!
//! ## Quick Start
//!
//! ```no_run
//! use termkit::{messages, layout, progress, format};
//!
//! // Messages
//! messages::info("Starting operation...");
//! messages::success("Done!");
//!
//! // Layout
//! layout::header("Configuration");
//! layout::kv("Version", "1.0.0");
//!
//! // Progress
//! let spinner = progress::spinner("Loading...");
//! // ... do work ...
//! progress::finish_success(&spinner, "Loaded");
//!
//! // Formatting
//! let size = format::human_size(1024 * 1024 * 50);
//! assert_eq!(size, "50.0 MB");
//! ```
//!
//! ## Design Philosophy
//!
//! termkit provides opinionated defaults for a consistent look:
//! - Success: green checkmark (✓)
//! - Error: red cross (✗)
//! - Warning: yellow warning sign (⚠)
//! - Info: blue info icon (ℹ)
//! - Headers: cyan and bold
//!
//! This creates a unified visual language across all tools using termkit.

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod format;
pub mod layout;
pub mod messages;
pub mod progress;

// Re-export commonly used items at crate root for convenience
pub use format::{human_size, parse_size, truncate_path};
pub use layout::{header, kv, section, step};
pub use messages::{dim, error, info, success, warn};
pub use progress::{spinner, StageProgress};
