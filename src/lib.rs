//! Explicit inputs and native adapters for Bossa application use cases.
//!
//! The CLI owns argument parsing, logging, terminal interaction, and exit handling.
//! This boundary is intentionally small while commands migrate incrementally.

mod app;
mod application;
mod configuration;
mod process;

pub use app::{AppEnvironment, AppPaths};
pub use application::{ResourceApplyOptions, apply_resources};
pub use configuration::{
    ConfigDocument, ConfigError, ConfigFormat, ConfigSource, FileConfigSource,
    load_config_document, write_config_document,
};
pub use process::{CommandRequest, IoMode, ProcessRunner, SystemRunner};
