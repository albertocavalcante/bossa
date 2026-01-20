use anyhow::{Context, Result};
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

/// Run a command silently, returning success/failure
pub fn run_quiet(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if a command exists
pub fn command_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run a script from the dotfiles bin directory
pub fn run_script(script_name: &str, args: &[&str]) -> Result<ExitStatus> {
    // Scripts are in PATH after stow, so just run by name
    run(script_name, args)
}

/// Get the path to a script (for display purposes)
pub fn script_path(script_name: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        format!("{}/bin/{}", home.display(), script_name)
    } else {
        script_name.to_string()
    }
}
