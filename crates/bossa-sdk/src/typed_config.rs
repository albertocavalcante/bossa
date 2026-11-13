//! Typed configuration decoding and source-bound persistence.
use crate::{ConfigFormat, DocumentStore, StoreRevision, StoreSnapshot, schema::BossaConfig};
use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug)]
pub struct DecodedBossaConfig {
    pub desired: BossaConfig,
    pub legacy: bool,
}

/// Decode explicit bytes without filesystem or environment access.
pub fn decode_bossa_config(format: ConfigFormat, bytes: &[u8]) -> Result<DecodedBossaConfig> {
    let (desired, legacy) = match format {
        ConfigFormat::Json => crate::legacy_config::decode(crate::decode_unique_json(bytes)?)?,
        ConfigFormat::Toml => {
            let contents = std::str::from_utf8(bytes).context("Configuration is not UTF-8")?;
            crate::legacy_config::decode_toml(toml::from_str(contents)?)?
        }
    };
    Ok(DecodedBossaConfig { desired, legacy })
}

fn serialize(format: ConfigFormat, desired: &BossaConfig) -> Result<String> {
    Ok(match format {
        ConfigFormat::Json => serde_json::to_string_pretty(desired)?,
        ConfigFormat::Toml => toml::to_string_pretty(desired)?,
    })
}

fn source(store: &DocumentStore, snapshot: &StoreSnapshot) -> Result<(PathBuf, ConfigFormat)> {
    match &snapshot.document {
        Some(document) => {
            if decode_bossa_config(document.format, document.contents().as_bytes())?.legacy {
                anyhow::bail!(
                    "Legacy configuration is read-compatible; explicitly convert it before editing"
                );
            }
            Ok((document.path.clone(), document.format))
        }
        None => Ok((store.default_path(), ConfigFormat::Toml)),
    }
}

#[derive(Debug)]
pub struct LoadedBossaConfig {
    desired: BossaConfig,
    store: DocumentStore,
    revision: StoreRevision,
    selected: Option<(PathBuf, ConfigFormat)>,
    legacy: bool,
}
impl LoadedBossaConfig {
    pub fn load(store: DocumentStore, required: bool) -> Result<Self> {
        let snapshot = store.read()?;
        let (desired, legacy) = match &snapshot.document {
            Some(document) => {
                let decoded = decode_bossa_config(document.format, document.contents().as_bytes())
                    .with_context(|| format!("Could not load {}", document.path.display()))?;
                (decoded.desired, decoded.legacy)
            }
            None if required => anyhow::bail!(
                "No Bossa configuration found; select or create config.toml/config.json before applying"
            ),
            None => (BossaConfig::default(), false),
        };
        let selected = snapshot
            .document
            .as_ref()
            .map(|d| (d.path.clone(), d.format));
        Ok(Self {
            desired,
            store,
            revision: snapshot.revision,
            selected,
            legacy,
        })
    }
    pub fn desired(&self) -> &BossaConfig {
        &self.desired
    }
    pub fn desired_mut(&mut self) -> &mut BossaConfig {
        &mut self.desired
    }
    pub fn replace_desired(&mut self, desired: BossaConfig) {
        self.desired = desired;
    }
    pub fn store(&self) -> &DocumentStore {
        &self.store
    }
    pub fn selected(&self) -> Option<(&std::path::Path, ConfigFormat)> {
        self.selected.as_ref().map(|(p, f)| (p.as_path(), *f))
    }
    pub fn has_document(&self) -> bool {
        self.selected.is_some()
    }
    pub fn is_legacy(&self) -> bool {
        self.legacy
    }
    pub fn save(&mut self) -> Result<PathBuf> {
        self.desired.validate()?;
        let guard = self.store.lock()?;
        let current = guard.read()?;
        if self.revision != current.revision {
            return Err(crate::StoreError::Conflict.into());
        }
        let (path, format) = source(&self.store, &current)?;
        let contents = serialize(format, &self.desired)?;
        self.revision = guard.commit(&current.revision, &contents)?;
        self.selected = Some((path.clone(), format));
        Ok(path)
    }
}

