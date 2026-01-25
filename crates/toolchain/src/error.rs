//! Error types for toolchain operations.
//!
//! This module provides error types and categories for all toolchain operations.
//! Errors are categorized to enable smart retry logic and appropriate user feedback.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Result type alias for toolchain operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Categories of toolchain errors for retry logic.
///
/// Error categories help determine whether an operation should be retried
/// and what kind of user feedback is appropriate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Network-related errors (transient, retryable).
    Network,
    /// Platform not supported.
    Platform,
    /// Tool or version not found.
    NotFound,
    /// Permission denied during installation.
    Permission,
    /// Decompression or file format error.
    Format,
    /// Tool already installed (may be ignorable).
    AlreadyInstalled,
    /// Other/unknown errors.
    Other,
}

impl ErrorCategory {
    /// Whether this error category is typically transient and worth retrying.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Network)
    }

    /// Whether this error can be safely ignored (operation already done).
    #[must_use]
    pub fn is_ignorable(&self) -> bool {
        matches!(self, Self::AlreadyInstalled)
    }

    /// Get a user-friendly description of this error category.
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Network => "Network connectivity issue",
            Self::Platform => "Unsupported platform",
            Self::NotFound => "Tool or version not found",
            Self::Permission => "Permission denied",
            Self::Format => "Invalid file format",
            Self::AlreadyInstalled => "Already installed",
            Self::Other => "Unexpected error",
        }
    }

    /// Get actionable advice for resolving this error category.
    #[must_use]
    pub fn advice(&self) -> &'static str {
        match self {
            Self::Network => "Check your internet connection and try again",
            Self::Platform => "This tool may not be available for your platform",
            Self::NotFound => "Verify the tool name and version are correct",
            Self::Permission => "Check directory permissions or run with appropriate access",
            Self::Format => "The downloaded file may be corrupted, try again",
            Self::AlreadyInstalled => "Use --force to overwrite the existing installation",
            Self::Other => "Check the error details for more information",
        }
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

/// Errors that can occur during toolchain operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Failed to detect the current platform.
    #[error("unsupported platform: {os}/{arch}")]
    UnsupportedPlatform {
        /// Operating system.
        os: String,
        /// CPU architecture.
        arch: String,
    },

    /// HTTP request failed.
    #[error("HTTP request failed: {message}")]
    HttpError {
        /// Error message.
        message: String,
        /// HTTP status code if available.
        status: Option<u16>,
    },

    /// Failed to download a release.
    #[error("download failed for {tool}: {message}")]
    DownloadFailed {
        /// Tool being downloaded.
        tool: String,
        /// Error message.
        message: String,
    },

    /// Failed to decompress archive.
    #[error("decompression failed: {0}")]
    DecompressionFailed(String),

    /// IO error during file operations.
    #[error("IO error at {path}: {source}")]
    Io {
        /// Path involved in the error.
        path: PathBuf,
        /// Underlying IO error.
        #[source]
        source: io::Error,
    },

    /// Tool not found after installation.
    #[error("tool not found: {0}")]
    ToolNotFound(String),

    /// Version not found in releases.
    #[error("version {version} not found for {tool}")]
    VersionNotFound {
        /// Tool name.
        tool: String,
        /// Requested version.
        version: String,
    },

    /// GitHub API error.
    #[error("GitHub API error: {0}")]
    GitHubApi(String),

    /// Invalid response from API.
    #[error("invalid API response: {0}")]
    InvalidResponse(String),

    /// Permission denied during installation.
    #[error("permission denied: {path}")]
    PermissionDenied {
        /// Path that couldn't be accessed.
        path: PathBuf,
    },

    /// Generic error.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Create an IO error with path context.
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Create an HTTP error.
    pub fn http(message: impl Into<String>, status: Option<u16>) -> Self {
        Self::HttpError {
            message: message.into(),
            status,
        }
    }

    /// Get the error category for retry logic.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Error::UnsupportedPlatform { .. } => ErrorCategory::Platform,
            Error::HttpError { .. } => ErrorCategory::Network,
            Error::DownloadFailed { .. } => ErrorCategory::Network,
            Error::DecompressionFailed(_) => ErrorCategory::Format,
            Error::Io { source, .. } => {
                if source.kind() == io::ErrorKind::PermissionDenied {
                    ErrorCategory::Permission
                } else {
                    ErrorCategory::Other
                }
            }
            Error::ToolNotFound(_) => ErrorCategory::NotFound,
            Error::VersionNotFound { .. } => ErrorCategory::NotFound,
            Error::GitHubApi(_) => ErrorCategory::Network,
            Error::InvalidResponse(_) => ErrorCategory::Format,
            Error::PermissionDenied { .. } => ErrorCategory::Permission,
            Error::Other(msg) => {
                if msg.contains("already installed") {
                    ErrorCategory::AlreadyInstalled
                } else {
                    ErrorCategory::Other
                }
            }
        }
    }

    /// Whether this error is typically transient and worth retrying.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.category().is_retryable()
    }

    /// Whether this error can be safely ignored.
    #[must_use]
    pub fn is_ignorable(&self) -> bool {
        self.category().is_ignorable()
    }
}

