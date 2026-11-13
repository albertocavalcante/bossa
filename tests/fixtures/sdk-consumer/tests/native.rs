#![cfg(all(feature = "native", unix))]

use anyhow::{Result, bail};
use bossa_sdk::{
    AppEnvironment, ConfigFormat, ResourceApplyOptions, SystemRunner, decode_bossa_config, setup,
};
use declarative::{AutoDecline, CommandOutput, NoProgress, ResourceState, SudoProvider};
use std::{collections::BTreeMap, ffi::OsString, fs, process::Command, sync::Arc};

struct NoSudo;
impl SudoProvider for NoSudo {
    fn run(&self, _: &str, _: &[&str]) -> Result<CommandOutput> {
        bail!("unexpected privilege request")
    }
}

#[test]
fn configured_local_checkout_worktree_and_link() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let home = root.join("home");
    let source = root.join("source");
    fs::create_dir(&home).unwrap();
    fs::create_dir(home.join("mount")).unwrap();
    fs::write(home.join("mount/content"), "fixture").unwrap();
    let path = std::env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .env_clear()
            .env("PATH", &path)
            .env("HOME", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", home.join("no-gitconfig"))
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&[
        "init",
        "--initial-branch=main",
        "--template=",
        source.to_str().unwrap(),
    ]);
    fs::write(source.join("file"), "fixture").unwrap();
    git(&["-C", source.to_str().unwrap(), "add", "file"]);
    git(&["-C", source.to_str().unwrap(), "commit", "-m", "fixture"]);
    git(&["-C", source.to_str().unwrap(), "branch", "dev"]);
    let text = format!(
        "[collections.refs]\npath='~/collection'\n[[collections.refs.repos]]\nname='repo'\nurl='{}'\ndefault_branch='main'\n[workspaces]\nroot='~/workspaces'\n[[workspaces.repos]]\nname='repo'\ncategory='work'\nurl='{}'\nworktrees=['dev']\n[storage.disk]\nmount='~/mount'\ntype='internal'\n[[storage.disk.symlinks]]\nfrom='~/link'\nto='{{mount}}/content'\n",
        source.display(),
        source.display()
    );
    let desired = decode_bossa_config(ConfigFormat::Toml, text.as_bytes())
        .unwrap()
        .desired;
    let vars = BTreeMap::from([
        ("PATH".into(), path),
        ("HOME".into(), home.clone().into_os_string()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        (
            "GIT_CONFIG_GLOBAL".into(),
            home.join("no-gitconfig").into_os_string(),
        ),
    ]);
    let environment = AppEnvironment::new(Some(home.clone()), vars.clone());
    let runner = setup::git_runner(SystemRunner::new(vars));
    let plan = || {
        let selected = setup::select_configured(&desired, None, &environment, &root).unwrap();
        assert_eq!(
            selected.groups,
            ["collections.refs", "storage.disk", "workspaces.repo"]
        );
        setup::plan(
            selected.specs,
            Arc::new(runner.clone()),
            Arc::new(runner.clone()),
            root.clone(),
        )
        .unwrap()
    };
    let preview = plan();
    assert!(setup::observe(&preview).iter().all(|row| row.state.is_ok()));
    let result = setup::apply(
        preview,
        ResourceApplyOptions {
            dry_run: true,
            ..Default::default()
        },
        &runner,
        &root,
        || Ok(NoSudo),
        &mut NoProgress,
        &mut AutoDecline,
    )
    .unwrap();
    assert!(result.is_success());
    assert!(!home.join("collection").exists());
    assert!(!home.join("workspaces").exists());
    assert!(!home.join("link").exists());
    let apply = |plan| {
        setup::apply(
            plan,
            ResourceApplyOptions {
                yes: true,
                ..Default::default()
            },
            &runner,
            &root,
            || Ok(NoSudo),
            &mut NoProgress,
            &mut AutoDecline,
        )
        .unwrap()
    };
    assert!(apply(plan()).is_success());
    assert!(home.join("collection/repo/.git").is_dir());
    assert!(home.join("workspaces/work/repo.git").is_dir());
    assert!(home.join("workspaces/work/repo/dev").is_dir());
    assert!(
        fs::symlink_metadata(home.join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let second = plan();
    assert!(
        setup::observe(&second)
            .iter()
            .all(|row| matches!(&row.state, Ok(ResourceState::Present { .. })))
    );
    let report = apply(second);
    assert!(report.is_success());
    assert_eq!(report.total_changes(), 0);
}
