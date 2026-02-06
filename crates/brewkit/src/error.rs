//! Error types for Homebrew operations.
//!
//! Errors are categorized to enable smart retry logic and appropriate
//! user feedback. Each error type includes contextual information to
//! help users understand what went wrong and how to fix it.

use std::path::PathBuf;
use thiserror::Error;

/// Categories of Homebrew errors for retry logic.
///
/// Error categories help determine whether an operation should be retried
/// and what kind of user feedback is appropriate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Network-related errors (transient, retryable)
    Network,
    /// Package not found in any tap
    NotFound,
    /// Version or dependency conflict
    Conflict,
    /// Permission denied (may need sudo)
    Permission,
    /// Package is already installed
    AlreadyInstalled,
    /// Homebrew not found or not configured
    BrewNotFound,
    /// Other/unknown errors
    Other,
}

impl ErrorCategory {
    /// Whether this error category is typically transient and worth retrying.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Network)
    }

    /// Whether this error can be safely ignored (operation already done).
    pub fn is_ignorable(&self) -> bool {
        matches!(self, Self::AlreadyInstalled)
    }

    /// Get a user-friendly description of this error category.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Network => "Network connectivity issue",
            Self::NotFound => "Package not found",
            Self::Conflict => "Package conflict",
            Self::Permission => "Permission denied",
            Self::AlreadyInstalled => "Already installed",
            Self::BrewNotFound => "Homebrew not installed",
            Self::Other => "Unexpected error",
        }
    }

    /// Get actionable advice for resolving this error category.
    pub fn advice(&self) -> &'static str {
        match self {
            Self::Network => "Check your internet connection and try again",
            Self::NotFound => "Verify the package name or add the required tap",
            Self::Conflict => "Resolve the conflict by removing conflicting packages",
            Self::Permission => "Check directory permissions or run with appropriate access",
            Self::AlreadyInstalled => "No action needed - package is already installed",
            Self::BrewNotFound => "Install Homebrew from https://brew.sh",
            Self::Other => "Check the error details for more information",
        }
    }
}

/// Errors that can occur during Homebrew operations.
///
/// Each variant includes relevant context to help diagnose and resolve issues.
#[derive(Debug, Error)]
pub enum Error {
    /// Network-related error (connection, timeout, DNS, etc.)
    #[error("network error: {message}")]
    Network {
        /// Detailed error message from the failed network operation
        message: String,
    },

    /// Package not found in any configured tap
    #[error("package not found: {name}")]
    NotFound {
        /// Name of the package that could not be found
        name: String,
    },

    /// Version or dependency conflict
    #[error("conflict: {message}")]
    Conflict {
        /// Description of the conflict
        message: String,
    },

    /// Permission denied
    #[error("permission denied: {message}")]
    Permission {
        /// Details about what permission was denied
        message: String,
    },

    /// Package is already installed
    #[error("already installed: {name}")]
    AlreadyInstalled {
        /// Name of the already-installed package
        name: String,
    },

    /// Homebrew is not installed or not found in PATH
    #[error("Homebrew not found. Install it from https://brew.sh")]
    BrewNotFound,

    /// Brewfile not found at the specified path
    #[error("Brewfile not found: {0}")]
    BrewfileNotFound(PathBuf),

    /// Invalid Brewfile syntax
    #[error("invalid Brewfile syntax at line {line}: {message}")]
    BrewfileParse {
        /// Line number where the parse error occurred (1-indexed)
        line: usize,
        /// Description of the syntax error
        message: String,
    },

    /// Command execution failed
    #[error("command failed: {message}")]
    CommandFailed {
        /// Description of what command failed
        message: String,
        /// Standard error output from the failed command
        stderr: String,
    },

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON parsing error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Other error
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Get the error category for retry logic.
    pub fn category(&self) -> ErrorCategory {
        match self {
            Error::Network { .. } => ErrorCategory::Network,
            Error::NotFound { .. } => ErrorCategory::NotFound,
            Error::Conflict { .. } => ErrorCategory::Conflict,
            Error::Permission { .. } => ErrorCategory::Permission,
            Error::AlreadyInstalled { .. } => ErrorCategory::AlreadyInstalled,
            Error::BrewNotFound => ErrorCategory::BrewNotFound,
            _ => ErrorCategory::Other,
        }
    }