impl From<ureq::Error> for Error {
    fn from(err: ureq::Error) -> Self {
        match err {
            ureq::Error::StatusCode(code) => Self::HttpError {
                message: format!("HTTP {}", code),
                status: Some(code),
            },
            other => Self::HttpError {
                message: other.to_string(),
                status: None,
            },
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::Io {
            path: PathBuf::new(),
            source: err,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::InvalidResponse(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_category_retryable() {
        assert!(ErrorCategory::Network.is_retryable());
        assert!(!ErrorCategory::Platform.is_retryable());
        assert!(!ErrorCategory::NotFound.is_retryable());
        assert!(!ErrorCategory::Permission.is_retryable());
        assert!(!ErrorCategory::Format.is_retryable());
        assert!(!ErrorCategory::AlreadyInstalled.is_retryable());
        assert!(!ErrorCategory::Other.is_retryable());
    }

    #[test]
    fn test_error_category_ignorable() {
        assert!(ErrorCategory::AlreadyInstalled.is_ignorable());
        assert!(!ErrorCategory::Network.is_ignorable());
        assert!(!ErrorCategory::Platform.is_ignorable());
        assert!(!ErrorCategory::NotFound.is_ignorable());
        assert!(!ErrorCategory::Permission.is_ignorable());
        assert!(!ErrorCategory::Format.is_ignorable());
        assert!(!ErrorCategory::Other.is_ignorable());
    }

    #[test]
    fn test_error_category_description() {
        assert!(!ErrorCategory::Network.description().is_empty());
        assert!(!ErrorCategory::Platform.description().is_empty());
        assert!(!ErrorCategory::NotFound.description().is_empty());
    }

    #[test]
    fn test_error_category_advice() {
        assert!(!ErrorCategory::Network.advice().is_empty());
        assert!(!ErrorCategory::Platform.advice().is_empty());
        assert!(!ErrorCategory::NotFound.advice().is_empty());
    }

    #[test]
    fn test_error_category_display() {
        let display = format!("{}", ErrorCategory::Network);
        assert!(display.contains("Network"));
    }

    #[test]
    fn test_error_http_category() {
        let err = Error::http("connection failed", Some(503));
        assert_eq!(err.category(), ErrorCategory::Network);
        assert!(err.is_retryable());
    }

    #[test]
    fn test_error_unsupported_platform_category() {
        let err = Error::UnsupportedPlatform {
            os: "plan9".to_string(),
            arch: "mips".to_string(),
        };
        assert_eq!(err.category(), ErrorCategory::Platform);
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_error_download_failed_category() {
        let err = Error::DownloadFailed {
            tool: "buck2".to_string(),
            message: "timeout".to_string(),
        };
        assert_eq!(err.category(), ErrorCategory::Network);
        assert!(err.is_retryable());
    }

    #[test]
    fn test_error_decompression_failed_category() {
        let err = Error::DecompressionFailed("invalid data".to_string());
        assert_eq!(err.category(), ErrorCategory::Format);
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_error_io_permission_denied_category() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");
        let err = Error::io("/usr/local/bin", io_err);
        assert_eq!(err.category(), ErrorCategory::Permission);
    }

    #[test]
    fn test_error_io_other_category() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "not found");
        let err = Error::io("/some/path", io_err);
        assert_eq!(err.category(), ErrorCategory::Other);
    }

    #[test]
    fn test_error_tool_not_found_category() {
        let err = Error::ToolNotFound("buck2".to_string());
        assert_eq!(err.category(), ErrorCategory::NotFound);
    }

    #[test]
    fn test_error_version_not_found_category() {
        let err = Error::VersionNotFound {
            tool: "buck2".to_string(),
            version: "nonexistent".to_string(),
        };
        assert_eq!(err.category(), ErrorCategory::NotFound);
    }

    #[test]
    fn test_error_already_installed_category() {
        let err = Error::Other("buck2 already installed at /usr/local/bin".to_string());
        assert_eq!(err.category(), ErrorCategory::AlreadyInstalled);
        assert!(err.is_ignorable());
    }

    #[test]
    fn test_error_permission_denied_category() {
        let err = Error::PermissionDenied {
            path: PathBuf::from("/usr/local/bin"),
        };
        assert_eq!(err.category(), ErrorCategory::Permission);
    }

    #[test]
    fn test_error_display() {
        let err = Error::UnsupportedPlatform {
            os: "plan9".to_string(),
            arch: "mips".to_string(),
        };
        let display = format!("{}", err);
        assert!(display.contains("plan9"));
        assert!(display.contains("mips"));
    }

    #[test]
    fn test_error_io_constructor() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err = Error::io("/some/path", io_err);
        match err {
            Error::Io { path, .. } => {
                assert_eq!(path, PathBuf::from("/some/path"));
            }
            _ => panic!("Expected Error::Io"),
        }
    }

    #[test]
    fn test_error_http_constructor() {
        let err = Error::http("connection reset", Some(502));
        match err {
            Error::HttpError { message, status } => {
                assert_eq!(message, "connection reset");
                assert_eq!(status, Some(502));
            }
            _ => panic!("Expected Error::HttpError"),
        }
    }

    #[test]
    fn test_error_from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "not found");
        let err: Error = io_err.into();
        match err {
            Error::Io { path, .. } => {
                assert_eq!(path, PathBuf::new()); // Default path when converted directly
            }
            _ => panic!("Expected Error::Io"),
        }
    }
}
