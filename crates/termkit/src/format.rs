//! Formatting utilities for human-readable output.
//!
//! This module provides functions for converting data into
//! human-friendly string representations.
//!
//! ## Size Formatting
//!
//! ```
//! use termkit::format;
//!
//! assert_eq!(format::human_size(1024), "1.0 KB");
//! assert_eq!(format::human_size(1024 * 1024 * 50), "50.0 MB");
//! assert_eq!(format::human_size(1024u64.pow(4)), "1.00 TB");
//! ```
//!
//! ## Size Parsing
//!
//! ```
//! use termkit::format;
//!
//! assert_eq!(format::parse_size("100MB").unwrap(), 104_857_600);
//! assert_eq!(format::parse_size("1.5GB").unwrap(), 1_610_612_736);
//! ```

/// Byte size constants for formatting.
const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const GB: u64 = MB * 1024;
const TB: u64 = GB * 1024;

/// Format bytes as a human-readable size string.
///
/// Automatically selects the appropriate unit (B, KB, MB, GB, TB)
/// based on the magnitude of the value.
///
/// # Examples
///
/// ```
/// use termkit::format::human_size;
///
/// assert_eq!(human_size(0), "0 B");
/// assert_eq!(human_size(1023), "1023 B");
/// assert_eq!(human_size(1024), "1.0 KB");
/// assert_eq!(human_size(1024 * 1024), "1.0 MB");
/// assert_eq!(human_size(1024 * 1024 * 1024), "1.0 GB");
/// assert_eq!(human_size(1024u64 * 1024 * 1024 * 1024), "1.00 TB");
/// ```
pub fn human_size(bytes: u64) -> String {
    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Parse a human-readable size string into bytes.
///
/// Supports suffixes: B, KB, MB, GB, TB (case-insensitive).
/// Numbers without suffixes are treated as bytes.
/// Decimal values are supported (e.g., "1.5GB").
///
/// # Examples
///
/// ```
/// use termkit::format::parse_size;
///
/// // Without suffix (bytes)
/// assert_eq!(parse_size("100").unwrap(), 100);
///
/// // With suffix
/// assert_eq!(parse_size("1KB").unwrap(), 1024);
/// assert_eq!(parse_size("100MB").unwrap(), 104_857_600);
/// assert_eq!(parse_size("1.5GB").unwrap(), 1_610_612_736);
///
/// // Case insensitive
/// assert_eq!(parse_size("1kb").unwrap(), 1024);
/// assert_eq!(parse_size("1Kb").unwrap(), 1024);
///
/// // Whitespace is trimmed
/// assert_eq!(parse_size("  100MB  ").unwrap(), 104_857_600);
/// ```
///
/// # Errors
///
/// Returns an error if:
/// - The string is empty
/// - The numeric part cannot be parsed
/// - The value is negative
pub fn parse_size(size_str: &str) -> Result<u64, String> {
    let size_str = size_str.trim().to_uppercase();

    if size_str.is_empty() {
        return Err("Empty size string".to_string());
    }

    let (num_str, multiplier) = if let Some(num) = size_str.strip_suffix("TB") {
        (num, TB)
    } else if let Some(num) = size_str.strip_suffix("GB") {
        (num, GB)
    } else if let Some(num) = size_str.strip_suffix("MB") {
        (num, MB)
    } else if let Some(num) = size_str.strip_suffix("KB") {
        (num, KB)
    } else if let Some(num) = size_str.strip_suffix('B') {
        (num, 1u64)
    } else {
        // Assume bytes if no suffix
        (size_str.as_str(), 1u64)
    };

    let num: f64 = num_str
        .trim()
        .parse()
        .map_err(|_| format!("Invalid number in size: '{}'", num_str.trim()))?;

    if num < 0.0 {
        return Err(format!("Size cannot be negative: {}", num));
    }

    Ok((num * multiplier as f64) as u64)
}

/// Truncate a path string for display, preserving the end.
///
/// When a path is longer than `max_len`, it's truncated from the
/// beginning and prefixed with "..." to fit within the limit.
///
/// This preserves the filename and immediate parent directories,
/// which are usually the most relevant parts of a path.
///
/// # Examples
///
/// ```
/// use termkit::format::truncate_path;
///
/// // Short paths are unchanged
/// assert_eq!(truncate_path("short.txt", 20), "short.txt");
///
/// // Long paths are truncated from the start
/// assert_eq!(
///     truncate_path("/very/long/path/to/file.txt", 15),
///     ".../to/file.txt"
/// );
///
/// // Edge cases
/// assert_eq!(truncate_path("test", 3), "...");
/// assert_eq!(truncate_path("", 10), "");
/// ```
pub fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        format!("...{}", &path[path.len() - max_len + 3..])
    }
}

/// Format a count with singular/plural form.
///
/// # Examples
///
/// ```
/// use termkit::format::pluralize;
///
/// assert_eq!(pluralize(0, "file", "files"), "0 files");
/// assert_eq!(pluralize(1, "file", "files"), "1 file");
/// assert_eq!(pluralize(5, "file", "files"), "5 files");
/// ```
pub fn pluralize(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{} {}", count, singular)
    } else {
        format!("{} {}", count, plural)
    }
}

