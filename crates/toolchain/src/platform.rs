//! Platform detection for binary downloads.
//!
//! This module provides functions to detect the current platform and select
//! the appropriate binary to download. It supports major platforms and
//! architectures commonly used for build tools.
//!
//! # Example
//!
//! ```
//! use toolchain::platform;
//!
//! let platform = platform::detect().expect("unsupported platform");
//! println!("Running on: {}", platform.triple);
//! ```

use crate::error::{Error, Result};
use crate::types::Platform;

/// Detect the current platform.
///
/// Returns the appropriate platform triple for downloading binaries.
///
/// # Supported Platforms
///
/// | OS      | Arch    | Triple                       |
/// |---------|---------|------------------------------|
/// | macOS   | ARM64   | aarch64-apple-darwin         |
/// | macOS   | x86_64  | x86_64-apple-darwin          |
/// | Linux   | ARM64   | aarch64-unknown-linux-gnu    |
/// | Linux   | x86_64  | x86_64-unknown-linux-gnu     |
/// | Linux   | RISC-V  | riscv64gc-unknown-linux-gnu  |
/// | Windows | ARM64   | aarch64-pc-windows-msvc      |
/// | Windows | x86_64  | x86_64-pc-windows-msvc       |
///
/// # Errors
///
/// Returns `Error::UnsupportedPlatform` if the current platform is not supported.
pub fn detect() -> Result<Platform> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let triple = match (os, arch) {
        // macOS
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",

        // Linux (glibc)
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "riscv64") => "riscv64gc-unknown-linux-gnu",

        // Windows
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",

        _ => {
            return Err(Error::UnsupportedPlatform {
                os: os.to_string(),
                arch: arch.to_string(),
            });
        }
    };

    Ok(Platform::new(os, arch, triple))
}

/// Check if we're running on a musl-based Linux.
///
/// This can be used to select musl binaries instead of glibc.
/// Returns `false` on non-Linux platforms.
#[must_use]
pub fn is_musl() -> bool {
    if std::env::consts::OS != "linux" {
        return false;
    }

    // Check if /lib/ld-musl-* exists
    let musl_paths = ["/lib/ld-musl-x86_64.so.1", "/lib/ld-musl-aarch64.so.1"];

    musl_paths.iter().any(|p| std::path::Path::new(p).exists())
}

/// Get the musl variant of a platform triple.
///
/// Converts a glibc Linux triple to its musl equivalent.
/// Returns `None` for non-Linux triples or unsupported architectures.
#[must_use]
pub fn to_musl_triple(triple: &str) -> Option<&'static str> {
    match triple {
        "aarch64-unknown-linux-gnu" => Some("aarch64-unknown-linux-musl"),
        "x86_64-unknown-linux-gnu" => Some("x86_64-unknown-linux-musl"),
        _ => None,
    }
}

/// Get the file extension for executables on this platform.
///
/// Returns ".exe" on Windows, empty string on other platforms.
#[must_use]
pub fn executable_extension() -> &'static str {
    if std::env::consts::OS == "windows" {
        ".exe"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_platform() {
        let platform = detect().expect("should detect platform");
        assert!(!platform.os.is_empty());
        assert!(!platform.arch.is_empty());
        assert!(!platform.triple.is_empty());
    }

    #[test]
    fn test_detect_platform_has_valid_os() {
        let platform = detect().expect("should detect platform");
        let valid_os = ["macos", "linux", "windows"];
        assert!(valid_os.contains(&platform.os.as_str()));
    }

    #[test]
    fn test_detect_platform_has_valid_arch() {
        let platform = detect().expect("should detect platform");
        let valid_arch = ["aarch64", "x86_64", "riscv64"];
        assert!(valid_arch.contains(&platform.arch.as_str()));
    }

    #[test]
    fn test_executable_extension() {
        let ext = executable_extension();
        #[cfg(windows)]
        assert_eq!(ext, ".exe");
        #[cfg(not(windows))]
        assert_eq!(ext, "");
    }

    #[test]
    fn test_to_musl_triple_aarch64() {
        let result = to_musl_triple("aarch64-unknown-linux-gnu");
        assert_eq!(result, Some("aarch64-unknown-linux-musl"));
    }

    #[test]
    fn test_to_musl_triple_x86_64() {
        let result = to_musl_triple("x86_64-unknown-linux-gnu");
        assert_eq!(result, Some("x86_64-unknown-linux-musl"));
    }

    #[test]
    fn test_to_musl_triple_macos() {
        let result = to_musl_triple("aarch64-apple-darwin");
        assert_eq!(result, None);
    }

    #[test]
    fn test_to_musl_triple_windows() {
        let result = to_musl_triple("x86_64-pc-windows-msvc");
        assert_eq!(result, None);
    }

    #[test]
    fn test_to_musl_triple_already_musl() {
        let result = to_musl_triple("x86_64-unknown-linux-musl");
        assert_eq!(result, None);
    }

    #[test]
    fn test_is_musl_on_non_linux() {
        // On non-Linux platforms, this should always return false
        #[cfg(not(target_os = "linux"))]
        assert!(!is_musl());
    }
}
