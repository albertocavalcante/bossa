#![allow(dead_code)]

use colored::Colorize;

/// Print an info message
pub fn info(msg: &str) {
    println!("{} {}", "ℹ".blue(), msg);
}

/// Print a success message
pub fn success(msg: &str) {
    println!("{} {}", "✓".green(), msg);
}

/// Print a warning message
pub fn warn(msg: &str) {
    println!("{} {}", "⚠".yellow(), msg);
}

/// Print an error message
pub fn error(msg: &str) {
    eprintln!("{} {}", "✗".red(), msg);
}

/// Print a dim/muted message
pub fn dim(msg: &str) {
    println!("  {}", msg.dimmed());
}

/// Print a header/title
pub fn header(title: &str) {
    println!();
    println!("{}", title.bold());
    println!("{}", "─".repeat(title.len()).dimmed());
}

/// Print a section header
pub fn section(title: &str) {
    println!();
    println!("{}", title.cyan().bold());
}

/// Print a key-value pair
pub fn kv(key: &str, value: &str) {
    println!("  {}: {}", key.dimmed(), value);
}

/// Print a step indicator
pub fn step(num: usize, total: usize, msg: &str) {
    println!("{} {}", format!("[{}/{}]", num, total).blue().bold(), msg);
}

// ============================================================================
// Size Formatting
// ============================================================================

const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const GB: u64 = MB * 1024;
const TB: u64 = GB * 1024;

/// Format bytes as human-readable size
pub fn format_size(bytes: u64) -> String {
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

/// Parse human-readable size string (e.g., "100MB", "1GB", "500")
///
/// Supports suffixes: B, KB, MB, GB, TB (case-insensitive)
/// Returns bytes as u64
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

/// Truncate a path string for display, keeping the end
pub fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        format!("...{}", &path[path.len() - max_len + 3..])
    }
}

/// Print the bossa banner
pub fn banner() {
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
        assert_eq!(format_size(1024 * 1024 * 1024 * 2 + 1024 * 1024 * 512), "2.5 GB");
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
