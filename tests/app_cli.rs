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
        self.command_with_env(args, &[])
    }

    fn command_with_env(
        &self,
        args: &[&str],
        overrides: &[(&str, Option<&str>)],
    ) -> io::Result<Output> {
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
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", root.join("home/unused-gitconfig"))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr);
        for (key, value) in overrides {
            if let Some(value) = value {
                command.env(key, value);
            } else {
                command.env_remove(key);
            }
        }
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

#[test]
fn malformed_configuration_is_an_error_and_preserves_original_bytes() {
    let fixture = Fixture::new();
    let path = fixture.root.path().join("config/config.toml");
    let original = b"[defaults\nbroken";
    fs::write(&path, original).unwrap();
    let output = fixture
        .command(&["nova", "--only", "defaults", "--dry-run"])
        .unwrap();
    assert!(
        !output.status.success(),
        "Malformed input must not become an empty successful preview"
    );
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn crud_and_status_share_the_canonical_collection_shape() {
    let fixture = Fixture::new();
    let added = fixture
        .command(&["add", "collection", "fixture", "~/collections"])
        .unwrap();
    assert!(added.status.success());
    let status = fixture.command(&["status", "collections"]).unwrap();
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("fixture"));
}

#[test]
fn editing_json_configuration_keeps_its_source_and_unrelated_collection() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let path = root.join("config/config.json");
    fs::write(
        &path,
        r#"{"collections":{"original":{"path":"~/original"}}}"#,
    )
    .unwrap();
    let output = fixture
        .command(&["add", "collection", "fixture", "~/collections"])
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!root.join("config/config.toml").exists());
    let json = fs::read_to_string(path).unwrap();
    assert!(json.contains("original"));
    assert!(json.contains("fixture"));
}

#[test]
fn higher_priority_parse_failure_blocks_readers_and_crud_without_fallback() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let original = b"[broken";
    fs::write(root.join("config/config.toml"), original).unwrap();
    fs::write(root.join("config/config.json"), "{}").unwrap();
    for args in [
        vec!["status"],
        vec!["diff"],
        vec!["apply", "--dry-run"],
        vec!["add", "collection", "fixture", "~/collections"],
    ] {
        let result = fixture.command(&args).unwrap();
        assert!(!result.status.success(), "{args:?}");
        assert_eq!(fs::read(root.join("config/config.toml")).unwrap(), original);
        assert_eq!(fs::read(root.join("config/config.json")).unwrap(), b"{}");
    }
}

