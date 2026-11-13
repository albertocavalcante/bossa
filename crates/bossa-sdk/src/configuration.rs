//! Configuration source selection without migration, writes, or ambient paths.

use crate::confirm_missing_parent;
use serde::de::DeserializeOwned;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024;

use crate::ConfigFormat;

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
    let path = std::path::absolute(path)?;
    let store = crate::DocumentStore::file(&path, ConfigFormat::Toml)?;
    let guard = store.lock().map_err(io::Error::other)?;
    let snapshot = guard.read().map_err(io::Error::other)?;
    guard
        .replace(&snapshot.revision, write)
        .map_err(io::Error::other)?;
    Ok(())
}

/// Source metadata and original contents. No Debug implementation prints bytes.
pub struct ConfigDocument {
    pub path: PathBuf,
    pub format: ConfigFormat,
    contents: String,
}

impl ConfigDocument {
    pub(crate) fn from_contents(path: PathBuf, format: ConfigFormat, contents: String) -> Self {
        Self {
            path,
            format,
            contents,
        }
    }
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
            ConfigFormat::Json => crate::decode_unique_json(self.contents.as_bytes())
                .map_err(|error| error.to_string()),
        };
        result.map_err(|message| ConfigError::Parse {
            path: self.path.clone(),
            message,
        })
    }
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
        assert_directory_entries(temp.path(), &["config.json"]);
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
        assert_directory_entries(temp.path(), &[".bossa-store.lock", "config.toml"]);
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
        assert_directory_entries(
            temp.path(),
            &[".bossa-store.lock", "config.toml", "source.toml"],
        );
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
        assert_directory_entries(temp.path(), &[".bossa-store.lock", "config.toml"]);
    }

    fn assert_directory_entries(path: &Path, expected: &[&str]) {
        let mut actual: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        actual.sort();
        assert_eq!(
            actual,
            expected
                .iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
        );
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
