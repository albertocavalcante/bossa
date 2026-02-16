#![allow(dead_code)]

use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

/// Run a command and inherit stdio (shows output in real-time)
pub fn run(cmd: &str, args: &[&str]) -> Result<ExitStatus> {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("Failed to execute: {} {}", cmd, args.join(" ")))
}

/// Run a command and capture output
pub fn run_capture(cmd: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("Failed to execute: {} {}", cmd, args.join(" ")))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Command failed: {}", stderr.trim())
    }
}

/// Check if a command exists
pub fn command_exists(cmd: &str) -> bool {
    let cmd_path = Path::new(cmd);

    if cmd_path.components().count() > 1 {
        return command_candidate_exists(cmd_path);
    }

    let Some(path_var) = env::var_os("PATH") else {
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

    let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());

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
