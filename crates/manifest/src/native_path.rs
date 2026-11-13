//! Versioned, lossless native path identities.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// A platform-tagged path identity. Unix paths contain bytes; Windows paths
/// contain UTF-16 code units. The representation preserves non-Unicode names.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "encoding", content = "units", rename_all = "snake_case")]
pub enum NativePath {
    /// Unix OS-native bytes.
    UnixBytesV1(Vec<u8>),
    /// Windows OS-native UTF-16 units.
    WindowsUtf16V1(Vec<u16>),
}

impl NativePath {
    /// Encode an OS-native path without Unicode conversion.
    #[cfg(unix)]
    pub fn from_path(path: &Path) -> Self {
        use std::os::unix::ffi::OsStrExt;
        Self::UnixBytesV1(path.as_os_str().as_bytes().to_vec())
    }
    /// Encode an OS-native path without Unicode conversion.
    #[cfg(windows)]
    pub fn from_path(path: &Path) -> Self {
        use std::os::windows::ffi::OsStrExt;
        Self::WindowsUtf16V1(path.as_os_str().encode_wide().collect())
    }
    /// Decode only identities for this operating system.
    pub fn to_path_buf(&self) -> Result<PathBuf> {
        match self {
            #[cfg(unix)]
            Self::UnixBytesV1(bytes) if !bytes.contains(&0) => {
                use std::os::unix::ffi::OsStringExt;
                Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.clone())))
            }
            #[cfg(windows)]
            Self::WindowsUtf16V1(units) if !units.contains(&0) => {
                use std::os::windows::ffi::OsStringExt;
                Ok(PathBuf::from(std::ffi::OsString::from_wide(units)))
            }
            _ => Err(Error::InvalidPath(
                "foreign or malformed path identity".into(),
            )),
        }
    }
    pub(crate) fn to_blob(&self) -> Vec<u8> {
        match self {
            Self::UnixBytesV1(bytes) => [b"U1".as_slice(), bytes].concat(),
            Self::WindowsUtf16V1(units) => {
                let mut data = b"W1".to_vec();
                for unit in units {
                    data.extend_from_slice(&unit.to_le_bytes());
                }
                data
            }
        }
    }
    pub(crate) fn from_blob(data: &[u8]) -> Result<Self> {
        let value = match data.get(..2) {
            Some([b'U', b'1']) => Self::UnixBytesV1(data[2..].to_vec()),
            Some([b'W', b'1']) if data.len().is_multiple_of(2) => Self::WindowsUtf16V1(
                data[2..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect(),
            ),
            _ => return Err(Error::InvalidPath("malformed path encoding".into())),
        };
        value.to_path_buf()?;
        Ok(value)
    }
    pub(crate) fn relative(&self) -> Result<PathBuf> {
        let path = self.to_path_buf()?;
        if path.as_os_str().is_empty()
            || !path.components().all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(Error::InvalidPath("expected nonempty relative path".into()));
        }
        Ok(path)
    }
}

impl std::fmt::Display for NativePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.to_path_buf() {
            Ok(path) => {
                for c in path.to_string_lossy().chars() {
                    if c.is_control() {
                        write!(f, "{}", c.escape_default())?;
                    } else {
                        write!(f, "{c}")?;
                    }
                }
                Ok(())
            }
            Err(_) => f.write_str("<invalid native path>"),
        }
    }
}