/// Format a duration in a human-readable way.
///
/// # Examples
///
/// ```
/// use termkit::format::human_duration;
/// use std::time::Duration;
///
/// assert_eq!(human_duration(Duration::from_millis(500)), "500ms");
/// assert_eq!(human_duration(Duration::from_secs(5)), "5.0s");
/// assert_eq!(human_duration(Duration::from_secs(90)), "1m 30s");
/// assert_eq!(human_duration(Duration::from_secs(3661)), "1h 1m");
/// ```
pub fn human_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();

    if secs == 0 {
        format!("{}ms", millis)
    } else if secs < 60 {
        format!("{}.{}s", secs, millis / 100)
    } else if secs < 3600 {
        let mins = secs / 60;
        let remaining_secs = secs % 60;
        format!("{}m {}s", mins, remaining_secs)
    } else {
        let hours = secs / 3600;
        let remaining_mins = (secs % 3600) / 60;
        format!("{}h {}m", hours, remaining_mins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // =========================================================================
    // human_size tests
    // =========================================================================

    #[test]
    fn test_human_size_bytes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(100), "100 B");
        assert_eq!(human_size(1023), "1023 B");
    }

    #[test]
    fn test_human_size_kb() {
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(10240), "10.0 KB");
    }

    #[test]
    fn test_human_size_mb() {
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
        assert_eq!(human_size(1024 * 1024 * 100), "100.0 MB");
    }

    #[test]
    fn test_human_size_gb() {
        assert_eq!(human_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(human_size(1024 * 1024 * 1024 * 2 + 1024 * 1024 * 512), "2.5 GB");
    }

    #[test]
    fn test_human_size_tb() {
        assert_eq!(human_size(1024u64 * 1024 * 1024 * 1024), "1.00 TB");
        assert_eq!(human_size(1024u64 * 1024 * 1024 * 1024 * 2), "2.00 TB");
    }

    // =========================================================================
    // parse_size tests
    // =========================================================================

    #[test]
    fn test_parse_size_bytes() {
        assert_eq!(parse_size("100").unwrap(), 100);
        assert_eq!(parse_size("100B").unwrap(), 100);
        assert_eq!(parse_size("100b").unwrap(), 100);
    }

    #[test]
    fn test_parse_size_kb() {
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("1kb").unwrap(), 1024);
        assert_eq!(parse_size("10KB").unwrap(), 10240);
    }

    #[test]
    fn test_parse_size_mb() {
        assert_eq!(parse_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("100MB").unwrap(), 100 * 1024 * 1024);
        assert_eq!(parse_size("1.5MB").unwrap(), (1.5 * 1024.0 * 1024.0) as u64);
    }

    #[test]
    fn test_parse_size_gb() {
        assert_eq!(parse_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("2GB").unwrap(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_tb() {
        assert_eq!(parse_size("1TB").unwrap(), 1024u64 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_whitespace() {
        assert_eq!(parse_size("  100MB  ").unwrap(), 100 * 1024 * 1024);
        assert_eq!(parse_size(" 1 GB").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_errors() {
        assert!(parse_size("").is_err());
        assert!(parse_size("abc").is_err());
        assert!(parse_size("MB").is_err());
        assert!(parse_size("-100MB").is_err());
    }

    // =========================================================================
    // truncate_path tests
    // =========================================================================

    #[test]
    fn test_truncate_path_short() {
        assert_eq!(truncate_path("short.txt", 20), "short.txt");
        assert_eq!(truncate_path("exact", 5), "exact");
    }

    #[test]
    fn test_truncate_path_long() {
        assert_eq!(
            truncate_path("/very/long/path/to/file.txt", 15),
            ".../to/file.txt"
        );
    }

    #[test]
    fn test_truncate_path_edge_cases() {
        assert_eq!(truncate_path("test", 3), "...");
        assert_eq!(truncate_path("test", 2), "...");
        assert_eq!(truncate_path("", 10), "");
    }

    // =========================================================================
    // pluralize tests
    // =========================================================================

    #[test]
    fn test_pluralize() {
        assert_eq!(pluralize(0, "file", "files"), "0 files");
        assert_eq!(pluralize(1, "file", "files"), "1 file");
        assert_eq!(pluralize(2, "file", "files"), "2 files");
        assert_eq!(pluralize(100, "item", "items"), "100 items");
    }

    // =========================================================================
    // human_duration tests
    // =========================================================================

    #[test]
    fn test_human_duration_millis() {
        assert_eq!(human_duration(Duration::from_millis(0)), "0ms");
        assert_eq!(human_duration(Duration::from_millis(500)), "500ms");
        assert_eq!(human_duration(Duration::from_millis(999)), "999ms");
    }

    #[test]
    fn test_human_duration_seconds() {
        assert_eq!(human_duration(Duration::from_secs(1)), "1.0s");
        assert_eq!(human_duration(Duration::from_secs(5)), "5.0s");
        assert_eq!(human_duration(Duration::from_millis(5500)), "5.5s");
    }

    #[test]
    fn test_human_duration_minutes() {
        assert_eq!(human_duration(Duration::from_secs(60)), "1m 0s");
        assert_eq!(human_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(human_duration(Duration::from_secs(3599)), "59m 59s");
    }

    #[test]
    fn test_human_duration_hours() {
        assert_eq!(human_duration(Duration::from_secs(3600)), "1h 0m");
        assert_eq!(human_duration(Duration::from_secs(3661)), "1h 1m");
        assert_eq!(human_duration(Duration::from_secs(7200)), "2h 0m");
    }
}
