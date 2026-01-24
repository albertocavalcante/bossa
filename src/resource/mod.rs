//! Resource implementations for bossa
//!
//! This module provides concrete resource implementations using the
//! declarative crate's Resource trait.

#![allow(dead_code)]

// Re-export core types from declarative crate
pub use declarative::{
    ApplyContext, ApplyResult, Resource, ResourceState, SudoRequirement,
};

// Bossa-specific resource implementations
pub mod brew_package;
pub mod dock;
pub mod file_handler;
pub mod gh_extension;
pub mod macos_default;
pub mod pnpm_package;
pub mod service;
pub mod symlink;
pub mod vscode_extension;

pub use brew_package::BrewPackage;
pub use dock::{DockApp, DockFolder};
pub use file_handler::FileHandler;
pub use gh_extension::GHExtension;
pub use macos_default::{DefaultValue, MacOSDefault};
pub use pnpm_package::PnpmPackage;
pub use symlink::Symlink;
pub use vscode_extension::VSCodeExtension;
