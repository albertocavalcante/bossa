//! Layout primitives for structuring CLI output.
//!
//! This module provides functions for organizing output into logical
//! sections with consistent visual hierarchy.
//!
//! ## Hierarchy
//!
//! ```text
//! header("Main Title")           # Bold with underline
//! ─────────────────
//!
//! section("Subsection")          # Cyan and bold
//!   kv("Key", "Value")           # Indented key-value
//!   kv("Another", "Value")
//!
//! step(1, 3, "First step")       # [1/3] First step
//! step(2, 3, "Second step")      # [2/3] Second step
//! ```
//!
//! ## Example
//!
//! ```no_run
//! use termkit::layout;
//!
//! layout::header("System Information");
//!
//! layout::section("Hardware");
//! layout::kv("CPU", "Apple M1 Pro");
//! layout::kv("Memory", "32 GB");
//!
//! layout::section("Software");
//! layout::kv("OS", "macOS 14.0");
//! layout::kv("Rust", "1.90.0");
//! ```

use colored::Colorize;

/// Print a header/title with an underline.
///
/// Headers are bold and followed by a dimmed line of the same width.
/// A blank line is printed before the header for visual separation.
///
/// # Example
///
/// ```no_run
/// termkit::layout::header("Configuration");
/// // Output:
/// //
/// // Configuration
/// // ─────────────
/// ```
pub fn header(title: &str) {
    println!();
    println!("{}", title.bold());
    println!("{}", "─".repeat(title.len()).dimmed());
}

/// Print a section header.
///
/// Sections are cyan and bold, with a blank line before for separation.
/// Use sections to group related information under a header.
///
/// # Example
///
/// ```no_run
/// termkit::layout::section("Dependencies");
/// // Output:
/// //
/// // Dependencies
/// ```
pub fn section(title: &str) {
    println!();
    println!("{}", title.cyan().bold());
}

/// Print a key-value pair.
///
/// The key is dimmed and the value is normal. Both are indented.
/// Use for displaying configuration, status, or metadata.
///
/// # Example
///
/// ```no_run
/// termkit::layout::kv("Version", "1.0.0");
/// termkit::layout::kv("Status", "Active");
/// // Output:
/// //   Version: 1.0.0
/// //   Status: Active
/// ```
pub fn kv(key: &str, value: &str) {
    println!("  {}: {}", key.dimmed(), value);
}

/// Print a step indicator for multi-step operations.
///
/// Shows progress through a sequence of steps with `[current/total]` prefix.
/// The indicator is blue and bold for visibility.
///
/// # Example
///
/// ```no_run
/// termkit::layout::step(1, 3, "Fetching dependencies");
/// termkit::layout::step(2, 3, "Compiling");
/// termkit::layout::step(3, 3, "Linking");
/// // Output:
/// // [1/3] Fetching dependencies
/// // [2/3] Compiling
/// // [3/3] Linking
/// ```
pub fn step(num: usize, total: usize, msg: &str) {
    println!("{} {}", format!("[{}/{}]", num, total).blue().bold(), msg);
}

/// Print a blank line for visual separation.
///
/// Convenience function for adding vertical whitespace.
pub fn blank() {
    println!();
}

/// Print a horizontal rule/divider.
///
/// # Arguments
///
/// * `width` - The width of the divider in characters
///
/// # Example
///
/// ```no_run
/// termkit::layout::divider(40);
/// // Output:
/// // ────────────────────────────────────────
/// ```
pub fn divider(width: usize) {
    println!("{}", "─".repeat(width).dimmed());
}

/// Print an indented line.
///
/// # Arguments
///
/// * `level` - Indentation level (each level = 2 spaces)
/// * `msg` - The message to print
///
/// # Example
///
/// ```no_run
/// termkit::layout::indent(1, "First level");
/// termkit::layout::indent(2, "Second level");
/// // Output:
/// //   First level
/// //     Second level
/// ```
pub fn indent(level: usize, msg: &str) {
    println!("{}{}", "  ".repeat(level), msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_does_not_panic() {
        header("Test Header");
    }

    #[test]
    fn test_section_does_not_panic() {
        section("Test Section");
    }

    #[test]
    fn test_kv_does_not_panic() {
        kv("key", "value");
    }

    #[test]
    fn test_step_does_not_panic() {
        step(1, 5, "First step");
        step(5, 5, "Last step");
    }

    #[test]
    fn test_blank_does_not_panic() {
        blank();
    }

    #[test]
    fn test_divider_does_not_panic() {
        divider(40);
        divider(0);
    }

    #[test]
    fn test_indent_does_not_panic() {
        indent(0, "No indent");
        indent(1, "One level");
        indent(5, "Five levels");
    }

    #[test]
    fn test_empty_values() {
        header("");
        section("");
        kv("", "");
        step(0, 0, "");
    }

    #[test]
    fn test_unicode() {
        header("日本語ヘッダー");
        section("セクション");
        kv("キー", "値");
    }
}
