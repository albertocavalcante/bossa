//! Explicit inputs and native adapters for Bossa application use cases.
//!
//! The CLI owns argument parsing, logging, terminal interaction, and exit handling.
//! This boundary is intentionally small while commands migrate incrementally.

mod app;
#[cfg(feature = "native")]
mod application;
#[cfg(feature = "native")]
mod configuration;
#[cfg(feature = "native")]
mod filesystem;
mod format;
mod json;
mod legacy_config;
#[cfg(feature = "native")]
mod loaded_config;
mod location;
pub mod schema;
#[cfg(feature = "native")]
pub mod setup;
#[cfg(feature = "native")]
mod store;
mod typed_config;

pub use app::AppEnvironment;
#[cfg(feature = "native")]
pub use app::AppPaths;
#[cfg(feature = "native")]
pub use application::{ResourceApplyOptions, apply_resources, confirm_apply};
#[cfg(feature = "native")]
pub use bossa_process::{CommandRequest, IoMode, ProcessLimits, ProcessRunner, SystemRunner};
#[cfg(feature = "native")]
pub use configuration::{
    ConfigDocument, ConfigError, ConfigSource, FileConfigSource, load_config_document,
    write_config_document,
};
#[cfg(feature = "native")]
pub use filesystem::confirm_missing_parent;
pub use format::ConfigFormat;
pub use json::decode_unique_json;
#[cfg(feature = "native")]
pub use loaded_config::{LoadedBossaConfig, update_bossa_config};
pub use location::resolve_location_path;
#[cfg(feature = "native")]
pub use store::{DocumentStore, StoreError, StoreGuard, StoreRevision, StoreSnapshot};
pub use typed_config::{DecodedBossaConfig, decode_bossa_config};