#[test]
fn missing_configuration_is_unconfigured_for_status_but_not_apply() {
    let fixture = Fixture::new();
    assert!(fixture.command(&["status"]).unwrap().status.success());
    assert!(
        !fixture
            .command(&["apply", "--dry-run"])
            .unwrap()
            .status
            .success()
    );
    assert!(
        !fixture
            .command(&["nova", "--only", "defaults", "--dry-run"])
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        fs::read_dir(fixture.root.path().join("config"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn unknown_fields_are_errors_and_legacy_reads_never_rewrite_or_autoconvert() {
    let fixture = Fixture::new();
    let path = fixture.root.path().join("config/config.json");
    fs::write(&path, r#"{"packges":{}}"#).unwrap();
    assert!(!fixture.command(&["status"]).unwrap().status.success());
    let legacy = br#"{"collections":[{"name":"legacy","path":"~/legacy"}]}"#;
    fs::write(&path, legacy).unwrap();
    let status = fixture.command(&["status", "collections"]).unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(String::from_utf8_lossy(&status.stdout).contains("legacy"));
    assert_eq!(fs::read(&path).unwrap(), legacy);
    assert!(
        !fixture
            .command(&["add", "collection", "new", "~/new"])
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(path).unwrap(), legacy);
}

#[test]
fn duplicate_json_keys_block_editing_without_losing_original_bytes() {
    let fixture = Fixture::new();
    let path = fixture.root.path().join("config/config.json");
    let original = br#"{"collections":{"original":{"path":"~/original"}},"collections":{"later":{"path":"~/later"}}}"#;
    fs::write(&path, original).unwrap();
    assert!(
        !fixture
            .command(&["add", "collection", "added", "~/added"])
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn historical_unused_workspace_defaults_remain_readable_and_editable() {
    let fixture = Fixture::new();
    let path = fixture.root.path().join("config/config.toml");
    let original =
        "[collections.old]\npath = '~/old'\n[workspaces]\nroot = ''\nstructure = ''\nrepos = []\n";
    fs::write(&path, original).unwrap();
    assert!(
        fixture
            .command(&["status", "collections"])
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    let added = fixture
        .command(&["add", "collection", "added", "~/added"])
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let updated = fs::read_to_string(path).unwrap();
    assert!(updated.contains("collections.old"));
    assert!(updated.contains("collections.added"));
    assert!(updated.contains("root = \"\""));
}

#[test]
fn native_toml_values_survive_loading_and_unrelated_edits() {
    let fixture = Fixture::new();
    let path = fixture.root.path().join("config/config.toml");
    fs::write(
        &path,
        "[defaults]\n'com.example.Float' = nan\n[configs.git.core]\nwhen = 1979-05-27T07:32:00Z\n",
    )
    .unwrap();
    let status = fixture.command(&["status"]).unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(
        fixture
            .command(&["add", "collection", "added", "~/added"])
            .unwrap()
            .status
            .success()
    );
    let updated: toml::Value = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(
        updated["defaults"]["com.example.Float"]
            .as_float()
            .unwrap()
            .is_nan()
    );
    assert!(updated["configs"]["git"]["core"]["when"].is_datetime());
}

#[test]
fn invalid_workspace_categories_branches_and_layouts_fail_before_creating_directories() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    for workspace in [
        "root = '~/workspaces'\nstructure = 'unsupported'\n",
        "root = '~/workspaces'\n[[workspaces.repos]]\nname='repo'\nurl='fixture-source'\ncategory='../outside'\n",
        "root = '~/workspaces'\n[[workspaces.repos]]\nname='repo'\nurl='fixture-source'\ncategory='/outside'\n",
        "root = '~/workspaces'\n[[workspaces.repos]]\nname='repo'\nurl='fixture-source'\nworktrees=['../outside']\n",
    ] {
        fs::write(
            root.join("config/config.toml"),
            format!("[workspaces]\n{workspace}"),
        )
        .unwrap();
        let applied = fixture.command(&["apply", "workspaces"]).unwrap();
        assert!(!applied.status.success(), "{workspace}");
        let error = String::from_utf8_lossy(&applied.stderr);
        assert!(
            error.contains("safe relative") || error.contains("Unsupported workspace structure"),
            "{error}"
        );
        assert_eq!(fs::read_dir(root.join("home")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.join("workspaces")).unwrap().count(), 0);
    }
}

#[cfg(unix)]
#[test]
fn existing_directory_symlinks_cannot_redirect_workspace_apply_outside_its_root() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("workspaces/linked")).unwrap();
    fs::write(root.join("config/config.toml"), "[workspaces]\nroot='~/ignored'\n[[workspaces.repos]]\nname='repo'\nurl='fixture-source'\ncategory='linked'\n").unwrap();
    let result = fixture.command(&["apply", "workspaces"]).unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("outside the configured root"));
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn relocation_does_not_replace_malformed_required_state_with_an_empty_state() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let state = root.join("state/state.toml");
    fs::write(&state, "[broken").unwrap();
    let result = fixture
        .command(&["relocate", "~/old", "~/", "--dry-run"])
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(state).unwrap(), b"[broken");
    assert_eq!(fs::read_dir(root.join("home")).unwrap().count(), 0);
}

#[test]
fn migration_never_turns_selected_source_errors_into_an_empty_replacement() {
    for (name, original) in [
        ("config.toml", "[broken"),
        (
            "config.toml",
            "[collections.keep]\npath='~/keep'\n[extension]\nvalue=true\n",
        ),
        (
            "config.json",
            r#"{"collections":{"keep":{"path":"~/keep"}},"collections":{}}"#,
        ),
    ] {
        let fixture = Fixture::new();
        let root = fixture.root.path();
        let path = root.join("config").join(name);
        fs::write(&path, original).unwrap();
        let legacy = br#"{"root_directory":"~/refs","repositories":[]}"#;
        fs::write(root.join("config/refs.json"), legacy).unwrap();
        let result = fixture.command(&["migrate"]).unwrap();
        assert!(!result.status.success());
        assert_eq!(fs::read(path).unwrap(), original.as_bytes());
        assert_eq!(fs::read(root.join("config/refs.json")).unwrap(), legacy);
    }
}

#[test]
fn migration_preview_and_apply_preserve_json_source_and_unrelated_configuration() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let path = root.join("config/config.json");
    let original = br#"{"collections":{"keep":{"path":"~/keep"}}}"#;
    let legacy = br#"{"root_directory":"~/refs","repositories":[]}"#;
    fs::write(&path, original).unwrap();
    fs::write(root.join("config/refs.json"), legacy).unwrap();
    let preview = fixture.command(&["migrate", "--dry-run"]).unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(String::from_utf8_lossy(&preview.stdout).contains("config.json"));
    assert_eq!(fs::read(&path).unwrap(), original);
    let result = fixture.command(&["migrate"]).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let updated = fs::read_to_string(path).unwrap();
    assert!(updated.contains("keep"));
    assert!(updated.contains("refs"));
    assert!(!root.join("config/config.toml").exists());
    assert_eq!(fs::read(root.join("config/refs.json")).unwrap(), legacy);
}

#[test]
fn relative_workspace_roots_execute_at_the_same_absolute_destination_as_preflight() {
    for override_value in [None, Some("relative-override")] {
        let fixture = Fixture::new();
        let root = fixture.root.path();
        fs::write(root.join("config/config.toml"), "[workspaces]\nroot='relative-config'\n[[workspaces.repos]]\nname='repo'\nurl='fixture-source'\ncategory='work'\n").unwrap();
        // PATH intentionally has no Git. The checked parent must be created,
        // then launch must fail; no clone, network, or default-home write occurs.
        let result = fixture
            .command_with_env(
                &["apply", "workspaces"],
                &[("BOSSA_WORKSPACES_DIR", override_value)],
            )
            .unwrap();
        assert!(!result.status.success());
        let selected = override_value.unwrap_or("relative-config");
        assert!(root.join(selected).join("work").is_dir());
        assert!(!root.join("home/dev").exists());
        assert!(!root.join(selected).join(selected).exists());
    }
}

#[cfg(unix)]
#[test]
fn storage_mount_expansion_and_placeholders_match_canonical_intent() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let mount = root.join("home/disk");
    fs::create_dir(&mount).unwrap();
    fs::write(mount.join("content"), "fixture").unwrap();
    for selected in ["~/disk", "$EXTERNAL"] {
        fs::write(root.join("config/config.toml"), format!("[storage.disk]\nmount='{selected}'\ntype='external'\n[[storage.disk.symlinks]]\nfrom='~/link'\nto='{{mount}}/content'\n")).unwrap();
        let result = fixture
            .command_with_env(
                &["apply", "storage"],
                &[("EXTERNAL", Some(mount.to_str().unwrap()))],
            )
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            fs::read_link(root.join("home/link")).unwrap(),
            mount.join("content")
        );
        fs::remove_file(root.join("home/link")).unwrap();
    }
}

