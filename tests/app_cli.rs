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
        self.command_after_spawn(args, overrides, |_| Ok(()))
    }

    fn command_after_spawn(
        &self,
        args: &[&str],
        overrides: &[(&str, Option<&str>)],
        started: impl FnOnce(u32) -> io::Result<()>,
    ) -> io::Result<Output> {
        let root = self.root.path();
        // File-backed output avoids pipe deadlock while the parent enforces a
        // deadline. Process-supervision cases use finite, owned child fixtures.
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
        if let Err(error) = started(child.0.id()) {
            let stdout = fs::read(root.join("stdout"))?;
            let stderr = fs::read(root.join("stderr"))?;
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "{error}; fixture stdout: {}; fixture stderr: {}",
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr)
                ),
            ));
        }
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

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn termination_signals_cancel_native_followup_jobs_and_preserve_prior_effects() {
    for (signal, status) in [("-INT", 130), ("-TERM", 143)] {
        let fixture = Fixture::new();
        let root = fixture.root.path();
        fixture.executable("defaults", include_str!("fixtures/defaults-state.sh"));
        fixture.executable("killall", include_str!("fixtures/process-tree.sh"));
        let contents =
            "[defaults]\n'com.example.Enabled' = true\n[defaults.restart]\nservices = ['pids']\n";
        fs::write(root.join("config/config.toml"), contents).unwrap();
        let state = root.join("observed-default");
        let pids = root.join("pids");
        let output = fixture
            .command_after_spawn(
                &["nova", "--only", "defaults", "--yes"],
                &[("BOSSA_TEST_DEFAULT_STATE", Some(state.to_str().unwrap()))],
                |pid| {
                    // Startup includes repeated native observations. Give the
                    // fixture scheduler headroom; cancellation is checked below.
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while !fs::read_to_string(&pids)
                        .is_ok_and(|contents| contents.lines().count() == 2)
                    {
                        if Instant::now() >= deadline {
                            return Err(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "fixture follow-up did not start",
                            ));
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    // This ID comes directly from our unreaped child handle.
                    let result = Command::new("/bin/kill")
                        .args([signal, &pid.to_string()])
                        .status()?;
                    assert!(result.success());
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(output.status.code(), Some(status));
        assert!(state.is_file(), "the earlier verified effect was lost");
        assert_eq!(
            fs::read_to_string(root.join("config/config.toml")).unwrap(),
            contents
        );
        assert_eq!(fs::read_dir(root.join("state")).unwrap().count(), 0);
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("Selected postconditions verified")
        );
        for pid in fs::read_to_string(&pids).unwrap().lines() {
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                let observed = Command::new("/bin/ps")
                    .env_clear()
                    .args(["-o", "stat=", "-p", pid])
                    .output()
                    .unwrap();
                let state = String::from_utf8(observed.stdout).unwrap();
                if (observed.status.code() == Some(1)
                    && state.is_empty()
                    && observed.stderr.is_empty())
                    || (observed.status.success() && state.trim_start().starts_with('Z'))
                {
                    break;
                }
                assert!(observed.status.success(), "process observation failed");
                assert!(
                    Instant::now() < deadline,
                    "fixture child remained live: {pid}: {state}"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn failed_native_observations_never_become_successful_previews() {
    let mut hidden = Vec::new();
    for (name, stage, program, contents) in [
        (
            "defaults",
            "defaults",
            "defaults",
            "[defaults]\n'com.example.Value' = true\n",
        ),
        (
            "brew",
            "packages",
            "brew",
            "[packages.brew]\nformulas = ['git']\n",
        ),
        (
            "pnpm",
            "ecosystem",
            "pnpm",
            "[packages.pnpm]\nglobals = ['typescript']\n",
        ),
        (
            "gh",
            "ecosystem",
            "gh",
            "[packages.gh]\nextensions = ['owner/gh-tool']\n",
        ),
        (
            "code",
            "ecosystem",
            "code",
            "[packages.vscode]\nextensions = ['publisher.extension']\n",
        ),
        (
            "handler",
            "handlers",
            "duti",
            "[handlers]\n'com.example.App' = ['public.text']\n",
        ),
        ("dock", "dock", "defaults", ""),
    ] {
        let fixture = Fixture::new();
        let root = fixture.root.path();
        let contents = if name == "dock" {
            let app = root.join("Apps/Fixture.app");
            fs::create_dir_all(&app).unwrap();
            format!("[dock]\napps = ['{}']\n", app.display())
        } else {
            contents.into()
        };
        fs::write(root.join("config/config.toml"), &contents).unwrap();
        fixture.executable(program, include_str!("fixtures/failed-command.sh"));
        if name == "dock" {
            fixture.executable("killall", include_str!("fixtures/failed-command.sh"));
        }
        let trace = root.join("native-trace");
        let output = fixture
            .command_with_env(
                &["nova", "--only", stage, "--dry-run", "--yes"],
                &[("BOSSA_TEST_TRACE", Some(trace.to_str().unwrap()))],
            )
            .unwrap();
        if output.status.success() {
            hidden.push(name);
        }
        assert!(trace.is_file(), "{name}: observation was not exercised");
        assert_eq!(
            fs::read_to_string(root.join("config/config.toml")).unwrap(),
            contents
        );
        for directory in ["home", "state", "workspaces"] {
            assert_eq!(fs::read_dir(root.join(directory)).unwrap().count(), 0);
        }
    }
    assert_eq!(
        hidden,
        Vec::<&str>::new(),
        "failed observations disappeared"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn native_inventory_decode_errors_and_unverified_effects_fail_closed() {
    for (stage, program, contents, absent, malformed) in [
        (
            "defaults",
            "defaults",
            "[defaults]\n'com.example.Enabled' = true\n",
            "<plist><dict/></plist>",
            "<plist><array/></plist>",
        ),
        (
            "ecosystem",
            "pnpm",
            "[packages.pnpm]\nglobals = ['typescript']\n",
            "[]",
            "{}",
        ),
        (
            "ecosystem",
            "gh",
            "[packages.gh]\nextensions = ['owner/gh-tool']\n",
            "",
            "unsupported inventory format",
        ),
        (
            "ecosystem",
            "code",
            "[packages.vscode]\nextensions = ['publisher.extension']\n",
            "",
            "not an extension",
        ),
        (
            "handlers",
            "duti",
            "[handlers]\n'com.example.App' = ['public.text']\n",
            "com.example.Other\n",
            "",
        ),
    ] {
        let fixture = Fixture::new();
        let root = fixture.root.path();
        fs::write(root.join("config/config.toml"), contents).unwrap();
        fixture.executable(program, include_str!("fixtures/native-snapshot.sh"));
        let trace = root.join("trace");
        let response = root.join("response");
        let overrides = [
            ("BOSSA_TEST_TRACE", Some(trace.to_str().unwrap())),
            ("BOSSA_TEST_RESPONSE", Some(response.to_str().unwrap())),
        ];
        fs::write(&response, malformed).unwrap();
        assert!(
            !fixture
                .command_with_env(&["nova", "--only", stage, "--dry-run", "--yes"], &overrides)
                .unwrap()
                .status
                .success(),
            "{program}: invalid observation accepted"
        );
        fs::write(&trace, "").unwrap();
        fs::write(&response, absent).unwrap();
        assert!(
            fixture
                .command_with_env(&["nova", "--only", stage, "--dry-run", "--yes"], &overrides)
                .unwrap()
                .status
                .success(),
            "{program}: supported absence rejected"
        );
        let preview = fs::read_to_string(&trace).unwrap();
        let effect = match program {
            "defaults" => "write ",
            "brew" => "install ",
            "pnpm" => "add ",
            "gh" => "extension install ",
            "code" => "--install-extension ",
            "duti" => "-s ",
            _ => unreachable!(),
        };
        assert!(
            !preview.lines().any(|line| line.starts_with(effect)),
            "{preview}"
        );
        let output = fixture
            .command_with_env(&["nova", "--only", stage, "--yes"], &overrides)
            .unwrap();
        assert!(
            !output.status.success(),
            "{program}: successful command falsely proved its effect"
        );
        let trace = fs::read_to_string(&trace).unwrap();
        assert!(
            trace.lines().any(|line| line.starts_with(effect)),
            "{program}: effect was not exercised: {trace}"
        );
        assert_eq!(
            fs::read_to_string(root.join("config/config.toml")).unwrap(),
            contents
        );
        for directory in ["home", "state", "workspaces"] {
            assert_eq!(fs::read_dir(root.join(directory)).unwrap().count(), 0);
        }
    }
}

#[test]
#[cfg(unix)]
fn symlink_preflight_rejects_missing_packages_and_preserves_conflicts() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let source = root.join("source");
    let config = format!(
        "[symlinks]\nsource = '{}'\ntarget = '{}'\npackages = ['shell']\n",
        source.display(),
        root.join("home").display()
    );
    fs::write(root.join("config/config.toml"), config).unwrap();
    assert!(
        !fixture
            .command(&["nova", "--only", "symlinks", "--dry-run"])
            .unwrap()
            .status
            .success()
    );
    fs::create_dir_all(source.join("shell")).unwrap();
    fs::write(source.join("shell/a-new"), "new").unwrap();
    fs::write(source.join("shell/z-existing"), "desired").unwrap();
    let target = root.join("home/z-existing");
    symlink(root.join("retained-missing-source"), &target).unwrap();
    assert!(
        !fixture
            .command(&["nova", "--only", "symlinks", "--yes"])
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        fs::read_link(target).unwrap(),
        root.join("retained-missing-source")
    );
    assert!(!root.join("home/a-new").exists());
    fs::remove_file(root.join("home/z-existing")).unwrap();
    fs::remove_file(source.join("shell/z-existing")).unwrap();
    fs::create_dir(source.join("shell/z-parent")).unwrap();
    fs::write(source.join("shell/z-parent/file"), "nested").unwrap();
    symlink(root.join("missing-directory"), root.join("home/z-parent")).unwrap();
    assert!(
        !fixture
            .command(&["nova", "--only", "symlinks", "--yes"])
            .unwrap()
            .status
            .success()
    );
    assert!(!root.join("home/a-new").exists());
    assert_eq!(
        fs::read_link(root.join("home/z-parent")).unwrap(),
        root.join("missing-directory")
    );
}

#[test]
#[cfg(unix)]
fn discovered_symlink_names_remain_literal_native_paths() {
    #[cfg(target_os = "linux")]
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let source = root.join("source");
    fs::create_dir_all(source.join("shell")).unwrap();
    let names = vec![std::ffi::OsString::from("literal-$HOME")];
    #[cfg(target_os = "linux")]
    let names = {
        let mut names = names;
        names.push(std::ffi::OsString::from_vec(vec![b'n', 0xff]));
        names
    };
    for name in &names {
        fs::write(source.join("shell").join(name), "fixture").unwrap();
    }
    fs::write(
        root.join("config/config.toml"),
        format!(
            "[symlinks]\nsource = '{}'\ntarget = '{}'\npackages = ['shell']\n",
            source.display(),
            root.join("home").display()
        ),
    )
    .unwrap();
    for _ in 0..2 {
        let output = fixture
            .command(&["nova", "--only", "symlinks", "--yes"])
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for name in &names {
            assert_eq!(
                fs::read_link(root.join("home").join(name)).unwrap(),
                source.join("shell").join(name)
            );
        }
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn dock_layout_is_one_verified_action_and_reapplication_is_read_only() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let apps = root.join("Apps");
    let snapshots = root.join("snapshots");
    fs::create_dir(&snapshots).unwrap();
    for name in ["A", "B", "C"] {
        fs::create_dir_all(apps.join(format!("{name}.app"))).unwrap();
    }
    let mut config = String::from("[dock]\napps = [");
    for name in ["A", "B", "C"] {
        config.push_str(&format!(
            "'{}',",
            apps.join(format!("{name}.app")).display()
        ));
    }
    config.push_str("]\n");
    fs::write(root.join("config/config.toml"), config).unwrap();
    for (stage, names) in [
        (0, ["C", "B", "A"]),
        (1, ["A", "C", "B"]),
        (2, ["A", "B", "C"]),
    ] {
        let mut xml = String::from(
            "<plist><dict><key>persistent-others</key><array/><key>persistent-apps</key><array>",
        );
        for name in names {
            xml.push_str(&format!("<dict><key>tile-data</key><dict><key>file-label</key><string>{name}</string><key>file-data</key><dict><key>_CFURLString</key><string>{}</string><key>_CFURLStringType</key><integer>0</integer></dict></dict></dict>", apps.join(format!("{name}.app")).display()));
        }
        xml.push_str("</array></dict></plist>");
        fs::write(snapshots.join(stage.to_string()), xml).unwrap();
    }
    for program in ["defaults", "dockutil", "killall"] {
        fixture.executable(program, include_str!("fixtures/dock-state.sh"));
    }
    let trace = root.join("trace");
    let state = root.join("dock-state");
    let overrides = [
        ("BOSSA_TEST_TRACE", Some(trace.to_str().unwrap())),
        ("BOSSA_TEST_DOCK_STATE", Some(state.to_str().unwrap())),
        (
            "BOSSA_TEST_DOCK_SNAPSHOTS",
            Some(snapshots.to_str().unwrap()),
        ),
    ];
    let output = fixture
        .command_with_env(&["nova", "--only", "dock", "--yes"], &overrides)
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first = fs::read_to_string(&trace).unwrap();
    assert_eq!(
        first
            .lines()
            .filter(|line| line.starts_with("--move"))
            .count(),
        2
    );
    assert_eq!(first.lines().filter(|line| *line == "Dock").count(), 1);
    fs::write(&trace, "").unwrap();
    assert!(
        fixture
            .command_with_env(&["nova", "--only", "dock", "--yes"], &overrides)
            .unwrap()
            .status
            .success()
    );
    let repeat = fs::read_to_string(trace).unwrap();
    assert!(
        repeat.lines().all(|line| line == "export com.apple.dock -"),
        "{repeat}"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn dock_folder_updates_use_the_exact_preflight_label_and_declared_section() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let folder = root.join("Foo.app");
    fs::create_dir(&folder).unwrap();
    fs::write(
        root.join("config/config.toml"),
        format!("[[dock.folders]]\npath = '{}'\n", folder.display()),
    )
    .unwrap();
    for program in ["defaults", "dockutil", "killall"] {
        fixture.executable(program, include_str!("fixtures/native-snapshot.sh"));
    }
    let trace = root.join("trace");
    let response = root.join("response");
    let overrides = [
        ("BOSSA_TEST_TRACE", Some(trace.to_str().unwrap())),
        ("BOSSA_TEST_RESPONSE", Some(response.to_str().unwrap())),
    ];
    for (others, label) in [
        (String::new(), "Foo.app"),
        (
            format!(
                "<dict><key>tile-data</key><dict><key>file-label</key><string>Custom Label</string><key>file-data</key><dict><key>_CFURLString</key><string>{}</string><key>_CFURLStringType</key><integer>0</integer></dict><key>showas</key><integer>3</integer><key>displayas</key><integer>0</integer><key>arrangement</key><integer>2</integer></dict></dict>",
                folder.display()
            ),
            "Custom Label",
        ),
    ] {
        fs::write(&response, format!("<plist><dict><key>persistent-apps</key><array><dict><key>tile-data</key><dict><key>file-label</key><string>Foo</string></dict></dict></array><key>persistent-others</key><array>{others}</array></dict></plist>")).unwrap();
        fs::write(&trace, "").unwrap();
        assert!(
            !fixture
                .command_with_env(&["nova", "--only", "dock", "--yes"], &overrides)
                .unwrap()
                .status
                .success()
        ); // fake effect leaves the snapshot unchanged.
        let trace = fs::read_to_string(&trace).unwrap();
        let effect = trace
            .lines()
            .find(|line| line.starts_with("--add "))
            .unwrap();
        assert!(
            effect.contains("--section others --type directory"),
            "{effect}"
        );
        assert!(
            effect.contains(&format!("--label {label} --view")),
            "{effect}"
        );
        if label == "Custom Label" {
            assert!(effect.ends_with("--replacing Custom Label"), "{effect}");
        }
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unsupported_native_elevation_is_blocked_before_any_effect() {
    for contents in [
        "[packages.brew]\nfonts = ['fixture-font']\n[sudo]\ncasks = ['fixture-font']\n",
        "[defaults]\n'com.example.Enabled' = true\n[sudo]\ndefaults = ['com.example.Enabled']\n",
    ] {
        let fixture = Fixture::new();
        let root = fixture.root.path();
        fs::write(root.join("config/config.toml"), contents).unwrap();
        for program in ["brew", "defaults", "sudo"] {
            fixture.executable(program, include_str!("fixtures/failed-command.sh"));
        }
        // A version-only prerequisite has no install or privilege effects.
        fixture.executable("brew", include_str!("fixtures/native-snapshot.sh"));
        let trace = root.join("trace");
        let output = fixture
            .command_with_env(
                &["nova", "--only", "packages,defaults", "--yes"],
                &[("BOSSA_TEST_TRACE", Some(trace.to_str().unwrap()))],
            )
            .unwrap();
        assert!(!output.status.success());
        let trace = fs::read_to_string(trace).unwrap_or_default();
        assert!(trace.lines().all(|line| line == "--version"), "{trace}");
        assert_eq!(fs::read_dir(root.join("home")).unwrap().count(), 0);
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn homebrew_reads_receipts_without_metadata_commands_or_unverified_success() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let brew = root.join("brew");
    fs::create_dir_all(brew.join("Cellar")).unwrap();
    fs::create_dir_all(brew.join("Caskroom")).unwrap();
    fs::create_dir_all(brew.join("repo/Library/Taps/homebrew/homebrew-core/Formula")).unwrap();
    fs::write(
        brew.join("repo/Library/Taps/homebrew/homebrew-core/Formula/git.rb"),
        "fixture metadata only",
    )
    .unwrap();
    fs::write(
        root.join("config/config.toml"),
        "[packages.brew]\nformulas = ['git']\n",
    )
    .unwrap();
    fixture.executable("brew", include_str!("fixtures/brew-state.sh"));
    let trace = root.join("trace");
    let overrides = [
        ("BOSSA_TEST_TRACE", Some(trace.to_str().unwrap())),
        ("BOSSA_TEST_BREW_ROOT", Some(brew.to_str().unwrap())),
    ];
    assert!(
        fixture
            .command_with_env(&["nova", "--only", "packages", "--dry-run"], &overrides)
            .unwrap()
            .status
            .success()
    );
    assert!(
        fs::read_to_string(&trace)
            .unwrap()
            .lines()
            .all(|line| line.starts_with("--"))
    );
    assert!(
        !fixture
            .command_with_env(&["nova", "--only", "packages", "--yes"], &overrides)
            .unwrap()
            .status
            .success()
    );
    assert!(
        fs::read_to_string(&trace)
            .unwrap()
            .contains("install --formula git")
    );
    fs::create_dir_all(brew.join("Cellar/git/1")).unwrap();
    let receipt = brew.join("Cellar/git/1/INSTALL_RECEIPT.json");
    fs::write(&receipt, r#"{"source":{"tap":"homebrew/core"}}"#).unwrap();
    fs::write(&trace, "").unwrap();
    assert!(
        fixture
            .command_with_env(&["nova", "--only", "packages", "--yes"], &overrides)
            .unwrap()
            .status
            .success()
    );
    assert!(
        fs::read_to_string(&trace)
            .unwrap()
            .lines()
            .all(|line| line.starts_with("--"))
    );
    for invalid in [
        "{}",
        r#"{"source":{"tap":"homebrew/core","tap":"other/tap"}}"#,
    ] {
        fs::write(&receipt, invalid).unwrap();
        fs::write(&trace, "").unwrap();
        assert!(
            !fixture
                .command_with_env(&["nova", "--only", "packages", "--yes"], &overrides)
                .unwrap()
                .status
                .success()
        );
        assert!(!fs::read_to_string(&trace).unwrap().contains("install"));
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn global_package_observation_and_effect_keep_the_same_manager_selection() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    fs::write(
        root.join("config/config.toml"),
        "[packages.pnpm]\nglobals = ['typescript']\n",
    )
    .unwrap();
    fs::write(
        root.join("home/package.json"),
        r#"{"packageManager":"pnpm@fixture"}"#,
    )
    .unwrap();
    fixture.executable("pnpm", include_str!("fixtures/native-snapshot.sh"));
    let trace = root.join("trace");
    let response = root.join("response");
    let environment = root.join("environment");
    fs::write(&response, "[]").unwrap();
    let overrides = [
        ("BOSSA_TEST_TRACE", Some(trace.to_str().unwrap())),
        ("BOSSA_TEST_RESPONSE", Some(response.to_str().unwrap())),
        ("BOSSA_TEST_ENV_TRACE", Some(environment.to_str().unwrap())),
        ("COREPACK_ENABLE_PROJECT_SPEC", Some("1")),
        ("COREPACK_ENABLE_AUTO_PIN", Some("1")),
        ("COREPACK_ENABLE_NETWORK", Some("1")),
    ];
    assert!(
        !fixture
            .command_with_env(&["nova", "--only", "ecosystem", "--yes"], &overrides)
            .unwrap()
            .status
            .success()
    );
    let env = fs::read_to_string(environment).unwrap();
    assert!(
        env.lines()
            .any(|line| line.starts_with("add -g typescript|C|0|0|1|")),
        "{env}"
    );
    assert!(
        env.lines()
            .any(|line| line.starts_with("list --global --depth=0 --json|C|0|0|0|")),
        "{env}"
    );
    for line in env.lines() {
        assert_eq!(
            std::path::Path::new(line.rsplit('|').next().unwrap())
                .canonicalize()
                .unwrap(),
            root.join("home").canonicalize().unwrap()
        );
    }
    assert_eq!(
        fs::read_to_string(root.join("home/package.json")).unwrap(),
        r#"{"packageManager":"pnpm@fixture"}"#
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn missing_dock_and_restart_capabilities_block_before_other_selected_effects() {
    let fixture = Fixture::new();
    let root = fixture.root.path();
    let app = root.join("Apps/Fixture.app");
    fs::create_dir_all(&app).unwrap();
    fs::write(
        root.join("config/config.toml"),
        format!(
            "[defaults]\n'com.example.Enabled' = true\n[dock]\napps = ['{}']\n",
            app.display()
        ),
    )
    .unwrap();
    fixture.executable("defaults", include_str!("fixtures/native-snapshot.sh"));
    let trace = root.join("trace");
    let response = root.join("response");
    let overrides = [
        ("BOSSA_TEST_TRACE", Some(trace.to_str().unwrap())),
        ("BOSSA_TEST_RESPONSE", Some(response.to_str().unwrap())),
    ];
    fs::write(&response, "<plist><dict><key>Enabled</key><false/><key>persistent-apps</key><array/><key>persistent-others</key><array/></dict></plist>").unwrap();
    fixture.executable("killall", include_str!("fixtures/native-snapshot.sh"));
    for capability in ["dockutil", "killall"] {
        let output = fixture
            .command_with_env(&["nova", "--only", "defaults,dock", "--yes"], &overrides)
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(capability));
        assert!(
            fs::read_to_string(&trace)
                .unwrap()
                .lines()
                .all(|line| line.starts_with("export "))
        );
        if capability == "dockutil" {
            fixture.executable("dockutil", include_str!("fixtures/native-snapshot.sh"));
            fs::remove_file(root.join("bin/killall")).unwrap();
        }
    }
    fs::remove_file(root.join("bin/dockutil")).unwrap();
    fs::write(&response, format!("<plist><dict><key>Enabled</key><true/><key>persistent-apps</key><array><dict><key>tile-data</key><dict><key>file-label</key><string>Fixture</string><key>file-data</key><dict><key>_CFURLString</key><string>{}</string><key>_CFURLStringType</key><integer>0</integer></dict></dict></dict></array><key>persistent-others</key><array/></dict></plist>", app.display())).unwrap();
    // Configured follow-ups require their executable even when currently
    // unchanged; this cannot race a second snapshot that needs an effect.
    assert!(
        !fixture
            .command_with_env(&["nova", "--only", "defaults,dock", "--yes"], &overrides)
            .unwrap()
            .status
            .success()
    );
    fixture.executable("killall", include_str!("fixtures/native-snapshot.sh"));
    assert!(
        fixture
            .command_with_env(&["nova", "--only", "defaults,dock", "--yes"], &overrides)
            .unwrap()
            .status
            .success()
    );
}
