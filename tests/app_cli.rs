//! Real dispatch with isolated roots. These fixtures are not an OS sandbox.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn binary() -> PathBuf {
    // Cargo supplies an absolute path; rules_rust also defines this variable,
    // but relative to the test's runfiles workspace. Resolve either before cwd
    // changes for the child.
    let binary = option_env!("CARGO_BIN_EXE_bossa").map_or_else(
        || {
            std::env::var_os("BOSSA_TEST_BINARY")
                .map(PathBuf::from)
                .expect("Cargo or Bazel must supply the executable")
        },
        PathBuf::from,
    );
    if binary.is_absolute() {
        binary
    } else {
        std::env::current_dir().unwrap().join(binary)
    }
}

struct Fixture {
    root: TempDir,
}

struct RunningChild(Child);

impl Drop for RunningChild {
    fn drop(&mut self) {
        // Also covers output/metadata/read errors while the child is running.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Fixture {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        for name in ["home", "config", "state", "workspaces", "bin"] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        Self { root }
    }

    fn command(&self, args: &[&str]) -> io::Result<Output> {
        let root = self.root.path();
        // File-backed output avoids pipe deadlock while the parent enforces a
        // deadline. No selected fixture path spawns background child processes.
        let stdout = fs::File::create(root.join("stdout"))?;
        let stderr = fs::File::create(root.join("stderr"))?;
        let mut command = Command::new(binary());
        command
            .args(args)
            .current_dir(root)
            .env_clear()
            .env("HOME", root.join("home"))
            .env("USERPROFILE", root.join("home"))
            .env("BOSSA_CONFIG_DIR", root.join("config"))
            .env("BOSSA_STATE_DIR", root.join("state"))
            .env("BOSSA_WORKSPACES_DIR", root.join("workspaces"))
            .env("XDG_CONFIG_HOME", root.join("home/.config"))
            .env("XDG_STATE_HOME", root.join("home/.local/state"))
            .env("PATH", root.join("bin"))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr);
        let mut child = RunningChild(command.spawn()?);
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.0.try_wait()? {
                break status;
            }
            let output_size =
                fs::metadata(root.join("stdout"))?.len() + fs::metadata(root.join("stderr"))?.len();
            if Instant::now() >= deadline || output_size >= 64 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "fixture exceeded time or output limit",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        };
        let stdout = fs::read(root.join("stdout"))?;
        let stderr = fs::read(root.join("stderr"))?;
        assert!(stdout.len() + stderr.len() < 64 * 1024);
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
}

#[test]
fn actual_add_collection_dispatch_writes_only_the_selected_config_root() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let config = root.join("config/config.toml");
    let output = fixture
        .command(&["add", "collection", "fixture", "~/collections"])
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document = fs::read_to_string(&config).unwrap();
    assert!(document.contains("[collections.fixture]"));
    assert!(document.contains("~/collections"));
    for name in ["home", "state", "workspaces", "bin"] {
        assert_eq!(fs::read_dir(root.join(name)).unwrap().count(), 0, "{name}");
    }
}

#[test]
fn selected_empty_nova_preview_needs_no_live_tools_or_state() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    fs::write(root.join("config/config.toml"), "[defaults]\n").unwrap();
    let before = fs::read(root.join("config/config.toml")).unwrap();
    let output = fixture
        .command(&["nova", "--only", "defaults", "--dry-run"])
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(root.join("config/config.toml")).unwrap(), before);
    for name in ["home", "state", "workspaces", "bin"] {
        assert_eq!(fs::read_dir(root.join(name)).unwrap().count(), 0, "{name}");
    }
}
