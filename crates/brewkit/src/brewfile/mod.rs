//! Brewfile parsing and generation.
//!
//! This module provides functions for reading and writing Homebrew Brewfiles.

pub mod parser;
pub mod writer;

pub use parser::{parse_file, parse_string};
pub use writer::{write_file, write_string, WriteOptions};
