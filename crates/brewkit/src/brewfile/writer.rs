//! Writer for generating Brewfile content.
//!
//! Generates properly formatted Brewfile with optional version comments.

use crate::types::{Brewfile, Package, PackageType};
use std::fmt::Write;
use std::path::Path;

/// Options for writing a Brewfile.
#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    /// Include version comments
    pub include_versions: bool,
    /// Group packages by type with section headers
    pub group_by_type: bool,
    /// Sort packages alphabetically within groups
    pub sort_packages: bool,
}

/// Write a Brewfile to a file.
pub fn write_file(brewfile: &Brewfile, path: &Path, options: &WriteOptions) -> std::io::Result<()> {
    let content = write_string(brewfile, options);
    std::fs::write(path, content)
}

/// Write a Brewfile to a string.
pub fn write_string(brewfile: &Brewfile, options: &WriteOptions) -> String {
    let mut output = String::new();

    if options.group_by_type {
        write_grouped(&mut output, brewfile, options);
    } else {
        write_flat(&mut output, brewfile, options);
    }

    output
}

/// Write all packages without grouping.
fn write_flat(output: &mut String, brewfile: &Brewfile, options: &WriteOptions) {
    let packages = if options.sort_packages {
        let mut sorted = brewfile.packages.clone();
        sorted.sort_by(|a, b| {
            a.package_type
                .directive()
                .cmp(b.package_type.directive())
                .then_with(|| a.name.cmp(&b.name))
        });
        sorted
    } else {
        brewfile.packages.clone()
    };

    for package in &packages {
        write_package(output, package, options);
    }
}

/// Write packages grouped by type with headers.
fn write_grouped(output: &mut String, brewfile: &Brewfile, options: &WriteOptions) {
    let sections = [
        (PackageType::Tap, "Taps"),
        (PackageType::Brew, "Formulae"),
        (PackageType::Cask, "Casks"),
        (PackageType::Mas, "Mac App Store"),
        (PackageType::Vscode, "VS Code Extensions"),
    ];

    let mut first_section = true;

    for (package_type, header) in sections {
        let mut packages: Vec<_> = brewfile.packages_of_type(package_type);

        if packages.is_empty() {
            continue;
        }

        if options.sort_packages {
            packages.sort_by_key(|p| &p.name);
        }

        // Add blank line between sections
        if !first_section {
            writeln!(output).unwrap();
        }
        first_section = false;

        // Write section header
        writeln!(output, "# {header}").unwrap();

        for package in packages {
            write_package(output, package, options);
        }
    }
}

/// Write a single package entry.
fn write_package(output: &mut String, package: &Package, options: &WriteOptions) {
    let directive = package.package_type.directive();

    // Start with directive and name
    write!(output, "{} \"{}\"", directive, package.name).unwrap();

    // Add options
    let mut options_written = false;
    for (key, value) in &package.options {
        if options_written {
            write!(output, ", ").unwrap();
        } else {
            write!(output, ", ").unwrap();
            options_written = true;
        }

        // Determine if value should be a symbol or string
        if is_symbol_value(value) {
            write!(output, "{key}: :{value}").unwrap();
        } else if value.chars().all(|c| c.is_ascii_digit()) {
            // Numeric value
            write!(output, "{key}: {value}").unwrap();
        } else {
            // String value
            write!(output, "{key}: \"{value}\"").unwrap();
        }
    }

    // Add version comment if present and requested
    if options.include_versions
        && let Some(version) = &package.version
    {
        write!(output, " # {version}").unwrap();
    }

    writeln!(output).unwrap();
}

/// Check if a value should be written as a Ruby symbol.
fn is_symbol_value(value: &str) -> bool {
    // Common Ruby symbols used in Brewfiles
    matches!(
        value,
        "changed" | "always" | "force" | "true" | "false" | "nil" | "yes" | "no"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_simple_tap() {
        let mut brewfile = Brewfile::new();
        brewfile.add(Package::tap("homebrew/cask"));

        let output = write_string(&brewfile, &WriteOptions::default());
        assert_eq!(output, "tap \"homebrew/cask\"\n");
    }

    #[test]
    fn test_write_brew_with_version() {
        let mut brewfile = Brewfile::new();
        brewfile.add(Package::brew("git").with_version("2.40.0"));

        let options = WriteOptions {
            include_versions: true,
            ..Default::default()
        };
        let output = write_string(&brewfile, &options);
        assert_eq!(output, "brew \"git\" # 2.40.0\n");
    }

    #[test]
    fn test_write_brew_with_options() {
        let mut brewfile = Brewfile::new();
        brewfile.add(Package::brew("postgresql@14").with_option("restart_service", "changed"));

        let output = write_string(&brewfile, &WriteOptions::default());
        assert_eq!(
            output,
            "brew \"postgresql@14\", restart_service: :changed\n"
        );
    }

    #[test]
    fn test_write_mas() {
        let mut brewfile = Brewfile::new();
        brewfile.add(Package::mas("Xcode", "497799835"));

        let output = write_string(&brewfile, &WriteOptions::default());
        assert_eq!(output, "mas \"Xcode\", id: 497799835\n");
    }

    #[test]
    fn test_write_grouped() {
        let mut brewfile = Brewfile::new();
        brewfile.add(Package::tap("homebrew/cask"));
        brewfile.add(Package::brew("git"));
        brewfile.add(Package::cask("firefox"));

        let options = WriteOptions {
            group_by_type: true,
            ..Default::default()
        };
        let output = write_string(&brewfile, &options);

        assert!(output.contains("# Taps"));
        assert!(output.contains("# Formulae"));
        assert!(output.contains("# Casks"));
    }

    #[test]
    fn test_write_sorted() {
        let mut brewfile = Brewfile::new();
        brewfile.add(Package::brew("zsh"));
        brewfile.add(Package::brew("bash"));
        brewfile.add(Package::brew("git"));

        let options = WriteOptions {
            sort_packages: true,
            ..Default::default()
        };
        let output = write_string(&brewfile, &options);

        let lines: Vec<_> = output.lines().collect();
        assert!(lines[0].contains("bash"));
        assert!(lines[1].contains("git"));
        assert!(lines[2].contains("zsh"));
    }

    #[test]
    fn test_write_grouped_and_sorted() {
        let mut brewfile = Brewfile::new();
        brewfile.add(Package::cask("firefox"));
        brewfile.add(Package::brew("zsh"));
        brewfile.add(Package::tap("homebrew/cask"));
        brewfile.add(Package::brew("bash"));

        let options = WriteOptions {
            group_by_type: true,
            sort_packages: true,
            ..Default::default()
        };
        let output = write_string(&brewfile, &options);

        // Should be: Taps, Formulae (sorted), Casks
        assert!(output.find("# Taps").unwrap() < output.find("# Formulae").unwrap());
        assert!(output.find("# Formulae").unwrap() < output.find("# Casks").unwrap());

        // Brews should be sorted
        assert!(output.find("bash").unwrap() < output.find("zsh").unwrap());
    }

    #[test]
    fn test_write_vscode() {
        let mut brewfile = Brewfile::new();
        brewfile.add(Package::vscode("ms-python.python"));

        let output = write_string(&brewfile, &WriteOptions::default());
        assert_eq!(output, "vscode \"ms-python.python\"\n");
    }
}