#[cfg(unix)]
impl Fixture {
    fn executable(&self, name: &str, contents: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.root.path().join("bin").join(name);
        fs::write(&path, contents).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

// These recording scripts never run a real installer or Git operation. Their
// exact contents are checked with ShellCheck and shfmt during native validation.
#[cfg(unix)]
const RECORD_CURL: &str =
    "#!/bin/sh\nset -eu\nprintf 'installer\\n' >>\"$BOSSA_TEST_TRACE\"\nprintf '%s\\n' ':'\n";
#[cfg(unix)]
const RECORD_GIT: &str =
    "#!/bin/sh\nset -eu\nprintf 'git %s\\n' \"$*\" >>\"$BOSSA_TEST_TRACE\"\nexit 1\n";

#[test]
#[cfg(unix)]
fn nova_homebrew_preview_never_runs_an_installer_even_with_yes() {
    for yes in [true, false] {
        let fixture = Fixture::new();
        let root = fixture.root.path();
        fs::write(root.join("config/config.toml"), "").unwrap();
        fixture.executable("curl", RECORD_CURL);
        let trace = root.join("trace");
        let mut args = vec!["nova", "--only", "homebrew", "--dry-run"];
        if yes {
            args.push("--yes");
        }
        let output = fixture
            .command_with_env(
                &args,
                &[("BOSSA_TEST_TRACE", Some(trace.to_str().unwrap()))],
            )
            .unwrap();
        assert!(!trace.exists(), "preview invoked an installer");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("blocked"));
        assert_eq!(fs::read_dir(root.join("home")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.join("state")).unwrap().count(), 0);
    }
}

#[test]
#[cfg(unix)]
fn nova_dotfiles_preview_never_clones_and_reports_its_blocker() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let contents = format!(
        "[dotfiles]\nrepo = 'fixture-source'\npath = '{}'\n",
        root.join("home/dotfiles").display()
    );
    fs::write(root.join("config/config.toml"), &contents).unwrap();
    fixture.executable("git", RECORD_GIT);
    let trace = root.join("trace");
    let output = fixture
        .command_with_env(
            &["nova", "--only", "dotfiles", "--dry-run", "--yes"],
            &[("BOSSA_TEST_TRACE", Some(trace.to_str().unwrap()))],
        )
        .unwrap();
    assert!(!trace.exists(), "preview invoked Git mutation");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("blocked"));
    assert_eq!(fs::read_dir(root.join("home")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(root.join("state")).unwrap().count(), 0);
    assert_eq!(
        fs::read_to_string(root.join("config/config.toml")).unwrap(),
        contents
    );
}