/// Read and mutate under the same store guard; do not rewrite an unchanged value.
pub fn update_bossa_config<T>(
    store: &DocumentStore,
    mutate: impl FnOnce(&mut BossaConfig) -> Result<T>,
) -> Result<T> {
    let guard = store.lock()?;
    let snapshot = guard.read()?;
    let (path, format) = source(store, &snapshot)?;
    let mut desired = match &snapshot.document {
        Some(document) => {
            decode_bossa_config(document.format, document.contents().as_bytes())?.desired
        }
        None => BossaConfig::default(),
    };
    let before = serialize(format, &desired)?;
    let result = mutate(&mut desired)?;
    desired.validate()?;
    let after = serialize(format, &desired)?;
    if before != after {
        guard
            .commit(&snapshot.revision, &after)
            .with_context(|| format!("Could not update configuration {}", path.display()))?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppEnvironment, resolve_location_path};

    #[test]
    fn decode_is_strict_and_preserves_legacy_classification() {
        assert!(
            decode_bossa_config(
                ConfigFormat::Json,
                br#"{"locations":{"paths":{"x":"a","x":"b"}}}"#
            )
            .is_err()
        );
        assert!(
            decode_bossa_config(ConfigFormat::Toml, b"[locations.paths]\nx='a'\nx='b'").is_err()
        );
        assert!(decode_bossa_config(ConfigFormat::Json, br#"{"unknown":true}"#).is_err());
        assert!(decode_bossa_config(ConfigFormat::Json, br#"{"workspaces":[{}]}"#).is_err());
        let legacy = decode_bossa_config(ConfigFormat::Json, br#"{"collections":[]}"#).unwrap();
        assert!(legacy.legacy);
    }

    #[cfg(unix)]
    #[test]
    fn loaded_revision_prevents_stale_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        std::fs::write(&path, "{}").unwrap();
        let store = DocumentStore::config(temp.path(), "config").unwrap();
        let mut first = LoadedBossaConfig::load(store.clone(), true).unwrap();
        let mut stale = LoadedBossaConfig::load(store, true).unwrap();
        first
            .desired_mut()
            .locations
            .paths
            .insert("one".into(), "/one".into());
        first.save().unwrap();
        stale
            .desired_mut()
            .locations
            .paths
            .insert("two".into(), "/two".into());
        assert!(stale.save().is_err());
        assert!(std::fs::read_to_string(path).unwrap().contains("one"));
    }

    #[test]
    fn location_resolution_is_explicit_and_cycle_safe() {
        let mut locations = crate::schema::LocationsConfig::default();
        locations
            .paths
            .insert("a".into(), "${locations.b}/a".into());
        locations.paths.insert("b".into(), "~/b".into());
        let env = AppEnvironment::new(Some(PathBuf::from("/foreign")), []);
        assert_eq!(
            resolve_location_path("${locations.a}", &locations, &env).unwrap(),
            PathBuf::from("/foreign/b/a")
        );
        assert_eq!(
            resolve_location_path("${locations.missing}", &locations, &env).unwrap(),
            PathBuf::from("${locations.missing}")
        );
        locations.paths.insert("b".into(), "${locations.a}".into());
        assert!(resolve_location_path("${locations.a}", &locations, &env).is_err());
    }

    #[test]
    fn canonical_formats_and_native_toml_values_decode_without_io() {
        for format in [ConfigFormat::Json, ConfigFormat::Toml] {
            assert!(decode_bossa_config(format, b"\xff").is_err());
        }
        let json = decode_bossa_config(
            ConfigFormat::Json,
            br#"{"collections":{"refs":{"path":"~/refs"}},"defaults":{"custom.key":3}}"#,
        )
        .unwrap();
        let toml = decode_bossa_config(
            ConfigFormat::Toml,
            b"[collections.refs]\npath='~/refs'\n[defaults]\n'custom.key'=3\n",
        )
        .unwrap();
        assert_eq!(
            json.desired.collections["refs"].path,
            toml.desired.collections["refs"].path
        );
        assert!(json.desired.defaults.settings.contains_key("custom.key"));
        let native = decode_bossa_config(
            ConfigFormat::Toml,
            b"[defaults]\n'custom.float'=nan\n[configs.git.core]\nwhen=1979-05-27T07:32:00Z\n",
        )
        .unwrap();
        assert!(
            matches!(native.desired.defaults.settings["custom.float"], crate::schema::DefaultValue::Float(n) if n.is_nan())
        );
        assert!(
            native.desired.configs.git.unwrap().core["when"]
                .as_datetime()
                .is_some()
        );
    }

    #[cfg(unix)]
    #[test]
    fn absent_and_selection_changes_conflict_with_loaded_authority() {
        let temp = tempfile::tempdir().unwrap();
        let store = DocumentStore::config(temp.path(), "config").unwrap();
        let mut absent = LoadedBossaConfig::load(store.clone(), false).unwrap();
        assert!(!absent.has_document());
        std::fs::write(temp.path().join("config.json"), "{}").unwrap();
        assert!(absent.save().is_err());
        let mut json = LoadedBossaConfig::load(store, true).unwrap();
        assert_eq!(json.selected().unwrap().1, ConfigFormat::Json);
        std::fs::write(temp.path().join("config.toml"), "").unwrap();
        assert!(json.save().is_err());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("config.json")).unwrap(),
            "{}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn legacy_load_cannot_save_without_explicit_conversion() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let original = r#"{"collections":[{"name":"refs","path":"~/refs"}]}"#;
        std::fs::write(&path, original).unwrap();
        let store = DocumentStore::config(temp.path(), "config").unwrap();
        let mut loaded = LoadedBossaConfig::load(store.clone(), true).unwrap();
        assert!(loaded.is_legacy());
        assert!(loaded.save().is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        // Explicit conversion uses a fresh revision and the selected format.
        let snapshot = store.read().unwrap();
        let converted = serde_json::to_string_pretty(loaded.desired()).unwrap();
        store.commit(&snapshot.revision, &converted).unwrap();
        let converted = LoadedBossaConfig::load(store, true).unwrap();
        assert!(!converted.is_legacy());
        assert_eq!(converted.desired().collections["refs"].path, "~/refs");
    }

    #[cfg(unix)]
    #[test]
    fn update_noop_and_save_preserve_symlink_and_permissions() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.json");
        let link = temp.path().join("config.json");
        std::fs::write(&target, "{}\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, &link).unwrap();
        let store = DocumentStore::config(temp.path(), "config").unwrap();
        let inode = std::fs::metadata(&target).unwrap().ino();
        update_bossa_config(&store, |_| Ok(())).unwrap();
        assert_eq!(std::fs::metadata(&target).unwrap().ino(), inode);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}\n");
        let mut loaded = LoadedBossaConfig::load(store, true).unwrap();
        loaded
            .desired_mut()
            .locations
            .paths
            .insert("dev".into(), "/fixture".into());
        assert_eq!(loaded.save().unwrap(), link);
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(
            std::fs::read_to_string(&target)
                .unwrap()
                .contains("/fixture")
        );
    }

    #[cfg(unix)]
    #[test]
    fn interpolated_non_utf8_environment_is_rejected_but_literal_fallback_remains() {
        use std::os::unix::ffi::OsStringExt;
        let non_utf8 = std::ffi::OsString::from_vec(vec![b'/', 0xff]);
        let env = AppEnvironment::new(
            Some(PathBuf::from(non_utf8.clone())),
            [("ROOT".into(), non_utf8)],
        );
        let mut locations = crate::schema::LocationsConfig::default();
        locations.paths.insert("home".into(), "~/data".into());
        locations.paths.insert("env".into(), "$ROOT/data".into());
        assert!(resolve_location_path("${locations.home}", &locations, &env).is_err());
        assert!(resolve_location_path("${locations.env}", &locations, &env).is_err());
        assert_eq!(
            resolve_location_path("~/data", &locations, &env).unwrap(),
            PathBuf::from("~/data")
        );
        assert_eq!(
            resolve_location_path("${locations.missing}", &locations, &env).unwrap(),
            PathBuf::from("${locations.missing}")
        );
    }
}
