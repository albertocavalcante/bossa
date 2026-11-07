//! Scoped sudo context with deterministic allowlist
//!
//! Sudo is never requested for the entire process. Instead:
//! 1. Config defines which operations need sudo (allowlist)
//! 2. All changes are computed first (no sudo needed)
//! 3. Sudo is acquired once for privileged batch
//! 4. Sudo is released immediately after

#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use bossa::IoMode;
use declarative::{CommandOutput, SudoClassifier, SudoProvider};
use serde::{Deserialize, Serialize};
use std::process::Output;

/// Configuration for sudo allowlist
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SudoConfig {
    /// Casks that require sudo (e.g., ["docker", "1password"])
    #[serde(default)]
    pub casks: Vec<String>,

    /// macOS defaults that require sudo
    #[serde(default)]
    pub defaults: Vec<String>,

    /// Custom operations requiring sudo
    #[serde(default)]
    pub operations: Vec<String>,
}

impl SudoConfig {
    /// Check if a cask requires sudo
    pub fn cask_requires_sudo(&self, name: &str) -> bool {
        self.casks.iter().any(|c| c == name)
    }

    /// Check if a default requires sudo
    pub fn default_requires_sudo(&self, domain_key: &str) -> bool {
        self.defaults.iter().any(|d| d == domain_key)
    }

    /// Check if an operation requires sudo
    pub fn operation_requires_sudo(&self, op: &str) -> bool {
        self.operations.iter().any(|o| o == op)
    }
}

/// Implement SudoClassifier for SudoConfig
impl SudoClassifier for SudoConfig {
    fn requires_sudo(&self, resource_type: &str, resource_id: &str) -> bool {
        match resource_type {
            "brew_cask" => self.cask_requires_sudo(resource_id),
            "macos_default" => self.default_requires_sudo(resource_id),
            _ => false,
        }
    }
}

/// Scoped sudo context - automatically invalidates on drop
pub struct SudoContext {
    validated: bool,
}

impl SudoContext {
    /// Acquire sudo privileges with a reason shown to user
    pub fn acquire(reason: &str) -> Result<Self> {
        {
            let mut state = TIMESTAMP
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.shutdown || state.active || state.cleanup_failed {
                bail!(
                    "Privilege scope unavailable after shutdown, pending activity, or incomplete cleanup"
                );
            }
            state.active = true;
        }
        let mut context = Self { validated: false };
        // Prompt user with reason
        eprintln!();
        eprintln!("  Sudo required: {reason}");
        eprintln!();

        // Validate sudo (will prompt for password)
        let status = crate::runner::capture_using(
            crate::runner::native_runner(),
            "sudo",
            ["-v"],
            IoMode::Inherit,
        )
        .context("Failed to execute sudo")?
        .status;

        if !status.success() {
            bail!("Failed to acquire sudo privileges");
        }

        context.validated = true;
        Ok(context)
    }

    /// Check if sudo is currently valid (without prompting)
    pub fn is_valid() -> bool {
        crate::runner::capture_using(
            crate::runner::native_runner(),
            "sudo",
            ["-n", "true"],
            IoMode::ForegroundCapture,
        )
        .map(|output| output.status.success())
        .unwrap_or(false)
    }

    /// Run a command with sudo (internal)
    fn run_internal(&self, cmd: &str, args: &[&str]) -> Result<Output> {
        if !self.validated {
            bail!("Sudo context not validated");
        }

        let output = crate::runner::capture_using(
            crate::runner::native_runner(),
            "sudo",
            ["-n", "--", cmd].into_iter().chain(args.iter().copied()),
            IoMode::ForegroundCapture,
        )
        .with_context(|| format!("Failed to execute scoped sudo command {cmd}"))?;

        Ok(output)
    }

