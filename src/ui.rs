//! UI utilities for bossa CLI.
//!
//! This module re-exports pintui functions and adds bossa-specific utilities.

// Re-export all pintui functionality
#[allow(unused_imports)]
pub use pintui::format::human_size as format_size;
#[allow(unused_imports)]
pub use pintui::format::parse_size;
#[allow(unused_imports)]
pub use pintui::format::truncate_path;
#[allow(unused_imports)]
pub use pintui::layout::{header, kv, section, step};
#[allow(unused_imports)]
pub use pintui::messages::{dim, error, info, success, warn};

/// Print the bossa banner.
pub fn banner() {
    use colored::Colorize;
    println!(
        "{}",
        r#"
  ██████╗  ██████╗ ███████╗███████╗ █████╗
  ██╔══██╗██╔═══██╗██╔════╝██╔════╝██╔══██╗
  ██████╔╝██║   ██║███████╗███████╗███████║
  ██╔══██╗██║   ██║╚════██║╚════██║██╔══██║
  ██████╔╝╚██████╔╝███████║███████║██║  ██║
  ╚═════╝  ╚═════╝ ╚══════╝╚══════╝╚═╝  ╚═╝
"#
        .cyan()
    );
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(100), "100 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn test_format_size_kb() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(10240), "10.0 KB");
    }

    #[test]
    fn test_format_size_mb() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 100), "100.0 MB");
    }

    #[test]
    fn test_format_size_gb() {
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(
            format_size(1024 * 1024 * 1024 * 2 + 1024 * 1024 * 512),
            "2.5 GB"
        );
    }

    #[test]
    fn test_format_size_tb() {
        assert_eq!(format_size(1024u64 * 1024 * 1024 * 1024), "1.00 TB");
        assert_eq!(format_size(1024u64 * 1024 * 1024 * 1024 * 2), "2.00 TB");
    }

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

    #[test]
    fn test_truncate_path_short() {
        assert_eq!(truncate_path("short.txt", 20), "short.txt");
        assert_eq!(truncate_path("exact", 5), "exact");
    }

    #[test]
    fn test_truncate_path_long() {
        // "/very/long/path/to/file.txt" is 27 chars
        // max_len=15, so we take last 12 chars + "..." = ".../to/file.txt"
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
}
