//! Message output functions for consistent CLI feedback.
//!
//! This module provides functions for printing different types of messages
//! with consistent styling across your CLI application.
//!
//! ## Message Types
//!
//! | Function | Icon | Color | Use Case |
//! |----------|------|-------|----------|
//! | [`info`] | ℹ | Blue | General information |
//! | [`success`] | ✓ | Green | Operation completed successfully |
//! | [`warn`] | ⚠ | Yellow | Warning, non-fatal issue |
//! | [`error`] | ✗ | Red | Error, operation failed |
//! | [`dim`] | (none) | Gray | Secondary/muted information |
//!
//! ## Example
//!
//! ```no_run
//! use termkit::messages;
//!
//! messages::info("Starting deployment...");
//! messages::warn("Using default configuration");
//! messages::success("Deployment complete!");
//! messages::dim("  Took 3.2 seconds");
//! ```

use colored::Colorize;

/// Print an info message with a blue ℹ icon.
///
/// Use for general information that doesn't indicate success or failure.
///
/// # Example
///
/// ```no_run
/// termkit::messages::info("Processing 42 files...");
/// // Output: ℹ Processing 42 files...
/// ```
pub fn info(msg: &str) {
    println!("{} {}", "ℹ".blue(), msg);
}

/// Print a success message with a green ✓ icon.
///
/// Use when an operation completes successfully.
///
/// # Example
///
/// ```no_run
/// termkit::messages::success("All tests passed");
/// // Output: ✓ All tests passed
/// ```
pub fn success(msg: &str) {
    println!("{} {}", "✓".green(), msg);
}

/// Print a warning message with a yellow ⚠ icon.
///
/// Use for non-fatal issues that the user should be aware of.
///
/// # Example
///
/// ```no_run
/// termkit::messages::warn("Config file not found, using defaults");
/// // Output: ⚠ Config file not found, using defaults
/// ```
pub fn warn(msg: &str) {
    println!("{} {}", "⚠".yellow(), msg);
}

/// Print an error message with a red ✗ icon.
///
/// Use when an operation fails. Prints to stderr.
///
/// # Example
///
/// ```no_run
/// termkit::messages::error("Failed to connect to database");
/// // Output (stderr): ✗ Failed to connect to database
/// ```
pub fn error(msg: &str) {
    eprintln!("{} {}", "✗".red(), msg);
}

/// Print a dim/muted message.
///
/// Use for secondary information, details, or context that shouldn't
/// draw attention away from primary content.
///
/// # Example
///
/// ```no_run
/// termkit::messages::success("Build complete");
/// termkit::messages::dim("  Output: ./target/release/myapp");
/// termkit::messages::dim("  Size: 4.2 MB");
/// ```
pub fn dim(msg: &str) {
    println!("  {}", msg.dimmed());
}

#[cfg(test)]
mod tests {
    // Note: These functions print to stdout/stderr, so we mainly test
    // that they don't panic. Visual output testing would require
    // capturing stdout which adds complexity.

    use super::*;

    #[test]
    fn test_info_does_not_panic() {
        // Just verify it doesn't panic
        info("test message");
    }

    #[test]
    fn test_success_does_not_panic() {
        success("test message");
    }

    #[test]
    fn test_warn_does_not_panic() {
        warn("test message");
    }

    #[test]
    fn test_error_does_not_panic() {
        error("test message");
    }

    #[test]
    fn test_dim_does_not_panic() {
        dim("test message");
    }

    #[test]
    fn test_empty_messages() {
        info("");
        success("");
        warn("");
        error("");
        dim("");
    }

    #[test]
    fn test_unicode_messages() {
        info("Hello 世界 🌍");
        success("Completed ✨");
    }
}
