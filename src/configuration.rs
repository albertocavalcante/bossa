//! Configuration source selection without migration, writes, or ambient paths.

use serde::de::DeserializeOwned;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Toml,
    Json,
}

impl ConfigFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Invalid configuration base name")]
    InvalidName,
    #[error("Could not read configuration {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Invalid configuration {}: {message}", path.display())]
    Parse { path: PathBuf, message: String },
}

/// None means confirmed absence; permission, type and read errors remain errors.
pub trait ConfigSource {
    fn read(&self, path: &Path) -> io::Result<Option<String>>;
}

/// Native regular-file reader. The default limit is 8 MiB; callers can choose a
/// different explicit limit. Over-limit inputs are rejected without parsing.
pub struct FileConfigSource {
    max_bytes: u64,
}

impl Default for FileConfigSource {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_BYTES)
    }
}

impl FileConfigSource {
    pub fn new(max_bytes: u64) -> Self {
        Self { max_bytes }
    }
}

impl ConfigSource for FileConfigSource {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                confirm_missing_parent(path)?;
                return Ok(None);
            }
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        // Follow intentional config symlinks, but reject dangling links and
        // special files. A present entry that fails to open never means absent.
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Expected a regular configuration file",
            ));
        }
        if metadata.len() > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Configuration exceeds byte limit",
            ));
        }
        let mut bytes = Vec::new();
        File::open(path)?
            .take(self.max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Configuration exceeds byte limit",
            ));
        }
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

pub(crate) fn confirm_missing_parent(path: &Path) -> io::Result<()> {
    // Missing leaves under inaccessible or dangling directories are unavailable.
    for ancestor in path.ancestors().skip(1) {
        let ancestor = if ancestor.as_os_str().is_empty() {
            Path::new(".")
        } else {
            ancestor
        };
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                if !fs::metadata(ancestor)?.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        "Configuration parent is not a directory",
                    ));
                }
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Stage complete contents before replacing the selected file. File symlinks
/// and permissions are preserved. Concurrent read-modify-write coordination
/// remains the store's responsibility.
pub fn write_config_document(path: &Path, contents: &str) -> io::Result<()> {
    if contents.len() as u64 > DEFAULT_MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Configuration exceeds byte limit",
        ));
    }
    replace_config_with(path, |file| file.write_all(contents.as_bytes()))
}

fn replace_config_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let filename = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Configuration needs a filename",
        )
    })?;
    let existing = match fs::symlink_metadata(path) {
        Ok(_) => Some(fs::metadata(path)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            confirm_missing_parent(path)?;
            None
        }
        Err(error) => return Err(error),
    };
    if let Some(metadata) = &existing
        && (!metadata.is_file() || metadata.permissions().readonly())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Configuration is not a writable regular file",
        ));
    }
    let target = if existing.is_some() {
        path.canonicalize()?
    } else {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        parent.canonicalize()?.join(filename)
    };
    let parent = target.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Configuration needs a parent directory",
        )
    })?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    write(staged.as_file_mut())?;
    if let Some(metadata) = existing.as_ref() {
        staged.as_file().set_permissions(metadata.permissions())?;
    }
    staged.as_file().sync_all()?;
    if existing.is_some() {
        if path.canonicalize()? != target {
            return Err(io::Error::other(
                "Selected configuration path changed before replacement",
            ));
        }
        staged.persist(target).map_err(|error| error.error)?;
    } else {
        let selected_parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if selected_parent.canonicalize()? != parent {
            return Err(io::Error::other(
                "Selected configuration directory changed before replacement",
            ));
        }
        staged
            .persist_noclobber(target)
            .map_err(|error| error.error)?;
    }
    Ok(())
}

/// Source metadata and original contents. No Debug implementation prints bytes.
pub struct ConfigDocument {
    pub path: PathBuf,
    pub format: ConfigFormat,
    contents: String,
}

impl ConfigDocument {
    pub fn read(
        source: &dyn ConfigSource,
        path: &Path,
        format: ConfigFormat,
    ) -> Result<Option<Self>, ConfigError> {
        Ok(source
            .read(path)
            .map_err(|source| ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })?
            .map(|contents| Self {
                path: path.to_path_buf(),
                format,
                contents,
            }))
    }

    pub fn contents(&self) -> &str {
        &self.contents
    }

    pub fn parse<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        let result = match self.format {
            ConfigFormat::Toml => toml::from_str(&self.contents)
                .map_err(|error: toml::de::Error| error.message().to_string()),
            ConfigFormat::Json => reject_duplicate_json_keys(&self.contents)
                .and_then(|()| serde_json::from_str(&self.contents))
                .map_err(|error| error.to_string()),
        };
        result.map_err(|message| ConfigError::Parse {
            path: self.path.clone(),
            message,
        })
    }
}

