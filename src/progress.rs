//! Progress indicators for bossa CLI.
//!
//! This module re-exports pintui progress functions.

#[allow(unused_imports)]
pub use pintui::progress::{
    bar as clone_bar, finish_clear, finish_error, finish_success, finish_warn, spinner,
    StageProgress,
};
