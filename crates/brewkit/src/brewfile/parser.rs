//! Strict line-oriented subset of the Homebrew Brewfile DSL.
//!
//! Ruby expressions, arrays, hashes and other directives are rejected rather
//! than silently omitted. Full bundle application delegates to Homebrew.

use crate::error::{Error, Result};
use crate::types::{Brewfile, OptionValue, Package, PackageType, ScalarNumber, ScalarOption};
use std::io::Read;
use std::path::Path;

const MAX_BREWFILE_BYTES: usize = 8 * 1024 * 1024;

/// Parse a Brewfile from a file path.
pub fn parse_file(path: &Path) -> Result<Brewfile> {
    let mut content = String::new();
    std::fs::File::open(path)?
        .take((MAX_BREWFILE_BYTES + 1) as u64)
        .read_to_string(&mut content)?;
    let mut result = parse_string(&content)?;
    result.path = Some(path.to_path_buf());
    Ok(result)
}

fn error(line: usize, message: &str) -> Error {
    Error::BrewfileParse {
        line,
        message: message.to_owned(),
    }
}

/// Parse every line in the supported subset, rejecting unsupported syntax.
pub fn parse_string(content: &str) -> Result<Brewfile> {
    if content.len() > MAX_BREWFILE_BYTES {
        return Err(error(1, "Brewfile exceeds 8 MiB limit"));
    }
    let mut result = Brewfile::new();
    for (index, line) in content.lines().enumerate() {
        let number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (directive, rest) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| error(number, "missing directive argument"))?;
        // Ruby method names are case-sensitive.
        let kind = match directive {
            "tap" => PackageType::Tap,
            "brew" => PackageType::Brew,
            "cask" => PackageType::Cask,
            "mas" => PackageType::Mas,
            "vscode" => PackageType::Vscode,
            _ => return Err(error(number, "unsupported directive")),
        };
        let (name, mut rest) = quoted(rest.trim_start(), number)?;
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(error(number, "invalid package name"));
        }
        let mut package = Package::new(name, kind);
        loop {
            rest = rest.trim_start();
            if rest.is_empty() {
                break;
            }
            if let Some(comment) = rest.strip_prefix('#') {
                let comment = comment.trim();
                // A single version-like token is generated observation metadata.
                if !comment.is_empty()
                    && !comment.chars().any(char::is_whitespace)
                    && (comment.starts_with(|c: char| c.is_ascii_digit())
                        || comment
                            .strip_prefix('v')
                            .is_some_and(|s| s.starts_with(|c: char| c.is_ascii_digit())))
                {
                    package.version = Some(comment.to_owned());
                }
                break;
            }
            rest = rest
                .strip_prefix(',')
                .ok_or_else(|| error(number, "expected comma or comment"))?
                .trim_start();
            let (key, after_key) = if rest.starts_with('\'') || rest.starts_with('"') {
                quoted(rest, number)?
            } else {
                let key_len = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                let key = &rest[..key_len];
                if !valid_identifier(key) {
                    return Err(error(number, "invalid option key"));
                }
                (key.to_owned(), &rest[key_len..])
            };
            rest = after_key
                .strip_prefix(':')
                .ok_or_else(|| error(number, "expected option colon"))?
                .trim_start();
            let (value, remaining) = scalar(rest, number)?;
            if package
                .options
                .insert(key, OptionValue::Typed(value))
                .is_some()
            {
                return Err(error(number, "duplicate option"));
            }
            rest = remaining;
        }
        if kind == PackageType::Mas {
            match package.options.get("id") {
                Some(OptionValue::Typed(ScalarOption::Number(id)))
                    if id.as_str().bytes().all(|b| b.is_ascii_digit()) => {}
                _ => return Err(error(number, "mas requires a numeric id")),
            }
        }
        result.add(package);
    }
    Ok(result)
}

pub(crate) fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn quoted(input: &str, line: usize) -> Result<(String, &str)> {
    let quote = input
        .chars()
        .next()
        .filter(|c| *c == '\'' || *c == '"')
        .ok_or_else(|| error(line, "expected quoted string"))?;
    let mut value = String::new();
    let mut chars = input[1..].char_indices();
    while let Some((offset, c)) = chars.next() {
        if c == quote {
            return Ok((value, &input[offset + 2..]));
        }
        if quote == '"' && c == '#' && input[offset + 2..].starts_with(['{', '@', '$']) {
            return Err(error(line, "Ruby interpolation is unsupported"));
        }
        if c == '\\' {
            let (_, escaped) = chars
                .next()
                .ok_or_else(|| error(line, "unfinished escape"))?;
            if escaped == 'u' && quote == '"' {
                let mut code = 0u32;
                for _ in 0..4 {
                    let (_, digit) = chars
                        .next()
                        .ok_or_else(|| error(line, "unfinished Unicode escape"))?;
                    code = code * 16
                        + digit
                            .to_digit(16)
                            .ok_or_else(|| error(line, "invalid Unicode escape"))?;
                }
                let decoded =
                    char::from_u32(code).ok_or_else(|| error(line, "invalid Unicode scalar"))?;
                value.push(decoded);
                continue;
            }
            let decoded = match escaped {
                '\\' => '\\',
                '\'' if quote == '\'' => '\'',
                '"' if quote == '"' => '"',
                '#' if quote == '"' => '#',
                'n' if quote == '"' => '\n',
                'r' if quote == '"' => '\r',
                't' if quote == '"' => '\t',
                _ => return Err(error(line, "unsupported string escape")),
            };
            value.push(decoded);
        } else if c.is_control() && c != '\t' {
            return Err(error(line, "unsupported control character"));
        } else {
            value.push(c);
        }
    }
    Err(error(line, "unclosed quote"))
}

