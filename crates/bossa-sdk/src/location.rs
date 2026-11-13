use crate::{AppEnvironment, schema::LocationsConfig};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

/// Expand known location references with explicit environment inputs.
/// Unknown and malformed references remain literal; cycles are errors.
pub fn resolve_location_path(
    path: &str,
    locations: &LocationsConfig,
    environment: &AppEnvironment,
) -> Result<PathBuf> {
    fn resolve(
        path: &str,
        locations: &LocationsConfig,
        environment: &AppEnvironment,
        active: &mut Vec<String>,
    ) -> Result<PathBuf> {
        let mut result = String::new();
        let mut rest = path;
        while let Some(start) = rest.find("${locations.") {
            result.push_str(&rest[..start]);
            let candidate = &rest[start..];
            let Some(end) = candidate.find('}') else {
                result.push_str(candidate);
                rest = "";
                break;
            };
            let name = &candidate[12..end];
            if let Some(value) = locations.paths.get(name) {
                if active.iter().any(|item| item == name) {
                    bail!("Cyclic location reference: {name}");
                }
                active.push(name.to_owned());
                let expanded = resolve(value, locations, environment, active)?;
                active.pop();
                result.push_str(
                    expanded
                        .to_str()
                        .context("Location expansion is not UTF-8")?,
                );
            } else {
                result.push_str(&candidate[..=end]);
            }
            rest = &candidate[end + 1..];
        }
        result.push_str(rest);
        if !active.is_empty() {
            // Probe the same expansion grammar while retaining unknown variables.
            // A non-UTF-8 OS value cannot be embedded in a string location.
            let non_utf8 = std::cell::Cell::new(false);
            let _ = shellexpand::full_with_context(
                &result,
                || {
                    environment.home_dir().ok().and_then(|home| {
                        let text = home.to_str();
                        non_utf8.set(non_utf8.get() || text.is_none());
                        text
                    })
                },
                |name| {
                    let value = environment.variable(name);
                    let text = value.and_then(std::ffi::OsStr::to_str);
                    non_utf8.set(non_utf8.get() || (value.is_some() && text.is_none()));
                    Ok::<_, ()>(text)
                },
            );
            if non_utf8.get() {
                bail!("Location expansion is not UTF-8");
            }
        }
        Ok(environment.expand(&result))
    }
    resolve(path, locations, environment, &mut Vec::new())
}
