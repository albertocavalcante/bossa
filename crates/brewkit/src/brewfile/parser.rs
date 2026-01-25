//! Parser for Brewfile Ruby DSL format.
//!
//! Handles the Homebrew Brewfile format which uses Ruby-like syntax:
//! ```text
//! tap "homebrew/cask-fonts"
//! brew "git", restart_service: :changed
//! cask "visual-studio-code"
//! mas "Xcode", id: 497799835
//! vscode "ms-python.python"
//! ```

use crate::error::{Error, Result};
use crate::types::{Brewfile, Package, PackageType};
use std::collections::HashMap;
use std::path::Path;

/// Parse a Brewfile from a file path.
pub fn parse_file(path: &Path) -> Result<Brewfile> {
    let content = std::fs::read_to_string(path)?;
    let mut brewfile = parse_string(&content)?;
    brewfile.path = Some(path.to_path_buf());
    Ok(brewfile)
}

/// Parse a Brewfile from a string.
pub fn parse_string(content: &str) -> Result<Brewfile> {
    let mut brewfile = Brewfile::new();

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Extract version comment if present (e.g., "# 2.40.0" at end of line)
        let (line, version_comment) = extract_version_comment(line);

        // Parse the package entry
        if let Some(mut package) = parse_line(line, line_num + 1)? {
            // Apply version from comment if present
            if let Some(version) = version_comment {
                package.version = Some(version);
            }
            brewfile.add(package);
        }
    }

    Ok(brewfile)
}

/// Extract version comment from end of line.
/// Returns (line without comment, optional version).
fn extract_version_comment(line: &str) -> (&str, Option<String>) {
    // Look for pattern: ... # version or ... # some version comment
    if let Some(hash_pos) = line.rfind('#') {
        let before = line[..hash_pos].trim();
        let comment = line[hash_pos + 1..].trim();

        // Check if comment looks like a version (starts with digit or 'v')
        if !comment.is_empty() {
            let first_char = comment.chars().next().unwrap();
            if first_char.is_ascii_digit() || (first_char == 'v' && comment.len() > 1) {
                // Extract version (first word of comment)
                let version = comment.split_whitespace().next().unwrap_or(comment);
                return (before, Some(version.to_string()));
            }
        }
    }
    (line, None)
}

/// Parse a single line of Brewfile.
fn parse_line(line: &str, line_num: usize) -> Result<Option<Package>> {
    // Find the directive (tap, brew, cask, mas, vscode)
    let (directive, rest) = match line.split_once(char::is_whitespace) {
        Some((d, r)) => (d, r.trim()),
        None => return Ok(None), // Line with just a word, ignore
    };

    let package_type = match PackageType::from_directive(directive) {
        Some(t) => t,
        None => return Ok(None), // Unknown directive, ignore
    };

    // Parse the rest of the line for name and options
    let (name, options) = parse_arguments(rest, line_num)?;

    let mut package = Package::new(name, package_type);
    package.options = options;

    Ok(Some(package))
}

/// Parse arguments from a Brewfile line.
/// Handles: "name", key: value, key: :symbol
fn parse_arguments(args: &str, line_num: usize) -> Result<(String, HashMap<String, String>)> {
    let mut options = HashMap::new();
    let args = args.trim();

    // Extract the package name (first quoted string or first word)
    let (name, rest) = extract_name(args, line_num)?;

    // Parse remaining options
    let rest = rest.trim();
    if !rest.is_empty() {
        // Skip leading comma if present
        let rest = rest.strip_prefix(',').unwrap_or(rest).trim();
        parse_options(rest, &mut options)?;
    }

    Ok((name, options))
}

