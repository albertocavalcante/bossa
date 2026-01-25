//! Drift detection for Homebrew packages.
//!
//! Compares installed packages against a Brewfile to detect:
//! - Packages installed but not in Brewfile (untracked)
//! - Packages in Brewfile but not installed (missing)
//! - Packages with version mismatches

use crate::backend::Backend;
use crate::error::Result;
use crate::types::{AuditResult, Brewfile, Package, PackageType};
use std::collections::HashSet;

/// Audit installed packages against a Brewfile.
///
/// Returns drift information including untracked, missing, and mismatched packages.
pub fn audit(backend: &dyn Backend, brewfile: &Brewfile) -> Result<AuditResult> {
    let mut result = AuditResult::default();

    // Audit each package type
    audit_type(backend, brewfile, PackageType::Tap, &mut result)?;
    audit_type(backend, brewfile, PackageType::Brew, &mut result)?;
    audit_type(backend, brewfile, PackageType::Cask, &mut result)?;

    // Optionally audit mas and vscode if there are entries
    if !brewfile.mas_apps().is_empty() {
        audit_type(backend, brewfile, PackageType::Mas, &mut result)?;
    }
    if !brewfile.vscode_extensions().is_empty() {
        audit_type(backend, brewfile, PackageType::Vscode, &mut result)?;
    }

    Ok(result)
}

/// Audit a single package type.
fn audit_type(
    backend: &dyn Backend,
    brewfile: &Brewfile,
    package_type: PackageType,
    result: &mut AuditResult,
) -> Result<()> {
    // Get installed packages of this type
    let installed = backend.list_installed(package_type)?;
    let installed_names: HashSet<String> = installed.iter().map(|p| p.name.to_lowercase()).collect();

    // Get Brewfile packages of this type
    let declared: Vec<&Package> = brewfile.packages_of_type(package_type);
    let declared_names: HashSet<String> = declared.iter().map(|p| p.name.to_lowercase()).collect();

    // Find untracked (installed but not declared)
    for pkg in &installed {
        // For brew formulas, only report packages installed on request
        if package_type == PackageType::Brew && !pkg.installed_on_request {
            continue;
        }

        if !declared_names.contains(&pkg.name.to_lowercase()) {
            result.untracked.push(pkg.clone());
        }
    }

    // Find missing (declared but not installed)
    for pkg in &declared {
        if !installed_names.contains(&pkg.name.to_lowercase()) {
            result.missing.push((*pkg).clone());
        }
    }

    // Find version mismatches
    for pkg in &declared {
        if let Some(declared_version) = &pkg.version
            && let Some(installed_pkg) = installed
                .iter()
                .find(|i| i.name.to_lowercase() == pkg.name.to_lowercase())
            && !versions_match(declared_version, &installed_pkg.version)
        {
            result.mismatched.push(((*pkg).clone(), installed_pkg.clone()));
        }
    }

    Ok(())
}

/// Check if two versions match (allowing for some flexibility).
fn versions_match(declared: &str, installed: &str) -> bool {
    let declared = declared.trim().to_lowercase();
    let installed = installed.trim().to_lowercase();

    // Exact match
    if declared == installed {
        return true;
    }

    // Strip 'v' prefix if present
    let declared = declared.strip_prefix('v').unwrap_or(&declared);
    let installed = installed.strip_prefix('v').unwrap_or(&installed);

    if declared == installed {
        return true;
    }

    // Allow prefix match (e.g., "2.40" matches "2.40.1")
    if installed.starts_with(declared) && installed.chars().nth(declared.len()) == Some('.') {
        return true;
    }

    false
}

/// Audit options.
#[derive(Debug, Clone, Default)]
pub struct AuditOptions {
    /// Include packages not installed on request (dependencies)
    pub include_dependencies: bool,
    /// Package types to audit (empty means all)
    pub package_types: Vec<PackageType>,
}

/// Audit with options.
pub fn audit_with_options(
    backend: &dyn Backend,
    brewfile: &Brewfile,
    options: &AuditOptions,
) -> Result<AuditResult> {
    let mut result = AuditResult::default();

    let types_to_audit = if options.package_types.is_empty() {
        vec![
            PackageType::Tap,
            PackageType::Brew,
            PackageType::Cask,
            PackageType::Mas,
            PackageType::Vscode,
        ]
    } else {
        options.package_types.clone()
    };

    for package_type in types_to_audit {
        audit_type_with_options(backend, brewfile, package_type, options, &mut result)?;
    }

    Ok(result)
}

fn audit_type_with_options(
    backend: &dyn Backend,
    brewfile: &Brewfile,
    package_type: PackageType,
    options: &AuditOptions,
    result: &mut AuditResult,
) -> Result<()> {
    let installed = backend.list_installed(package_type)?;
    let installed_names: HashSet<String> = installed.iter().map(|p| p.name.to_lowercase()).collect();

    let declared: Vec<&Package> = brewfile.packages_of_type(package_type);
    let declared_names: HashSet<String> = declared.iter().map(|p| p.name.to_lowercase()).collect();

    // Find untracked
    for pkg in &installed {
        // Filter dependencies unless included
        if package_type == PackageType::Brew && !pkg.installed_on_request && !options.include_dependencies {
            continue;
        }

        if !declared_names.contains(&pkg.name.to_lowercase()) {
            result.untracked.push(pkg.clone());
        }
    }

    // Find missing
    for pkg in &declared {
        if !installed_names.contains(&pkg.name.to_lowercase()) {
            result.missing.push((*pkg).clone());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_versions_match_exact() {
        assert!(versions_match("2.40.0", "2.40.0"));
        assert!(versions_match("1.0", "1.0"));
    }

    #[test]
    fn test_versions_match_v_prefix() {
        assert!(versions_match("v2.40.0", "2.40.0"));
        assert!(versions_match("2.40.0", "v2.40.0"));
        assert!(versions_match("v1.0", "v1.0"));
    }

    #[test]
    fn test_versions_match_prefix() {
        assert!(versions_match("2.40", "2.40.1"));
        assert!(versions_match("2", "2.40.1"));
        assert!(!versions_match("2.4", "2.40.1")); // 2.4 != 2.40
    }

    #[test]
    fn test_versions_match_case_insensitive() {
        assert!(versions_match("V2.40.0", "v2.40.0"));
    }

    #[test]
    fn test_versions_no_match() {
        assert!(!versions_match("2.40.0", "2.41.0"));
        assert!(!versions_match("1.0", "2.0"));
    }
}
