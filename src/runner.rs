#![allow(dead_code)]

use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitStatus};

use bossa::{CommandRequest, IoMode, ProcessRunner, SystemRunner};

/// One captured native environment and cancellation domain per CLI invocation.
pub fn native_runner() -> &'static SystemRunner {
    static RUNNER: std::sync::OnceLock<SystemRunner> = std::sync::OnceLock::new();
    RUNNER.get_or_init(|| SystemRunner::new(crate::paths::environment().variables().clone()))
}

#[cfg(unix)]
pub struct SignalGuard {
    handle: signal_hook::iterator::Handle,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl Drop for SignalGuard {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(unix)]
pub fn install_signal_handler() -> Result<SignalGuard> {
    use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
    let mut signals = signal_hook::iterator::Signals::new([SIGINT, SIGTERM, SIGHUP])?;
    let handle = signals.handle();
    let runner = native_runner().clone();
    let thread = std::thread::Builder::new()
        .name("bossa-signals".into())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                if let Err(error) = runner.cancel() {
                    eprintln!("Native job cancellation incomplete: {error}");
                }
                if !runner.wait_idle(std::time::Duration::from_secs(2)) {
                    eprintln!("Native job cleanup did not finish before termination");
                }
                std::process::exit(128 + signal);
            }
        })?;
    Ok(SignalGuard {
        handle,
        thread: Some(thread),
    })
}

/// Run a command and inherit stdio (shows output in real-time)
pub fn run(cmd: &str, args: &[&str]) -> Result<ExitStatus> {
    Ok(run_native(cmd, args, IoMode::Inherit)?.status)
}

/// Run a command and capture output
pub fn run_capture(cmd: &str, args: &[&str]) -> Result<String> {
    let output = run_native(cmd, args, IoMode::Capture)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Command failed: {}", stderr.trim())
    }
}

fn run_native(cmd: &str, args: &[&str], mode: IoMode) -> Result<std::process::Output> {
    native_runner()
        .run(
            &CommandRequest {
                program: cmd.into(),
                args: args.iter().map(|arg| (*arg).into()).collect(),
                working_dir: env::current_dir().context("Could not determine working directory")?,
            },
            mode,
        )
        .with_context(|| format!("Failed to execute {cmd}"))
}

/// Check if a command exists
pub fn command_exists(cmd: &str) -> bool {
    let cmd_path = Path::new(cmd);

    if cmd_path.components().count() > 1 {
        return command_candidate_exists(cmd_path);
    }

    let Some(path_var) = crate::paths::environment().variable("PATH") else {
        return false;
    };

    env::split_paths(&path_var).any(|dir| command_candidate_exists(&dir.join(cmd)))
}

#[cfg(windows)]
fn command_candidate_exists(candidate: &Path) -> bool {
    if is_executable(candidate) {
        return true;
    }

    if candidate.extension().is_some() {
        return false;
    }

    let pathext = crate::paths::environment()
        .variable("PATHEXT")
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(".COM;.EXE;.BAT;.CMD");

    pathext.split(';').filter(|ext| !ext.is_empty()).any(|ext| {
        let ext = if ext.starts_with('.') {
            ext.to_string()
        } else {
            format!(".{ext}")
        };
        let mut with_ext = candidate.as_os_str().to_os_string();
        with_ext.push(ext);
        is_executable(Path::new(&with_ext))
    })
}

#[cfg(not(windows))]
fn command_candidate_exists(candidate: &Path) -> bool {
    is_executable(candidate)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .map(|metadata| metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0))
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Run a script from the dotfiles bin directory
pub fn run_script(script_name: &str, args: &[&str]) -> Result<ExitStatus> {
    // Scripts are in PATH after stow, so just run by name
    run(script_name, args)
}

/// Get directory disk usage in bytes via `du -sk`.
///
/// Returns the on-disk size (block usage), which matches what Finder/`du` report.
/// Returns 0 if the path doesn't exist or `du` fails to parse.
pub fn dir_size(path: &Path) -> Result<u64> {
    let path_str = path.to_str().context("Path contains invalid UTF-8")?;
    let output = Command::new("du")
        .args(["-sk", path_str])
        .output()
        .context("Failed to run du command")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let size_kb: u64 = stdout
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Ok(size_kb * 1024)
}

#[cfg(test)]
mod tests {
    use super::command_exists;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn command_exists_returns_false_for_missing_absolute_path() {
        let tmp = TempDir::new().expect("temp dir should be created");
        let missing = tmp.path().join("missing-command");
        let missing_str = missing.to_string_lossy().into_owned();

        assert!(!command_exists(&missing_str));
    }

    #[cfg(unix)]
    #[test]
    fn command_exists_detects_executable_absolute_path() {
        let tmp = TempDir::new().expect("temp dir should be created");
        let cmd = tmp.path().join("test-command");
        fs::write(&cmd, "#!/bin/sh\nexit 0\n").expect("script should be written");

        let mut perms = fs::metadata(&cmd)
            .expect("metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&cmd, perms).expect("permissions should be set");

        let cmd_str = cmd.to_string_lossy().into_owned();
        assert!(command_exists(&cmd_str));
    }

    #[cfg(unix)]
    #[test]
    fn command_exists_rejects_non_executable_absolute_path() {
        let tmp = TempDir::new().expect("temp dir should be created");
        let cmd = tmp.path().join("not-executable");
        fs::write(&cmd, "echo hi\n").expect("file should be written");

        let mut perms = fs::metadata(&cmd)
            .expect("metadata should be readable")
            .permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&cmd, perms).expect("permissions should be set");

        let cmd_str = cmd.to_string_lossy().into_owned();
        assert!(!command_exists(&cmd_str));
    }
}
