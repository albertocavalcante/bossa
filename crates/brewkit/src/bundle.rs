//! Bundle output parsing for `brew bundle --verbose`.
//!
//! Parses the verbose output to extract per-package installation results.

use crate::types::BundleResult;

/// Parse verbose output from `brew bundle --verbose`.
///
/// Extracts package names and their installation status (success, skip, fail).
pub fn parse_verbose_output(output: &str) -> BundleResult {
    let mut result = BundleResult::default();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Success patterns
        if let Some(rest) = line.strip_prefix("Installing ") {
            let name = extract_name(rest);
            result.installed.push(name);
        } else if let Some(rest) = line.strip_prefix("Tapping ") {
            let name = extract_name(rest);
            result.installed.push(name);
        } else if let Some(rest) = line.strip_prefix("Upgrading ") {
            let name = extract_name(rest);
            result.upgraded.push(name);
        }
        // Skip patterns
        else if line.contains("already installed") {
            if let Some(name) = extract_quoted_name(line) {
                result.skipped.push(name);
            } else if let Some(rest) = line.strip_prefix("Using ") {
                let name = extract_name(rest);
                result.skipped.push(name);
            }
        } else if let Some(rest) = line.strip_prefix("Using ") {
            let name = extract_name(rest);
            result.skipped.push(name);
        } else if let Some(rest) = line.strip_prefix("Skipping install of ") {
            let name = extract_name(rest);
            result.skipped.push(name);
        }
        // Failure patterns
        else if (line.starts_with("Error:") || line.contains(" failed"))
            && let Some(name) = extract_error_package(line)
        {
            result.failed.push((name, line.to_string()));
        } else if (line.contains("No available formula") || line.contains("No cask with this name"))
            && let Some(name) = extract_quoted_name(line)
        {
            result.failed.push((name, line.to_string()));
        }
    }

    result
}

/// Extract package name from a line (first word, trimmed).
fn extract_name(rest: &str) -> String {
    rest.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches(':')
        .to_string()
}

/// Extract a quoted name from a line.
fn extract_quoted_name(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

/// Extract package name from an error line.
fn extract_error_package(line: &str) -> Option<String> {
    // Try quoted name first
    if let Some(name) = extract_quoted_name(line) {
        return Some(name);
    }

    // Try "Error: name:" pattern
    if let Some(rest) = line.strip_prefix("Error: ") {
        let name = rest.split(':').next()?.trim();
        if !name.is_empty() && !name.contains(' ') {
            return Some(name.to_string());
        }
    }

    None
}

/// Merge multiple BundleResults into one.
pub fn merge_results(results: Vec<BundleResult>) -> BundleResult {
    let mut merged = BundleResult::default();
    for r in results {
        merged.merge(r);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_installing() {
        let output = "Installing git\nInstalling curl";
        let result = parse_verbose_output(output);
        assert_eq!(result.installed, vec!["git", "curl"]);
    }

    #[test]
    fn test_parse_tapping() {
        let output = "Tapping homebrew/cask";
        let result = parse_verbose_output(output);
        assert_eq!(result.installed, vec!["homebrew/cask"]);
    }

    #[test]
    fn test_parse_already_installed() {
        let output = r#"Warning: "git" is already installed"#;
        let result = parse_verbose_output(output);
        assert_eq!(result.skipped, vec!["git"]);
    }

    #[test]
    fn test_parse_using() {
        let output = "Using git";
        let result = parse_verbose_output(output);
        assert_eq!(result.skipped, vec!["git"]);
    }

    #[test]
    fn test_parse_upgrading() {
        let output = "Upgrading git";
        let result = parse_verbose_output(output);
        assert_eq!(result.upgraded, vec!["git"]);
    }

    #[test]
    fn test_parse_error() {
        let output = r#"Error: No available formula with the name "nonexistent""#;
        let result = parse_verbose_output(output);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].0, "nonexistent");
    }

    #[test]
    fn test_parse_mixed() {
        let output = r#"
Tapping homebrew/cask
Installing git
Using curl
Error: foo: not found
Upgrading wget
"#;
        let result = parse_verbose_output(output);
        assert_eq!(result.installed, vec!["homebrew/cask", "git"]);
        assert_eq!(result.skipped, vec!["curl"]);
        assert_eq!(result.upgraded, vec!["wget"]);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].0, "foo");
    }

    #[test]
    fn test_extract_quoted_name() {
        assert_eq!(
            extract_quoted_name(r#"Warning: "git" is already installed"#),
            Some("git".to_string())
        );
        assert_eq!(extract_quoted_name("no quotes here"), None);
    }

    #[test]
    fn test_merge_results() {
        let r1 = BundleResult {
            installed: vec!["git".to_string()],
            skipped: vec!["curl".to_string()],
            ..Default::default()
        };
        let r2 = BundleResult {
            installed: vec!["wget".to_string()],
            failed: vec![("foo".to_string(), "error".to_string())],
            ..Default::default()
        };

        let merged = merge_results(vec![r1, r2]);
        assert_eq!(merged.installed, vec!["git", "wget"]);
        assert_eq!(merged.skipped, vec!["curl"]);
        assert_eq!(merged.failed.len(), 1);
    }
}
