//! Explicit read compatibility for the former vector-shaped configuration.
//! Conversion never writes a document. Layouts without a lossless mapping fail.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::schema::{self, BossaConfig};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Collection {
    name: String,
    path: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    repositories: Vec<schema::CollectionRepo>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Storage {
    name: String,
    mount_point: String,
    storage_type: schema::StorageType,
    #[serde(default)]
    symlinks: Vec<Symlink>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Symlink {
    source: String,
    target: String,
}

fn insert_unique<T>(map: &mut HashMap<String, T>, name: String, value: T) -> Result<()> {
    if name.trim().is_empty() || map.insert(name.clone(), value).is_some() {
        bail!("Empty or duplicate legacy resource name: {name}");
    }
    Ok(())
}

fn convert(
    mut config: BossaConfig,
    collections: Option<Vec<Collection>>,
    storage: Option<Vec<Storage>>,
    legacy_workspaces: bool,
) -> Result<(BossaConfig, bool)> {
    let legacy = collections.is_some() || storage.is_some() || legacy_workspaces;
    if let Some(collections) = collections {
        for collection in collections {
            let entry = schema::Collection {
                path: collection.path,
                description: collection.description.unwrap_or_default(),
                clone: schema::CloneSettings {
                    depth: 1,
                    single_branch: true,
                    ..Default::default()
                },
                storage: None,
                repos: collection.repositories,
            };
            insert_unique(&mut config.collections, collection.name, entry)?;
        }
    }
    if let Some(storage) = storage {
        for storage in storage {
            let entry = schema::Storage {
                mount: storage.mount_point,
                storage_type: storage.storage_type,
                description: String::new(),
                symlinks: storage
                    .symlinks
                    .into_iter()
                    .map(|link| schema::Symlink {
                        from: link.target,
                        to: link.source,
                    })
                    .collect(),
            };
            insert_unique(&mut config.storage, storage.name, entry)?;
        }
    }
    config.validate()?;
    Ok((config, legacy))
}

fn reject_workspace_layout(count: usize) -> Result<()> {
    if count != 0 {
        bail!(
            "Legacy workspace paths cannot be represented losslessly by the canonical layout; review and explicitly convert the workspace configuration before applying"
        );
    }
    Ok(())
}

/// Normalize only recognized legacy fields, keeping native JSON value semantics.
pub fn decode(mut value: Value) -> Result<(BossaConfig, bool)> {
    let object = value
        .as_object_mut()
        .context("Bossa configuration must be a table/object")?;
    let collections = if object.get("collections").is_some_and(Value::is_array) {
        Some(
            serde_json::from_value(object.remove("collections").unwrap())
                .context("Unsupported legacy collections shape")?,
        )
    } else {
        None
    };
    let storage = if object.get("storage").is_some_and(Value::is_array) {
        Some(
            serde_json::from_value(object.remove("storage").unwrap())
                .context("Unsupported legacy storage shape")?,
        )
    } else {
        None
    };
    let legacy_workspaces = object.get("workspaces").is_some_and(Value::is_array);
    if legacy_workspaces {
        reject_workspace_layout(
            object
                .remove("workspaces")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
        )?;
    }
    let config: BossaConfig =
        serde_json::from_value(value).context("Invalid canonical Bossa configuration")?;
    convert(config, collections, storage, legacy_workspaces)
}

/// TOML stays in its native value model: floats and datetimes never pass through
/// JSON, which cannot represent all TOML values losslessly.
pub fn decode_toml(mut value: toml::Value) -> Result<(BossaConfig, bool)> {
    let object = value
        .as_table_mut()
        .context("Bossa configuration must be a table/object")?;
    let collections = if object.get("collections").is_some_and(toml::Value::is_array) {
        Some(
            object
                .remove("collections")
                .unwrap()
                .try_into()
                .context("Unsupported legacy collections shape")?,
        )
    } else {
        None
    };
    let storage = if object.get("storage").is_some_and(toml::Value::is_array) {
        Some(
            object
                .remove("storage")
                .unwrap()
                .try_into()
                .context("Unsupported legacy storage shape")?,
        )
    } else {
        None
    };
    let legacy_workspaces = object.get("workspaces").is_some_and(toml::Value::is_array);
    if legacy_workspaces {
        reject_workspace_layout(
            object
                .remove("workspaces")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
        )?;
    }
    // Value's owned deserializer exposes TOML datetimes as strings. Reparse
    // native TOML text so typed maps retain datetime and non-finite float values.
    let canonical = toml::to_string(&value).context("Could not normalize TOML configuration")?;
    let config: BossaConfig =
        toml::from_str(&canonical).context("Invalid canonical Bossa configuration")?;
    convert(config, collections, storage, legacy_workspaces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compatible_legacy_resources_preserve_unrelated_canonical_sections() {
        let (config, legacy) = decode(json!({
            "collections": [{"name":"refs","path":"~/refs","repositories":[{"name":"repo","url":"local-source"}]}],
            "workspaces": [],
            "storage": [{"name":"external","mount_point":"/fixture/mount","storage_type":"external","symlinks":[{"source":"/fixture/content","target":"~/link"}]}],
            "locations": {"paths":{"dev":"/fixture/dev"}}
        })).unwrap();
        assert!(legacy);
        assert_eq!(config.collections["refs"].repos[0].name, "repo");
        assert_eq!(config.collections["refs"].clone.depth, 1);
        assert_eq!(config.storage["external"].symlinks[0].from, "~/link");
        assert_eq!(
            config.storage["external"].symlinks[0].to,
            "/fixture/content"
        );
        assert_eq!(config.locations.paths["dev"], "/fixture/dev");
    }

    #[test]
    fn ambiguous_legacy_names_and_workspace_layouts_fail_explicitly() {
        assert!(
            decode(json!({"collections":[{"name":"same","path":"a"},{"name":"same","path":"b"}]}))
                .is_err()
        );
        let error = decode(json!({"workspaces":[{"name":"repo","url":"source","worktrees":[{"branch":"main","path":"/custom/path"}]}]})).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot be represented losslessly")
        );
    }

    #[test]
    fn native_toml_values_survive_legacy_section_conversion() {
        let input = r#"
collections = [{ name = "refs", path = "~/refs" }]
[defaults]
"com.example.Float" = nan
[configs.git.core]
when = 1979-05-27T07:32:00Z
"#;
        let (config, legacy) = decode_toml(toml::from_str(input).unwrap()).unwrap();
        assert!(legacy);
        assert!(
            matches!(config.defaults.settings["com.example.Float"], schema::DefaultValue::Float(value) if value.is_nan())
        );
        let when = &config.configs.git.unwrap().core["when"];
        assert!(when.as_datetime().is_some(), "{when:?}");
    }

    #[test]
    fn unknown_canonical_and_legacy_fields_are_not_ignored() {
        assert!(decode(json!({"packges": {}})).is_err());
        assert!(
            decode(json!({"collections":[{"name":"repo","path":"a","clone_typo":true}]})).is_err()
        );
        assert!(decode(json!({"collections":{"refs":{"path":"a","repoz":[]}}})).is_err());
    }
}