fn reject_duplicate_json_keys(contents: &str) -> Result<(), serde_json::Error> {
    use serde::{Deserialize, Deserializer, de};
    use std::collections::HashSet;
    use std::fmt;

    struct UniqueFields;
    struct Visitor;

    impl<'de> Deserialize<'de> for UniqueFields {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_any(Visitor)
        }
    }

    impl<'de> de::Visitor<'de> for Visitor {
        type Value = UniqueFields;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("JSON without duplicate object keys")
        }

        fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }
        fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }
        fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }
        fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }

        fn visit_seq<A: de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            while sequence.next_element::<UniqueFields>()?.is_some() {}
            Ok(UniqueFields)
        }

        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut keys = HashSet::new();
            while let Some(key) = map.next_key::<String>()? {
                if !keys.insert(key) {
                    return Err(de::Error::custom("duplicate object key"));
                }
                map.next_value::<UniqueFields>()?;
            }
            Ok(UniqueFields)
        }
    }

    let mut deserializer = serde_json::Deserializer::from_str(contents);
    UniqueFields::deserialize(&mut deserializer)?;
    deserializer.end()
}

/// Select TOML before JSON. An unreadable or malformed selected document must
/// not be replaced by a lower-priority source or a default configuration.
pub fn load_config_document(
    source: &dyn ConfigSource,
    directory: &Path,
    base_name: &str,
) -> Result<Option<ConfigDocument>, ConfigError> {
    let mut components = Path::new(base_name).components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || base_name.contains(['/', '\\'])
    {
        return Err(ConfigError::InvalidName);
    }
    for format in [ConfigFormat::Toml, ConfigFormat::Json] {
        let path = directory.join(format!("{base_name}.{}", format.extension()));
        if let Some(document) = ConfigDocument::read(source, &path, format)? {
            return Ok(Some(document));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct Unreadable(RefCell<Vec<PathBuf>>);

    impl ConfigSource for Unreadable {
        fn read(&self, path: &Path) -> io::Result<Option<String>> {
            self.0.borrow_mut().push(path.to_path_buf());
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
    }

    #[test]
    fn a_failed_higher_priority_read_never_falls_back() {
        let source = Unreadable(RefCell::new(Vec::new()));
        let error = load_config_document(&source, Path::new("/fixture"), "config");
        assert!(
            matches!(error, Err(ConfigError::Read { source, .. }) if source.kind() == io::ErrorKind::PermissionDenied)
        );
        assert_eq!(
            *source.0.borrow(),
            vec![PathBuf::from("/fixture/config.toml")]
        );
    }

    #[test]
    fn malformed_toml_does_not_fall_back_to_valid_json() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join("config.toml"), "[broken").unwrap();
        fs::write(temp.path().join("config.json"), "{}").unwrap();
        let document = load_config_document(&FileConfigSource::default(), temp.path(), "config")
            .unwrap()
            .unwrap();
        assert_eq!(document.format, ConfigFormat::Toml);
        assert!(document.parse::<HashMap<String, String>>().is_err());
        assert_eq!(
            fs::read(temp.path().join("config.toml")).unwrap(),
            b"[broken"
        );
    }

    #[test]
    fn absence_and_json_selection_are_explicit_and_read_only() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(
            load_config_document(&FileConfigSource::default(), temp.path(), "config")
                .unwrap()
                .is_none()
        );
        fs::write(temp.path().join("config.json"), r#"{"key":"value"}"#).unwrap();
        let document = load_config_document(&FileConfigSource::default(), temp.path(), "config")
            .unwrap()
            .unwrap();
        assert_eq!(document.format, ConfigFormat::Json);
        assert_eq!(
            document.parse::<HashMap<String, String>>().unwrap()["key"],
            "value"
        );
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn byte_limit_accepts_the_boundary_and_rejects_without_truncating() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("config.json");
        fs::write(&path, b"12345678").unwrap();
        assert_eq!(
            FileConfigSource::new(8).read(&path).unwrap().unwrap(),
            "12345678"
        );
        fs::write(&path, b"123456789").unwrap();
        assert!(FileConfigSource::new(8).read(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), b"123456789");
    }

    #[cfg(unix)]
    #[test]
    fn dangling_config_symlink_is_not_absence() {
        let temp = tempfile::TempDir::new().unwrap();
        std::os::unix::fs::symlink(temp.path().join("missing"), temp.path().join("config.toml"))
            .unwrap();
        fs::write(temp.path().join("config.json"), "{}").unwrap();
        assert!(load_config_document(&FileConfigSource::default(), temp.path(), "config").is_err());
    }

    #[test]
    fn base_name_cannot_escape_the_selected_directory() {
        let source = Unreadable(RefCell::new(Vec::new()));
        for name in ["", ".", "..", "../config", "a/b", "a\\b", "/absolute"] {
            assert!(matches!(
                load_config_document(&source, Path::new("/fixture"), name),
                Err(ConfigError::InvalidName)
            ));
        }
        assert!(source.0.borrow().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_parent_is_unavailable_but_a_new_directory_is_absent() {
        let temp = tempfile::TempDir::new().unwrap();
        let missing = temp.path().join("missing");
        assert!(
            load_config_document(&FileConfigSource::default(), &missing, "config")
                .unwrap()
                .is_none()
        );
        assert!(!missing.exists());
        let dangling = temp.path().join("dangling");
        std::os::unix::fs::symlink(&missing, &dangling).unwrap();
        assert!(
            load_config_document(
                &FileConfigSource::default(),
                &dangling.join("nested"),
                "config"
            )
            .is_err()
        );
    }

    #[test]
    fn large_shallow_input_is_accepted_but_excessive_json_depth_is_rejected() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("config.json");
        let value = "x".repeat(1024 * 1024);
        let contents = serde_json::to_string(&HashMap::from([("value", &value)])).unwrap();
        fs::write(&path, &contents).unwrap();
        let document = load_config_document(&FileConfigSource::default(), temp.path(), "config")
            .unwrap()
            .unwrap();
        assert_eq!(
            document.parse::<HashMap<String, String>>().unwrap()["value"],
            value
        );
        let deep = format!("{}0{}", "[".repeat(256), "]".repeat(256));
        fs::write(&path, &deep).unwrap();
        let document = load_config_document(&FileConfigSource::default(), temp.path(), "config")
            .unwrap()
            .unwrap();
        assert!(document.parse::<serde_json::Value>().is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), deep);
    }

    #[test]
    fn duplicate_json_keys_are_rejected_at_every_depth_including_escaped_keys() {
        for contents in [
            r#"{"key":1,"key":2}"#,
            r#"{"outer":[{"key":1,"k\u0065y":2}]}"#,
        ] {
            let document = ConfigDocument {
                path: "fixture.json".into(),
                format: ConfigFormat::Json,
                contents: contents.into(),
            };
            assert!(
                document
                    .parse::<serde_json::Value>()
                    .unwrap_err()
                    .to_string()
                    .contains("duplicate object key")
            );
        }
        let document = ConfigDocument {
            path: "fixture.json".into(),
            format: ConfigFormat::Json,
            contents: r#"{"left":{"key":1},"right":{"key":2}}"#.into(),
        };
        assert!(document.parse::<serde_json::Value>().is_ok());
    }

    #[test]
    fn staging_failure_preserves_original_bytes_and_removes_temporary_file() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(&path, "original").unwrap();
        let result = replace_config_with(&path, |file| {
            file.write_all(b"partial staged content")?;
            Err(io::Error::other("injected write failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(path).unwrap(), b"original");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn replacement_preserves_file_symlink_and_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("source.toml");
        let link = temp.path().join("config.toml");
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_config_document(&link, "new").unwrap();
        assert!(link.is_symlink());
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    }

    #[test]
    fn a_destination_created_during_staging_is_not_ours_to_overwrite() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        let result = replace_config_with(&path, |file| {
            fs::write(&path, "concurrent work")?;
            file.write_all(b"candidate")
        });
        assert!(result.is_err());
        assert_eq!(fs::read(path).unwrap(), b"concurrent work");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn oversize_generated_configuration_fails_before_creating_directories() {
        let temp = tempfile::TempDir::new().unwrap();
        let contents = "x".repeat(DEFAULT_MAX_BYTES as usize + 1);
        assert!(write_config_document(&temp.path().join("new/config.toml"), &contents).is_err());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn locked_toml_parser_rejects_deep_arrays_inline_tables_and_dotted_keys() {
        for contents in [
            format!("value = {}0{}", "[".repeat(256), "]".repeat(256)),
            format!("value = {}0{}", "{ key = ".repeat(256), "}".repeat(256)),
            format!("[{}]\nvalue = 0", vec!["key"; 256].join(".")),
        ] {
            let document = ConfigDocument {
                path: "fixture.toml".into(),
                format: ConfigFormat::Toml,
                contents,
            };
            let error = document.parse::<toml::Value>().unwrap_err().to_string();
            assert!(error.contains("recursion"), "{error}");
        }
    }
}