    /// Whether this error is typically transient and worth retrying.
    pub fn is_retryable(&self) -> bool {
        self.category().is_retryable()
    }

    /// Whether this error can be safely ignored.
    pub fn is_ignorable(&self) -> bool {
        self.category().is_ignorable()
    }

    /// Create an error from brew command output.
    ///
    /// Analyzes stderr to categorize the error appropriately.
    pub fn from_brew_output(stderr: &str, package_name: Option<&str>) -> Self {
        let stderr_lower = stderr.to_lowercase();

        // Network errors
        if stderr_lower.contains("curl")
            || stderr_lower.contains("could not resolve")
            || stderr_lower.contains("connection refused")
            || stderr_lower.contains("timed out")
            || stderr_lower.contains("network")
            || stderr_lower.contains("ssl")
            || stderr_lower.contains("certificate")
            || stderr_lower.contains("failed to download")
            || stderr_lower.contains("error: sha256 mismatch")
        {
            return Error::Network {
                message: stderr.trim().to_string(),
            };
        }

        // Not found errors
        if stderr_lower.contains("no available formula")
            || stderr_lower.contains("no formulae found")
            || stderr_lower.contains("no cask with this name")
            || stderr_lower.contains("unknown command")
            || stderr_lower.contains("error: no such keg")
            || stderr_lower.contains("couldn't find")
        {
            return Error::NotFound {
                name: package_name.unwrap_or("unknown").to_string(),
            };
        }

        // Already installed
        if stderr_lower.contains("already installed")
            || stderr_lower.contains("is already an installed")
        {
            return Error::AlreadyInstalled {
                name: package_name.unwrap_or("unknown").to_string(),
            };
        }

        // Conflicts
        if stderr_lower.contains("conflict")
            || stderr_lower.contains("depends on")
            || stderr_lower.contains("dependency")
            || stderr_lower.contains("is a dependency")
        {
            return Error::Conflict {
                message: stderr.trim().to_string(),
            };
        }

        // Permission errors
        if stderr_lower.contains("permission denied")
            || stderr_lower.contains("operation not permitted")
            || stderr_lower.contains("cannot write")
            || stderr_lower.contains("sudo")
        {
            return Error::Permission {
                message: stderr.trim().to_string(),
            };
        }

        // Default to command failed
        Error::CommandFailed {
            message: format!(
                "brew command failed{}",
                package_name
                    .map(|n| format!(" for {n}"))
                    .unwrap_or_default()
            ),
            stderr: stderr.trim().to_string(),
        }
    }
}

/// Result type for Homebrew operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_category_retryable() {
        assert!(ErrorCategory::Network.is_retryable());
        assert!(!ErrorCategory::NotFound.is_retryable());
        assert!(!ErrorCategory::AlreadyInstalled.is_retryable());
    }

    #[test]
    fn test_error_category_ignorable() {
        assert!(ErrorCategory::AlreadyInstalled.is_ignorable());
        assert!(!ErrorCategory::Network.is_ignorable());
        assert!(!ErrorCategory::NotFound.is_ignorable());
    }

    #[test]
    fn test_from_brew_output_network() {
        let err = Error::from_brew_output("curl: (6) Could not resolve host", Some("wget"));
        assert_eq!(err.category(), ErrorCategory::Network);
        assert!(err.is_retryable());
    }

    #[test]
    fn test_from_brew_output_not_found() {
        let err = Error::from_brew_output(
            "Error: No available formula with the name \"foo\"",
            Some("foo"),
        );
        assert_eq!(err.category(), ErrorCategory::NotFound);
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_from_brew_output_already_installed() {
        let err = Error::from_brew_output("Warning: git is already installed", Some("git"));
        assert_eq!(err.category(), ErrorCategory::AlreadyInstalled);
        assert!(err.is_ignorable());
    }

    #[test]
    fn test_from_brew_output_permission() {
        let err = Error::from_brew_output("Permission denied @ dir_s_mkdir", Some("foo"));
        assert_eq!(err.category(), ErrorCategory::Permission);
    }

    #[test]
    fn test_from_brew_output_conflict() {
        let err = Error::from_brew_output("Error: foo conflicts with bar", Some("foo"));
        assert_eq!(err.category(), ErrorCategory::Conflict);
    }
}