/// Extract package name from the start of arguments.
fn extract_name(args: &str, line_num: usize) -> Result<(String, &str)> {
    let args = args.trim();

    // Check for double-quoted string
    if let Some(stripped) = args.strip_prefix('"') {
        // Find closing quote
        if let Some(end) = stripped.find('"') {
            let name = &stripped[..end];
            let rest = &stripped[end + 1..];
            return Ok((name.to_string(), rest));
        } else {
            return Err(Error::BrewfileParse {
                line: line_num,
                message: "unclosed quote".to_string(),
            });
        }
    }

    // Check for single-quoted string
    if let Some(stripped) = args.strip_prefix('\'') {
        if let Some(end) = stripped.find('\'') {
            let name = &stripped[..end];
            let rest = &stripped[end + 1..];
            return Ok((name.to_string(), rest));
        } else {
            return Err(Error::BrewfileParse {
                line: line_num,
                message: "unclosed quote".to_string(),
            });
        }
    }

    // Unquoted - take until comma or whitespace
    let end = args.find(|c: char| c == ',' || c.is_whitespace()).unwrap_or(args.len());
    let name = &args[..end];
    let rest = &args[end..];

    Ok((name.to_string(), rest))
}

/// Parse key: value options.
fn parse_options(options_str: &str, options: &mut HashMap<String, String>) -> Result<()> {
    let mut current = options_str.trim();

    while !current.is_empty() {
        // Skip commas and whitespace
        current = current.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
        if current.is_empty() {
            break;
        }

        // Find key (until colon)
        let colon_pos = match current.find(':') {
            Some(pos) => pos,
            None => break, // No more options
        };

        let key = current[..colon_pos].trim().to_string();
        current = current[colon_pos + 1..].trim();

        // Parse value
        let (value, rest) = parse_option_value(current);
        options.insert(key, value);
        current = rest.trim();
    }

    Ok(())
}