#[test]
fn nova_cellar_preview_reports_blocked_without_creating_staging_directories() {
    for yes in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root.path();
        fs::write(
            root.join("config/config.toml"),
            format!("[cellar]\npath = '{}'\n", root.join("warehouse").display()),
        )
        .unwrap();
        let mut args = vec!["nova", "--only", "cellar", "--dry-run"];
        if yes {
            args.push("--yes");
        }
        let output = fixture.command(&args).unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Nova blocked"));
        assert!(!root.join("warehouse").exists());
        assert_eq!(fs::read_dir(root.join("state")).unwrap().count(), 0);
    }
}

#[test]
fn nova_rejects_unsupported_values_and_unknown_stage_selections() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    fs::write(
        root.join("config/config.toml"),
        "[defaults]\n'com.example.Array' = [1, 2]\n",
    )
    .unwrap();
    for args in [
        vec!["nova", "--only", "defaults", "--dry-run", "--yes"],
        vec!["nova", "--only", "typo", "--dry-run"],
        vec!["nova", "--skip", "typo", "--dry-run"],
        vec![
            "nova",
            "--only",
            "defaults",
            "--skip",
            "homebrew",
            "--dry-run",
        ],
    ] {
        let output = fixture.command(&args).unwrap();
        assert!(!output.status.success(), "{args:?}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("already configured"));
        assert_eq!(fs::read_dir(root.join("state")).unwrap().count(), 0);
    }
    // Unsupported values in a different scope do not block an empty symlink preview.
    assert!(
        fixture
            .command(&["nova", "--only", "symlinks", "--dry-run"])
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn nova_observation_failure_is_unsuccessful_without_any_mutation() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let contents = "[defaults]\n'com.example.Value' = true\n";
    fs::write(root.join("config/config.toml"), contents).unwrap();
    let output = fixture
        .command(&["nova", "--only", "defaults", "--dry-run", "--yes"])
        .unwrap();
    assert!(
        !output.status.success(),
        "failed defaults observation disappeared"
    );
    assert_eq!(
        fs::read_to_string(root.join("config/config.toml")).unwrap(),
        contents
    );
    for name in ["state", "home", "workspaces"] {
        assert_eq!(fs::read_dir(root.join(name)).unwrap().count(), 0);
    }
}

#[test]
#[cfg(unix)]
fn quiet_nova_needs_consent_and_verified_fixture_apply_is_idempotent() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let source = root.join("dot-source/shell");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join(".fixture-rc"), "preserved content").unwrap();
    fs::write(
        root.join("config/config.toml"),
        format!(
            "[symlinks]\nsource = '{}'\ntarget = '{}'\npackages = ['shell']\n",
            root.join("dot-source").display(),
            root.join("home").display()
        ),
    )
    .unwrap();
    let target = root.join("home/.fixture-rc");
    let declined = fixture
        .command(&["--quiet", "nova", "--only", "symlinks"])
        .unwrap();
    assert!(!declined.status.success());
    assert!(!target.exists());
    assert_eq!(declined.stdout, b"");
    let preview = fixture
        .command(&[
            "--quiet",
            "nova",
            "--only",
            "symlinks",
            "--dry-run",
            "--yes",
        ])
        .unwrap();
    assert!(preview.status.success());
    assert!(!target.exists());
    assert_eq!(preview.stdout, b"");
    for _ in 0..2 {
        let applied = fixture
            .command(&["--quiet", "nova", "--only", "symlinks", "--yes"])
            .unwrap();
        assert!(
            applied.status.success(),
            "{}",
            String::from_utf8_lossy(&applied.stderr)
        );
        assert_eq!(applied.stdout, b"");
        assert_eq!(fs::read_link(&target).unwrap(), source.join(".fixture-rc"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "preserved content");
        assert_eq!(fs::read_dir(root.join("state")).unwrap().count(), 0);
    }
}