    /// Run a command with sudo and capture output
    pub fn run_capture(&self, cmd: &str, args: &[&str]) -> Result<String> {
        let output = self.run_internal(cmd, args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Command failed: {}", stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Run a command with sudo, returning success/failure
    pub fn run_status(&self, cmd: &str, args: &[&str]) -> Result<bool> {
        let output = self.run_internal(cmd, args)?;
        Ok(output.status.success())
    }
}

/// Implement SudoProvider for SudoContext
impl SudoProvider for SudoContext {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
        let output = self.run_internal(cmd, args)?;
        Ok(output.into())
    }
}

// The CLI has one serial privilege scope. Serialize signal cleanup and Drop so
// process termination cannot skip an invalidation already in progress.
struct TimestampState {
    active: bool,
    shutdown: bool,
    cleanup_failed: bool,
}
static TIMESTAMP: std::sync::Mutex<TimestampState> = std::sync::Mutex::new(TimestampState {
    active: false,
    shutdown: false,
    cleanup_failed: false,
});

pub fn begin_privilege_shutdown() {
    TIMESTAMP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .shutdown = true;
}

pub fn check_timestamp_cleanup() -> Result<()> {
    let state = TIMESTAMP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if state.active || state.cleanup_failed {
        bail!("Sudo timestamp cleanup incomplete; selected invocation did not finish cleanly");
    }
    Ok(())
}

pub fn invalidate_active_timestamp() {
    let mut state = TIMESTAMP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !state.active {
        return;
    }
    let cleanup = bossa::SystemRunner::new(crate::paths::environment().variables().clone())
        .with_limits(bossa::ProcessLimits {
            timeout: std::time::Duration::from_secs(2),
            max_output_bytes: 4096,
        });
    if let Err(error) =
        crate::runner::capture_using(&cleanup, "sudo", ["-k"], IoMode::ForegroundCapture).and_then(
            |output| {
                if output.status.success() {
                    Ok(output)
                } else {
                    bail!("sudo timestamp invalidation failed: {}", output.status)
                }
            },
        )
    {
        eprintln!("Sudo timestamp cleanup incomplete: {error}");
        state.cleanup_failed = true;
    }
    state.active = false;
}

impl Drop for SudoContext {
    fn drop(&mut self) {
        // A fresh, bounded cancellation domain performs only invalidation.
        invalidate_active_timestamp();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs only in an isolated copy of this test executable with a fake PATH.
    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn native_scope_fixture_child() {
        let Ok(mode) = std::env::var("BOSSA_TEST_PRIV_CHILD") else {
            return;
        };
        let fixture = std::env::var_os("BOSSA_TEST_SUDO_FIXTURE")
            .map(std::path::PathBuf::from)
            .expect("owned fake sudo path required");
        assert_eq!(
            std::env::var_os("PATH").as_deref(),
            Some(fixture.parent().unwrap().as_os_str())
        );
        assert_eq!(
            std::fs::read_to_string(&fixture).unwrap(),
            include_str!("../tests/fixtures/sudo-state.sh")
        );
        let _signals = crate::runner::install_signal_handler().unwrap();
        let context = SudoContext::acquire("owned fake privilege fixture");
        if mode != "acquire-failure" && mode != "signal" {
            assert!(context.is_ok());
        }
        if mode == "acquire-failure" {
            assert!(context.is_err());
        } else if let Ok(context) = context {
            assert!(SudoContext::acquire("nested scope is forbidden").is_err());
            let result = context.run("fixture-command", &["argument"]);
            if mode != "signal" {
                assert!(result.unwrap().success);
            }
            drop(context);
        }
        begin_privilege_shutdown();
        assert!(SudoContext::acquire("shutdown forbids another scope").is_err());
        if mode == "cleanup-failure" {
            assert!(check_timestamp_cleanup().is_err());
        } else {
            assert!(check_timestamp_cleanup().is_ok());
        }
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn native_scope_cleanup_uses_only_owned_child_fixtures() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        for (mode, gate, signal) in [
            ("normal", "", ""),
            ("acquire-failure", "", ""),
            ("cleanup-failure", "", ""),
            ("signal", "-v", "-INT"),
            ("signal", "-n", "-TERM"),
            ("signal", "-k", "-HUP"),
        ] {
            let temp = tempfile::TempDir::new().unwrap();
            let root = temp.path();
            let bin = root.join("bin");
            let home = root.join("home");
            std::fs::create_dir(&bin).unwrap();
            std::fs::create_dir(&home).unwrap();
            let sudo = bin.join("sudo");
            std::fs::write(&sudo, include_str!("../tests/fixtures/sudo-state.sh")).unwrap();
            std::fs::set_permissions(sudo, std::fs::Permissions::from_mode(0o755)).unwrap();
            let trace = root.join("trace");
            let ready = root.join("ready");
            let log = root.join("log");
            let mut child = ChildGuard(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "sudo::tests::native_scope_fixture_child",
                        "--nocapture",
                    ])
                    .current_dir(root)
                    .env_clear()
                    .env("HOME", &home)
                    .env("PATH", &bin)
                    .env("BOSSA_TEST_PRIV_CHILD", mode)
                    .env("BOSSA_TEST_SUDO_FIXTURE", bin.join("sudo"))
                    .env("BOSSA_TEST_SUDO_GATE", gate)
                    .env("BOSSA_TEST_SUDO_READY", &ready)
                    .env("BOSSA_TEST_SUDO_TRACE", &trace)
                    .env(
                        "BOSSA_TEST_SUDO_ACQUIRE_STATUS",
                        if mode == "acquire-failure" { "1" } else { "0" },
                    )
                    .env(
                        "BOSSA_TEST_SUDO_CLEANUP_STATUS",
                        if mode == "cleanup-failure" { "1" } else { "0" },
                    )
                    .stdin(Stdio::null())
                    .stdout(std::fs::File::create(&log).unwrap())
                    .stderr(std::fs::File::create(root.join("errors")).unwrap())
                    .spawn()
                    .unwrap(),
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            if !signal.is_empty() {
                while !ready.is_file() {
                    assert!(Instant::now() < deadline, "fake sudo gate not reached");
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(
                    Command::new("/bin/kill")
                        .env_clear()
                        .args([signal, &child.0.id().to_string()])
                        .status()
                        .unwrap()
                        .success()
                );
            }
            let status = loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    break status;
                }
                assert!(Instant::now() < deadline, "fake sudo child did not finish");
                std::thread::sleep(Duration::from_millis(5));
            };
            let actual = std::fs::read_to_string(&trace).unwrap();
            let expected = if mode == "acquire-failure" || gate == "-v" {
                "-v\n-k\n"
            } else {
                "-v\n-n -- fixture-command argument\n-k\n"
            };
            assert_eq!(actual, expected, "{mode}/{gate}");
            assert!(
                actual.lines().any(|line| line == "-k"),
                "{mode}/{gate}: {actual}"
            );
            if signal.is_empty() {
                assert!(
                    status.success(),
                    "{mode}: {}",
                    std::fs::read_to_string(root.join("errors")).unwrap()
                );
            } else {
                assert!(!status.success());
            }
            if gate == "-v" || mode == "acquire-failure" {
                assert!(!actual.contains("-n --"));
            }
        }
    }

    #[test]
    fn test_sudo_config_defaults() {
        let config = SudoConfig::default();
        assert_eq!(config.casks.len(), 0);
        assert_eq!(config.defaults.len(), 0);
        assert_eq!(config.operations.len(), 0);
    }

    #[test]
    fn test_cask_requires_sudo() {
        let config = SudoConfig {
            casks: vec!["docker".to_string(), "1password".to_string()],
            ..Default::default()
        };
        assert!(config.cask_requires_sudo("docker"));
        assert!(config.cask_requires_sudo("1password"));
        assert!(!config.cask_requires_sudo("raycast"));
    }

    #[test]
    fn test_sudo_classifier() {
        let config = SudoConfig {
            casks: vec!["docker".to_string()],
            defaults: vec!["com.apple.system".to_string()],
            ..Default::default()
        };

        assert!(config.requires_sudo("brew_cask", "docker"));
        assert!(!config.requires_sudo("brew_cask", "raycast"));
        assert!(config.requires_sudo("macos_default", "com.apple.system"));
        assert!(!config.requires_sudo("brew_formula", "ripgrep"));
    }
}
