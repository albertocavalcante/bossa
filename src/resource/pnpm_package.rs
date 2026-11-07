//! pnpm global package resource

use anyhow::{Context, Result, bail};

use super::{ApplyContext, ApplyResult, Resource, ResourceState};

/// A pnpm global package
#[derive(Debug, Clone)]
pub struct PnpmPackage {
    pub name: String,
}

impl PnpmPackage {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }

    /// Check if package is installed globally
    fn is_installed(&self) -> Result<bool> {
        validate_name(&self.name)?;
        installed_from_inventory(
            &crate::runner::observe("pnpm", ["list", "--global", "--depth=0", "--json"])?,
            &self.name,
        )
    }

    /// Install the package globally
    fn install(&self, _ctx: &ApplyContext) -> Result<()> {
        let output = crate::runner::capture("pnpm", ["add", "-g", &self.name])
            .context("Failed to run pnpm add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("pnpm add failed: {}", stderr.trim());
        }

        Ok(())
    }
}

fn validate_name(name: &str) -> Result<()> {
    super::native::package_name(name, "pnpm package")?;
    let pieces: Vec<_> = name.strip_prefix('@').unwrap_or(name).split('/').collect();
    if pieces.len() != if name.starts_with('@') { 2 } else { 1 }
        || pieces.iter().any(|piece| {
            piece.is_empty()
                || piece.starts_with(['-', '.', '_'])
                || piece
                    .chars()
                    .any(|value| !value.is_ascii_alphanumeric() && !"._-".contains(value))
        })
    {
        bail!(
            "pnpm globals require package names; version pins, URLs and patterns need an explicit supported configuration"
        );
    }
    Ok(())
}

fn installed_from_inventory(bytes: &[u8], wanted: &str) -> Result<bool> {
    let records: Vec<serde_json::Value> =
        bossa::decode_unique_json(bytes).context("Invalid pnpm global inventory JSON")?;
    if records.len() > 1 {
        bail!("Ambiguous multiple pnpm global roots");
    }
    let mut found = false;
    for record in records {
        let object = record
            .as_object()
            .context("Invalid pnpm global inventory record")?;
        let path = object
            .get("path")
            .and_then(serde_json::Value::as_str)
            .context("pnpm global root observation is unavailable")?;
        if !std::path::Path::new(path).is_absolute() {
            bail!("pnpm global root is not absolute");
        }
        for section in ["dependencies", "devDependencies", "optionalDependencies"] {
            if let Some(dependencies) = object.get(section) {
                let dependencies = dependencies
                    .as_object()
                    .context("Invalid pnpm dependency map")?;
                if dependencies.len() > 100_000 {
                    bail!("pnpm inventory exceeds entry limit");
                }
                if let Some(package) = dependencies.get(wanted) {
                    package
                        .get("version")
                        .and_then(serde_json::Value::as_str)
                        .filter(|value| !value.is_empty())
                        .context("pnpm package has no installed version observation")?;
                    found = true;
                }
            }
        }
    }
    Ok(found)
}

impl Resource for PnpmPackage {
    fn id(&self) -> String {
        format!("pnpm:{}", self.name)
    }

    fn description(&self) -> String {
        format!("Install pnpm global package {}", self.name)
    }

    fn resource_type(&self) -> &'static str {
        "pnpm_package"
    }

    fn current_state(&self) -> Result<ResourceState> {
        if self.is_installed()? {
            Ok(ResourceState::Present { details: None })
        } else {
            Ok(ResourceState::Absent)
        }
    }

    fn desired_state(&self) -> ResourceState {
        ResourceState::Present { details: None }
    }

    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped {
                reason: "Dry run".to_string(),
            });
        }

        if self.is_installed()? {
            return Ok(ApplyResult::NoChange);
        }

        self.install(ctx)?;
        Ok(ApplyResult::Created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_names_and_exact_inventory_observations() {
        for name in ["typescript", "@scope/package"] {
            validate_name(name).unwrap();
        }
        for name in [
            ".",
            "..",
            "./package",
            "file:package",
            "package@1",
            "@scope/package@1",
            "--flag",
        ] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        let bytes =
            br#"[{"path":"/fixture/global","dependencies":{"typescript":{"version":"1"}}}]"#;
        assert!(installed_from_inventory(bytes, "typescript").unwrap());
        assert!(!installed_from_inventory(bytes, "type").unwrap());
        for invalid in [
            b"{}".as_slice(),
            b"[{},{}]",
            b"[{}]",
            br#"[{"path":"/fixture","dependencies":[]}]"#,
            br#"[{"path":"/fixture","dependencies":{"typescript":{}}}]"#,
            br#"[{"path":"/fixture","dependencies":{},"dependencies":{}}]"#,
        ] {
            assert!(installed_from_inventory(invalid, "typescript").is_err());
        }
    }
}