fn scalar(input: &str, line: usize) -> Result<(ScalarOption, &str)> {
    if input.starts_with('\'') || input.starts_with('"') {
        let (value, rest) = quoted(input, line)?;
        return Ok((ScalarOption::String(value), rest));
    }
    if let Some(symbol) = input.strip_prefix(':')
        && (symbol.starts_with('\'') || symbol.starts_with('"'))
    {
        let (value, rest) = quoted(symbol, line)?;
        return Ok((ScalarOption::Symbol(value), rest));
    }
    let end = input
        .find(|c: char| c == ',' || c == '#' || c.is_whitespace())
        .unwrap_or(input.len());
    let (token, rest) = input.split_at(end);
    let value = match token {
        "true" => ScalarOption::Bool(true),
        "false" => ScalarOption::Bool(false),
        "nil" => ScalarOption::Nil,
        _ if token.starts_with(':') && valid_identifier(&token[1..]) => {
            ScalarOption::Symbol(token[1..].to_owned())
        }
        _ => ScalarOption::Number(
            ScalarNumber::try_from(token.to_owned())
                .map_err(|_| error(line, "unsupported option value"))?,
        ),
    };
    Ok((value, rest))
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
            Some(&OptionValue::Typed(ScalarOption::Symbol(
                "changed".to_string()
            )))
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
        let brewfile = parse_string(r"brew 'git'").unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(brewfile.packages[0].name, "git");
    }

    #[test]
    fn test_parse_multiple_options() {
        let brewfile =
            parse_string(r#"brew "nginx", restart_service: :changed, link: :force"#).unwrap();
        assert_eq!(brewfile.packages.len(), 1);
        assert_eq!(
            brewfile.packages[0].options.get("restart_service"),
            Some(&OptionValue::Typed(ScalarOption::Symbol(
                "changed".to_string()
            )))
        );
        assert_eq!(
            brewfile.packages[0].options.get("link"),
            Some(&OptionValue::Typed(ScalarOption::Symbol(
                "force".to_string()
            )))
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
    fn rejects_every_unsupported_line_with_its_number() {
        for bad in [
            "include \"other\"",
            "brew",
            "brew \"git\",",
            "brew \"git\", x: false, x: true",
            "brew \"git\" if OS.mac?",
            "brew \"git\", args: [1]",
            "brew \"unterminated",
        ] {
            let input = format!("# comment\n{bad}\n");
            assert!(
                matches!(
                    parse_string(&input),
                    Err(Error::BrewfileParse { line: 2, .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn scalar_types_and_quoted_comments_survive_rewrite() {
        let input = r#"brew "git#tool", a: "false", b: false, c: :false, d: nil, e: 12, f: "a,b#c" # 2.40.0"#;
        let parsed = parse_string(input).unwrap();
        assert_eq!(parsed.packages[0].version.as_deref(), Some("2.40.0"));
        let rewritten = crate::brewfile::write_string(
            &parsed,
            &crate::brewfile::WriteOptions {
                include_versions: true,
                ..Default::default()
            },
        );
        assert_eq!(parse_string(&rewritten).unwrap().packages, parsed.packages);
    }

    #[test]
    fn legacy_json_options_remain_legacy() {
        let value: OptionValue = serde_json::from_str(r#""changed""#).unwrap();
        assert_eq!(value, OptionValue::Legacy("changed".to_owned()));
        assert_eq!(serde_json::to_string(&value).unwrap(), r#""changed""#);
    }
    #[test]
    fn input_limit_and_large_comment() {
        let valid = format!("#{}\nbrew \"git\"", "x".repeat(1024 * 1024));
        assert_eq!(parse_string(&valid).unwrap().packages.len(), 1);
        assert!(parse_string(&"#".repeat(MAX_BREWFILE_BYTES + 1)).is_err());
    }

    #[test]
    fn rejects_interpolation_and_unsafe_number_deserialization() {
        for bad in [
            r##"brew "#@name""##,
            r##"brew "#{name}""##,
            r##"brew "#$name""##,
        ] {
            assert!(parse_string(bad).is_err());
        }
        assert!(ScalarNumber::try_from("1\n".to_owned()).is_err());
    }
    #[test]
    fn rejects_controls_that_writer_cannot_round_trip() {
        for c in ['\u{1}', '\u{7f}'] {
            assert!(parse_string(&format!("brew \"git\", x: \"{c}\"")).is_err());
        }
        let parsed = parse_string("brew \"git\", x: \"a\tb\"").unwrap();
        let written = crate::brewfile::write_string(&parsed, &Default::default());
        assert_eq!(parse_string(&written).unwrap().packages, parsed.packages);
    }
    #[test]
    fn constructed_control_string_round_trips() {
        let mut file = Brewfile::new();
        file.add(
            Package::brew("git")
                .with_typed_option("x", ScalarOption::String("a\u{1}\u{7f}b".to_owned())),
        );
        let written = crate::brewfile::write_string(&file, &Default::default());
        assert_eq!(parse_string(&written).unwrap().packages, file.packages);
        assert!(parse_string(r#"brew "git", x: "\uD800""#).is_err());
    }
}