/// Parse an option value (quoted string, symbol, or number).
fn parse_option_value(value_str: &str) -> (String, &str) {
    let value_str = value_str.trim();

    // Double-quoted string
    if let Some(stripped) = value_str.strip_prefix('"') {
        if let Some(end) = stripped.find('"') {
            let value = &stripped[..end];
            let rest = &stripped[end + 1..];
            return (value.to_string(), rest);
        }
    }

    // Single-quoted string
    if let Some(stripped) = value_str.strip_prefix('\'') {
        if let Some(end) = stripped.find('\'') {
            let value = &stripped[..end];
            let rest = &stripped[end + 1..];
            return (value.to_string(), rest);
        }
    }

    // Ruby symbol (:something)
    if let Some(stripped) = value_str.strip_prefix(':') {
        let end = stripped
            .find(|c: char| c == ',' || c.is_whitespace())
            .unwrap_or(stripped.len());
        let value = &stripped[..end];
        let rest = &stripped[end..];
        return (value.to_string(), rest);
    }

    // Number or unquoted value
    let end = value_str
        .find(|c: char| c == ',' || c.is_whitespace())
        .unwrap_or(value_str.len());
    let value = &value_str[..end];
    let rest = &value_str[end..];

    (value.to_string(), rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tap() {
        let brewfile = parse_string(r#"tap "homebrew/cask-fonts""#).unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(brewfile.packages[0].name, "homebrew/cask-fonts");
        assert_eq!(brewfile.packages[0].package_type, PackageType::Tap);
    }

    #[test]
    fn test_parse_brew() {
        let brewfile = parse_string(r#"brew "git""#).unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(brewfile.packages[0].name, "git");
        assert_eq!(brewfile.packages[0].package_type, PackageType::Brew);
    }

    #[test]
    fn test_parse_brew_with_options() {
        let brewfile = parse_string(r#"brew "postgresql@14", restart_service: :changed"#).unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(brewfile.packages[0].name, "postgresql@14");
        assert_eq!(
            brewfile.packages[0].options.get("restart_service"),
            Some(&"changed".to_string())
        );
    }

    #[test]
    fn test_parse_cask() {
        let brewfile = parse_string(r#"cask "visual-studio-code""#).unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(brewfile.packages[0].name, "visual-studio-code");
        assert_eq!(brewfile.packages[0].package_type, PackageType::Cask);
    }

    #[test]
    fn test_parse_mas() {
        let brewfile = parse_string(r#"mas "Xcode", id: 497799835"#).unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(brewfile.packages[0].name, "Xcode");
        assert_eq!(brewfile.packages[0].package_type, PackageType::Mas);
        assert_eq!(brewfile.packages[0].mas_id(), Some("497799835"));
    }

    #[test]
    fn test_parse_vscode() {
        let brewfile = parse_string(r#"vscode "ms-python.python""#).unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(brewfile.packages[0].name, "ms-python.python");
        assert_eq!(brewfile.packages[0].package_type, PackageType::Vscode);
    }

    #[test]
    fn test_parse_with_version_comment() {
        let brewfile = parse_string(r#"brew "git" # 2.40.0"#).unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(brewfile.packages[0].name, "git");
        assert_eq!(brewfile.packages[0].version, Some("2.40.0".to_string()));
    }

    #[test]
    fn test_parse_multiple_packages() {
        let content = r#"
# Taps
tap "homebrew/cask"
tap "homebrew/cask-fonts"

# CLI tools
brew "git" # 2.40.0
brew "curl"

# Applications
cask "firefox"
cask "visual-studio-code"
"#;
        let brewfile = parse_string(content).unwrap();
        assert_eq!(brewfile.packages.len(), 6);
        assert_eq!(brewfile.taps().len(), 2);
        assert_eq!(brewfile.brews().len(), 2);
        assert_eq!(brewfile.casks().len(), 2);
    }

    #[test]
    fn test_parse_skips_comments() {
        let content = r#"
# This is a comment
tap "homebrew/cask"
# Another comment
brew "git"
"#;
        let brewfile = parse_string(content).unwrap();
        assert_eq!(brewfile.packages.len(), 2);
    }

    #[test]
    fn test_parse_skips_empty_lines() {
        let content = r#"
tap "homebrew/cask"

brew "git"

"#;
        let brewfile = parse_string(content).unwrap();
        assert_eq!(brewfile.packages.len(), 2);
    }

    #[test]
    fn test_parse_single_quotes() {
        let brewfile = parse_string(r#"brew 'git'"#).unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(brewfile.packages[0].name, "git");
    }

    #[test]
    fn test_parse_multiple_options() {
        let brewfile = parse_string(r#"brew "nginx", restart_service: :changed, link: :force"#).unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(
            brewfile.packages[0].options.get("restart_service"),
            Some(&"changed".to_string())
        );
        assert_eq!(
            brewfile.packages[0].options.get("link"),
            Some(&"force".to_string())
        );
    }

    #[test]
    fn test_version_comment_with_v_prefix() {
        let brewfile = parse_string(r#"brew "node" # v18.16.0"#).unwrap();
        assert_eq!(brewfile.packages[0].version, Some("v18.16.0".to_string()));
    }

    #[test]
    fn test_regular_comment_not_version() {
        let brewfile = parse_string(r#"brew "git" # best version control"#).unwrap();
        // "best" doesn't start with a digit or 'v', so not a version
        assert_eq!(brewfile.packages[0].version, None);
    }

    #[test]
    fn test_extract_version_comment() {
        let (line, version) = extract_version_comment(r#"brew "git" # 2.40.0"#);
        assert_eq!(line, r#"brew "git""#);
        assert_eq!(version, Some("2.40.0".to_string()));

        let (line, version) = extract_version_comment(r#"brew "git" # this is a comment"#);
        // When comment is not a version, line is returned unchanged
        assert_eq!(line, r#"brew "git" # this is a comment"#);
        assert_eq!(version, None);

        let (line, version) = extract_version_comment(r#"brew "git""#);
        assert_eq!(line, r#"brew "git""#);
        assert_eq!(version, None);
    }
}
