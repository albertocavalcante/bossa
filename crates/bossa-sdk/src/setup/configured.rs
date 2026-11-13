//! Select and project typed configuration into verified setup resources.
use super::{Checkout, Link, NamedSetup, Setup, Storage, StorageKind, Workspace, Worktree};
use anyhow::{Context as _, Result, ensure};
use std::path::{Path, PathBuf};

pub struct SelectedSetup {
    pub groups: Vec<String>,
    pub specs: Vec<NamedSetup>,
}

fn absolute(path: PathBuf, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

pub fn select_configured(
    config: &crate::schema::BossaConfig,
    target: Option<&str>,
    environment: &crate::AppEnvironment,
    cwd: &Path,
) -> Result<SelectedSetup> {
    // Public callers may construct or mutate desired data without decoding it.
    // Reject unsafe names before any filesystem observation or projection.
    config.validate()?;
    ensure!(
        cwd.is_absolute() && cwd.is_dir(),
        "Setup needs an existing absolute working directory"
    );
    let mut groups: Vec<_> = config
        .collections
        .keys()
        .map(|name| ("collections", name.as_str()))
        .chain(
            config
                .workspaces
                .repos
                .iter()
                .map(|repo| ("workspaces", repo.name.as_str())),
        )
        .chain(config.storage.keys().map(|name| ("storage", name.as_str())))
        .collect();
    groups.sort_unstable();
    let (domain, name) = match target {
        None => (None, None),
        Some(domain @ ("collections" | "workspaces" | "storage")) => (Some(domain), None),
        Some(target) => {
            if let Some((domain @ ("collections" | "workspaces" | "storage"), name)) =
                target.split_once('.')
            {
                ensure!(
                    !name.is_empty() && groups.contains(&(domain, name)),
                    "Unknown setup selector: {target}"
                );
                (Some(domain), Some(name))
            } else {
                let found: Vec<_> = groups.iter().filter(|(_, name)| *name == target).collect();
                ensure!(
                    found.len() == 1,
                    "Unknown or ambiguous setup selector: {target}; use collections.NAME, workspaces.NAME or storage.NAME"
                );
                (Some(found[0].0), Some(found[0].1))
            }
        }
    };
    let matches = |kind, id: &str| domain.is_none_or(|d| d == kind) && name.is_none_or(|n| n == id);
    let names = groups
        .iter()
        .filter(|(kind, id)| matches(*kind, id))
        .map(|(kind, id)| format!("{kind}.{id}"))
        .collect();
    let mut specs = Vec::new();
    let mut collections: Vec<_> = config.collections.iter().collect();
    collections.sort_by_key(|(name, _)| *name);
    for (name, collection) in collections {
        if !matches("collections", name) {
            continue;
        }
        collection.clone.validate()?;
        let root = absolute(environment.expand(&collection.path), cwd);
        for repo in &collection.repos {
            specs.push(NamedSetup {
                name: format!("{name}/{}", repo.name),
                setup: Setup::Checkout(Checkout {
                    url: repo.url.clone(),
                    path: root.join(&repo.name),
                    branch: repo.default_branch.clone(),
                    depth: collection.clone.depth,
                    single_branch: collection.clone.single_branch,
                    options: collection.clone.options.clone(),
                }),
            });
        }
    }
    if config
        .workspaces
        .repos
        .iter()
        .any(|workspace| matches("workspaces", &workspace.name))
    {
        let root = absolute(
            if environment.variable("BOSSA_WORKSPACES_DIR").is_some() {
                environment.workspaces_dir()?
            } else {
                config.workspaces.expanded_root(environment)?
            },
            cwd,
        );
        let physical_root = super::resolve_location(&root)?;
        for workspace in &config.workspaces.repos {
            if !matches("workspaces", &workspace.name) {
                continue;
            }
            let bare = workspace.bare_path(&root);
            let worktrees: Vec<_> = workspace
                .worktrees
                .iter()
                .map(|branch| Worktree {
                    branch: branch.clone(),
                    path: workspace.worktree_path(&root, branch),
                })
                .collect();
            for path in std::iter::once(&bare).chain(worktrees.iter().map(|w| &w.path)) {
                ensure!(
                    super::resolve_location(path)?.starts_with(&physical_root),
                    "Workspace destination resolves outside the configured root: {}",
                    path.display()
                );
            }
            specs.push(NamedSetup {
                name: workspace.name.clone(),
                setup: Setup::Workspace(Workspace {
                    url: workspace.url.clone(),
                    bare,
                    worktrees,
                }),
            });
        }
    }
    let mut storages: Vec<_> = config.storage.iter().collect();
    storages.sort_by_key(|(name, _)| *name);
    for (name, storage) in storages {
        if !matches("storage", name) {
            continue;
        }
        let mount = absolute(storage.expanded_mount(environment)?, cwd);
        let mut links = Vec::new();
        for link in &storage.symlinks {
            let source = if link.to.contains("{mount}") {
                let text = mount
                    .to_str()
                    .context("A storage placeholder requires a UTF-8 mount path")?;
                environment.expand(&link.to.replace("{mount}", text))
            } else {
                environment.expand(&link.to)
            };
            links.push(Link {
                source: absolute(source, cwd),
                target: absolute(environment.expand(&link.from), cwd),
            });
        }
        specs.push(NamedSetup {
            name: name.clone(),
            setup: Setup::Storage(Storage {
                mount,
                links,
                kind: match storage.storage_type {
                    crate::schema::StorageType::Internal => StorageKind::Internal,
                    crate::schema::StorageType::External => StorageKind::External,
                    crate::schema::StorageType::Network => StorageKind::Network,
                },
            }),
        });
    }
    Ok(SelectedSetup {
        groups: names,
        specs,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{ConfigFormat, ResourceApplyOptions, SystemRunner, decode_bossa_config, setup};
    use declarative::{AutoDecline, CommandOutput, NoProgress, SudoProvider};
    use std::{ffi::OsString, fs, process::Command, sync::Arc};

    struct NoSudo;
    impl SudoProvider for NoSudo {
        fn run(&self, _: &str, _: &[&str]) -> Result<CommandOutput> {
            anyhow::bail!("unexpected privilege request")
        }
    }

    #[test]
    fn configured_fixture_preview_apply_and_repeat_share_sdk_path() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("home");
        let source = root.join("source");
        let mount = root.join("mount");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&mount).unwrap();
        fs::write(mount.join("content"), "fixture").unwrap();
        let git = |args: &[&str]| {
            let result = Command::new("git")
                .args(args)
                .current_dir(&root)
                .env("HOME", &home)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Fixture")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Fixture")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
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
        let json = serde_json::json!({
            "collections": {"refs": {"path": "~/collection", "repos": [{"name":"repo", "url":source, "default_branch":"main"}]}},
            "workspaces": {"root":"~/workspaces", "repos":[{"name":"workspace", "url":source, "category":"work", "worktrees":["dev"]}]},
            "storage": {"disk": {"mount":"~/mount", "type":"internal", "symlinks":[{"from":"~/link", "to":"{mount}/content"}]}}
        });
        // Mount is intentionally inside foreign HOME; no actual volume is needed.
        fs::rename(&mount, home.join("mount")).unwrap();
        let config = decode_bossa_config(ConfigFormat::Json, &serde_json::to_vec(&json).unwrap())
            .unwrap()
            .desired;
        let path = std::env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
        let environment = crate::AppEnvironment::new(
            Some(home.clone()),
            [
                ("PATH".into(), path),
                ("HOME".into(), home.clone().into_os_string()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ],
        );
        let runner = setup::git_runner(SystemRunner::new(environment.variables().clone()));
        let make_plan = || {
            let selected = select_configured(&config, None, &environment, &root).unwrap();
            assert_eq!(
                selected.groups,
                ["collections.refs", "storage.disk", "workspaces.workspace"]
            );
            setup::plan(
                selected.specs,
                Arc::new(runner.clone()),
                Arc::new(runner.clone()),
                root.clone(),
            )
            .unwrap()
        };
        let preview = make_plan();
        assert!(setup::observe(&preview).iter().all(|row| row.state.is_ok()));
        let report = setup::apply(
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
        assert!(report.is_success());
        assert!(!home.join("collection").exists());
        assert!(!home.join("workspaces").exists());
        assert!(!home.join("link").exists());
        let execute = |plan| {
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
        assert!(execute(make_plan()).is_success());
        assert!(home.join("collection/repo/.git").is_dir());
        assert!(home.join("workspaces/work/workspace.git").is_dir());
        assert!(home.join("workspaces/work/workspace/dev").is_dir());
        assert!(
            fs::symlink_metadata(home.join("link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let second = make_plan();
        assert!(
            setup::observe(&second)
                .iter()
                .all(|row| matches!(&row.state, Ok(declarative::ResourceState::Present { .. })))
        );
        let result = execute(second);
        assert!(result.is_success());
        assert_eq!(result.total_changes(), 0);
    }

    #[test]
    fn selectors_reject_unknown_and_ambiguous_but_keep_empty_group() {
        let temp = tempfile::tempdir().unwrap();
        let config = decode_bossa_config(ConfigFormat::Json, br#"{"collections":{"same":{"path":"/tmp/x"},"empty":{"path":"/tmp/y"}},"storage":{"same":{"mount":"/tmp/z","type":"internal"}}}"#).unwrap().desired;
        let env = crate::AppEnvironment::new(Some(temp.path().to_path_buf()), []);
        assert!(select_configured(&config, Some("same"), &env, temp.path()).is_err());
        assert!(
            select_configured(&config, Some("collections.missing"), &env, temp.path()).is_err()
        );
        let selected =
            select_configured(&config, Some("collections.empty"), &env, temp.path()).unwrap();
        assert_eq!(selected.groups, ["collections.empty"]);
        assert!(selected.specs.is_empty());
    }
    #[test]
    fn constructed_config_rejects_escaping_names_before_filesystem_access() {
        let mut config = crate::schema::BossaConfig::default();
        let mut collection = crate::schema::Collection {
            path: "/fixture/root".into(),
            description: String::new(),
            clone: Default::default(),
            storage: None,
            repos: Vec::new(),
        };
        for name in ["/tmp/escape", "../escape", "nested/escape"] {
            collection.repos = vec![crate::schema::CollectionRepo {
                name: name.into(),
                url: "fixture".into(),
                default_branch: "main".into(),
                description: String::new(),
            }];
            config
                .collections
                .insert("fixture".into(), collection.clone());
            let env = crate::AppEnvironment::new(None, []);
            let error =
                select_configured(&config, None, &env, std::path::Path::new("/not-present"))
                    .err()
                    .unwrap();
            assert!(
                error.to_string().contains("Invalid collection"),
                "{error:#}"
            );
        }
    }
}
