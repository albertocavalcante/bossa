//! Verified setup resources. Observations are ephemeral and grant no cleanup authority.

mod git;

use anyhow::{Context, Result, ensure};
use declarative::{ApplyContext, ApplyResult, ExecutionPlan, Resource, ResourceState};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::{ProcessRunner, SystemRunner, confirm_missing_parent};
use git::Git;

#[derive(Debug, Clone)]
pub struct Checkout {
    pub url: String,
    pub path: PathBuf,
    pub branch: String,
    pub depth: u32,
    pub single_branch: bool,
    pub options: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub url: String,
    pub bare: PathBuf,
    pub worktrees: Vec<Worktree>,
}

#[derive(Debug, Clone)]
pub struct Link {
    pub source: PathBuf,
    pub target: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub enum StorageKind {
    Internal,
    External,
    Network,
}

#[derive(Debug, Clone)]
pub struct Storage {
    pub mount: PathBuf,
    pub kind: StorageKind,
    pub links: Vec<Link>,
}

#[derive(Debug, Clone)]
pub enum Setup {
    Checkout(Checkout),
    Workspace(Workspace),
    Storage(Storage),
}

/// Names are caller-owned stable identities; no filesystem paths are derived from them.
#[derive(Debug, Clone)]
pub struct NamedSetup {
    pub name: String,
    pub setup: Setup,
}

/// Child-only Git policy; preserve owner transport/credential configuration.
/// This is not a sandbox. Authorized clone/checkout may run transport/filter helpers.
pub fn git_runner(runner: SystemRunner) -> SystemRunner {
    // Preserve transport and global-config selection, but reject repository or
    // command-config injection inherited from a caller's Git hook.
    runner
        .without_environment_variables(
            [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_COMMON_DIR",
                "GIT_INDEX_FILE",
                "GIT_OBJECT_DIRECTORY",
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                "GIT_CONFIG",
                "GIT_CONFIG_PARAMETERS",
                "GIT_CONFIG_COUNT",
                "GIT_SHALLOW_FILE",
                "GIT_REPLACE_REF_BASE",
                "GIT_QUARANTINE_PATH",
                "GIT_IMPLICIT_WORK_TREE",
                "GIT_PREFIX",
                "GIT_SUPER_PREFIX",
            ]
            .into_iter()
            .map(Into::into),
        )
        .with_environment_overrides([
            ("LC_ALL".into(), "C".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
            ("GIT_NO_LAZY_FETCH".into(), "1".into()),
            ("GCM_INTERACTIVE".into(), "never".into()),
        ])
}

/// Validate physical destination separation before building the existing executor plan.
/// All selected observations still run before any effects. No path is reserved here.
pub fn plan(
    specs: Vec<NamedSetup>,
    observer: Arc<dyn ProcessRunner + Send + Sync>,
    mutator: Arc<dyn ProcessRunner + Send + Sync>,
    working_dir: PathBuf,
) -> Result<ExecutionPlan> {
    ensure!(
        working_dir.is_absolute() && working_dir.is_dir(),
        "Setup needs an existing absolute working directory"
    );
    let mut identities = HashSet::new();
    let mut destinations = Vec::new();
    ensure!(
        specs.len() <= 1_000,
        "Too many selected setup resources (maximum 1000)"
    );
    for spec in &specs {
        ensure!(
            !spec.name.is_empty() && identities.insert((spec.kind(), spec.name.clone())),
            "Duplicate or empty setup identity"
        );
        spec.validate()?;
        for path in spec.destinations() {
            let resolved = destination_location(path)?;
            for previous in &destinations {
                ensure!(
                    !resolved.starts_with(previous) && !Path::new(previous).starts_with(&resolved),
                    "Selected setup destinations overlap: {}",
                    path.display()
                );
            }
            destinations.push(resolved);
            ensure!(
                destinations.len() <= 1_000,
                "Too many selected setup destinations (maximum 1000)"
            );
        }
    }
    // Sources are read dependencies, never safe places for another selected
    // resource to create a repository or link during the same invocation.
    for spec in &specs {
        if let Setup::Storage(storage) = &spec.setup {
            for link in &storage.links {
                let source = resolve_location(&link.source)?;
                ensure!(
                    !destinations
                        .iter()
                        .any(|p| source.starts_with(p) || p.starts_with(&source)),
                    "A selected destination overlaps a storage source"
                );
            }
        }
    }
    let git = Arc::new(Git {
        observer,
        mutator,
        cwd: working_dir,
    });
    let mut plan = ExecutionPlan::new();
    for spec in specs {
        let locations = spec
            .destinations()
            .into_iter()
            .map(|path| Ok((path.to_path_buf(), destination_location(path)?)))
            .collect::<Result<Vec<_>>>()?;
        plan.add_resource_explicit(Box::new(SetupResource {
            spec,
            locations,
            #[cfg(unix)]
            mount_identity: std::sync::Mutex::new(None),
            git: Arc::clone(&git),
        }));
    }
    Ok(plan)
}

impl NamedSetup {
    fn kind(&self) -> &'static str {
        match self.setup {
            Setup::Checkout(_) => "collection_repo",
            Setup::Workspace(_) => "workspace",
            Setup::Storage(_) => "storage",
        }
    }
    fn destinations(&self) -> Vec<&Path> {
        match &self.setup {
            Setup::Checkout(repo) => vec![&repo.path],
            Setup::Workspace(workspace) => std::iter::once(workspace.bare.as_path())
                .chain(workspace.worktrees.iter().map(|w| w.path.as_path()))
                .collect(),
            Setup::Storage(storage) => storage
                .links
                .iter()
                .map(|link| link.target.as_path())
                .collect(),
        }
    }
    fn validate(&self) -> Result<()> {
        for path in self.destinations() {
            validate_path(path)?;
        }
        match &self.setup {
            Setup::Checkout(repo) => {
                validate_url(&repo.url)?;
                validate_branch(&repo.branch)?;
                for option in &repo.options {
                    ensure!(
                        ["--no-checkout", "--no-tags", "--tags", "--no-hardlinks"]
                            .contains(&option.as_str()),
                        "Unsupported clone option {option:?}; setup accepts only --no-checkout, --no-tags, --tags and --no-hardlinks"
                    );
                }
            }
            Setup::Workspace(workspace) => {
                validate_url(&workspace.url)?;
                let mut branches = HashSet::new();
                for tree in &workspace.worktrees {
                    validate_branch(&tree.branch)?;
                    ensure!(
                        branches.insert(&tree.branch),
                        "Duplicate requested worktree branch"
                    );
                }
            }
            Setup::Storage(storage) => {
                validate_path(&storage.mount)?;
                for link in &storage.links {
                    validate_path(&link.source)?;
                }
            }
        }
        Ok(())
    }
}

fn validate_path(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
        "Setup paths must be absolute without traversal: {}",
        path.display()
    );
    ensure!(
        path.parent().is_some(),
        "Filesystem root cannot be a setup destination"
    );
    Ok(())
}
fn validate_url(url: &str) -> Result<()> {
    ensure!(
        !url.is_empty() && !url.starts_with('-') && !url.chars().any(char::is_control),
        "Invalid repository locator"
    );
    Ok(())
}
fn validate_branch(branch: &str) -> Result<()> {
    ensure!(
        !branch.is_empty()
            && branch != "HEAD"
            && !branch.starts_with('-')
            && !branch.contains("..")
            && !branch.contains("@{")
            && !branch.ends_with('.')
            && branch
                .split('/')
                .all(|p| !p.is_empty() && !p.starts_with('.') && !p.ends_with(".lock"))
            && !branch
                .chars()
                .any(|c| c.is_control() || " ~^:?*[\\".contains(c)),
        "Invalid branch name"
    );
    Ok(())
}

/// Resolve existing ancestors without treating broken links or non-directories as absence.
pub fn resolve_location(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "Location must be absolute");
    let mut existing = path.to_path_buf();
    let mut suffix = Vec::new();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(_) => {
                let mut resolved = existing.canonicalize().context("Location unavailable")?;
                ensure!(
                    suffix.is_empty() || fs::metadata(&existing)?.is_dir(),
                    "Location parent is not a directory"
                );
                for part in suffix.into_iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(
                    existing
                        .file_name()
                        .context("No existing location ancestor")?
                        .to_os_string(),
                );
                existing = existing.parent().context("No location parent")?.into();
            }
            Err(e) => return Err(e).context("Could not inspect location"),
        }
    }
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            confirm_missing_parent(path)?;
            Ok(false)
        }
        Err(e) => Err(e).context("Location observation unavailable"),
    }
}

// A link leaf is a destination directory entry, never its referenced source.
fn destination_location(path: &Path) -> Result<PathBuf> {
    Ok(
        resolve_location(path.parent().context("Destination has no parent")?)?
            .join(path.file_name().context("Destination has no name")?),
    )
}

fn directory(path: &Path) -> Result<()> {
    ensure!(
        fs::symlink_metadata(path)?.is_dir(),
        "Expected a real directory at {}",
        path.display()
    );
    Ok(())
}
fn create_parent(path: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().context("Destination has no parent")?)?;
    Ok(())
}

struct SetupResource {
    spec: NamedSetup,
    locations: Vec<(PathBuf, PathBuf)>,
    #[cfg(unix)]
    mount_identity: std::sync::Mutex<Option<(u64, u64)>>,
    git: Arc<Git>,
}
impl std::fmt::Debug for SetupResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupResource")
            .field("name", &self.spec.name)
            .finish_non_exhaustive()
    }
}
impl Resource for SetupResource {
    fn id(&self) -> String {
        self.spec.name.clone()
    }
    fn description(&self) -> String {
        format!("Verify setup for {}", self.spec.name)
    }
    fn resource_type(&self) -> &'static str {
        self.spec.kind()
    }
    fn desired_state(&self) -> ResourceState {
        ResourceState::Present { details: None }
    }
    fn current_state(&self) -> Result<ResourceState> {
        if !matches!(self.spec.setup, Setup::Storage(_)) {
            self.git.check_observation_capability()?;
        }
        for (path, expected) in &self.locations {
            ensure!(
                &destination_location(path)? == expected,
                "Destination parent resolves differently; preserved"
            );
        }
        let missing = match &self.spec.setup {
            Setup::Checkout(repo) => {
                if self.checkout(repo)? {
                    Vec::new()
                } else {
                    vec![repo.path.clone()]
                }
            }
            Setup::Workspace(workspace) => self.workspace(workspace)?,
            Setup::Storage(storage) => self.storage(storage)?,
        };
        Ok(if missing.is_empty() {
            self.desired_state()
        } else {
            ResourceState::Modified {
                from: format!("Missing: {missing:?}"),
                to: "Verified selected setup".into(),
            }
        })
    }
    fn can_parallelize(&self) -> bool {
        false
    }
    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        ensure!(!ctx.dry_run, "Preview cannot call setup mutations");
        self.apply_effects().with_context(|| {
            format!(
                "Setup incomplete; inspect retained destinations: {:?}",
                self.spec.destinations()
            )
        })
    }
}

impl SetupResource {
    fn apply_effects(&self) -> Result<ApplyResult> {
        if self.current_state()? == self.desired_state() {
            return Ok(ApplyResult::NoChange);
        }
        match &self.spec.setup {
            Setup::Checkout(repo) => {
                if self.checkout(repo)? {
                    return Ok(ApplyResult::NoChange);
                }
                create_parent(&repo.path)?;
                fs::create_dir(&repo.path).context(
                    "Checkout destination could not be reserved; existing entries preserved",
                )?;
                let mut options: Vec<OsString> = vec![
                    if repo.single_branch {
                        "--single-branch"
                    } else {
                        "--no-single-branch"
                    }
                    .into(),
                ];
                if repo.depth > 0 {
                    options.extend(["--depth".into(), repo.depth.to_string().into()]);
                }
                options.extend(repo.options.iter().map(Into::into));
                options.extend(["--branch".into(), repo.branch.clone().into()]);
                self.git.clone_repo(&repo.url, &repo.path, options)?;
                ensure!(
                    self.checkout(repo)?,
                    "Clone did not establish the requested checkout"
                );
            }
            Setup::Workspace(workspace) => {
                self.workspace(workspace)?;
                if !exists(&workspace.bare)? {
                    create_parent(&workspace.bare)?;
                    fs::create_dir(&workspace.bare).context(
                        "Bare destination could not be reserved; existing entries preserved",
                    )?;
                    self.git.clone_repo(
                        &workspace.url,
                        &workspace.bare,
                        vec!["--bare".into(), "--no-single-branch".into()],
                    )?;
                }
                // Validate every available branch/destination after clone before adding any tree.
                self.workspace(workspace)?;
                for tree in &workspace.worktrees {
                    if !exists(&tree.path)? {
                        self.workspace(workspace)?;
                        create_parent(&tree.path)?;
                        fs::create_dir(&tree.path).context("Worktree destination could not be reserved; existing entries preserved")?;
                        self.git.add(&workspace.bare, &tree.path, &tree.branch)?;
                        self.workspace(workspace)?;
                    }
                }
            }
            Setup::Storage(storage) => {
                self.storage(storage)?;
                for link in &storage.links {
                    self.storage(storage)?;
                    if !exists(&link.target)? {
                        create_parent(&link.target)?;
                        #[cfg(unix)]
                        std::os::unix::fs::symlink(&link.source, &link.target)?;
                        #[cfg(not(unix))]
                        anyhow::bail!("Storage link creation is unsupported on this platform");
                    }
                }
            }
        }
        ensure!(
            self.current_state()? == self.desired_state(),
            "Setup effects did not satisfy selected postconditions; partial files retained"
        );
        Ok(ApplyResult::Created)
    }

    fn checkout(&self, repo: &Checkout) -> Result<bool> {
        if !exists(&repo.path)? {
            if git::is_local_locator(&repo.url) {
                let source = PathBuf::from(self.git.locator(&repo.url)?);
                if exists(&source)? {
                    self.git.require_branch(&source, &repo.branch)?;
                }
            }
            return Ok(false);
        }
        directory(&repo.path)?;
        let root = repo.path.canonicalize()?;
        self.git.kind(&repo.path, false)?;
        ensure!(
            self.git.path(&repo.path, "--show-toplevel")? == root,
            "Checkout is not its repository root"
        );
        let git_dir = root.join(".git");
        directory(&git_dir)?;
        ensure!(
            self.git.path(&repo.path, "--absolute-git-dir")? == git_dir
                && self.git.path(&repo.path, "--git-common-dir")? == git_dir,
            "Collection needs a standalone checkout"
        );
        self.git.origin(&repo.path, &repo.url)?;
        self.git.branch(&repo.path, &repo.branch)?;
        Ok(true)
    }

    fn workspace(&self, workspace: &Workspace) -> Result<Vec<PathBuf>> {
        if !exists(&workspace.bare)? {
            for tree in &workspace.worktrees {
                ensure!(
                    !exists(&tree.path)?,
                    "Worktree destination exists without its configured bare repository; preserved"
                );
            }
            // Local source refs are knowable without contacting a remote.
            if git::is_local_locator(&workspace.url) {
                let source = PathBuf::from(self.git.locator(&workspace.url)?);
                if exists(&source)? {
                    for tree in &workspace.worktrees {
                        self.git.require_branch(&source, &tree.branch)?;
                    }
                }
            }
            return Ok(std::iter::once(workspace.bare.clone())
                .chain(workspace.worktrees.iter().map(|w| w.path.clone()))
                .collect());
        }
        directory(&workspace.bare)?;
        let bare = workspace.bare.canonicalize()?;
        self.git.kind(&bare, true)?;
        ensure!(
            self.git.path(&bare, "--absolute-git-dir")? == bare
                && self.git.path(&bare, "--git-common-dir")? == bare,
            "Bare repository identity differs"
        );
        self.git.origin(&bare, &workspace.url)?;
        let registrations = self.git.registrations(&bare)?;
        let mut missing = Vec::new();
        for tree in &workspace.worktrees {
            self.git.require_branch(&bare, &tree.branch)?;
            let location = resolve_location(&tree.path)?;
            let reference = format!("refs/heads/{}", tree.branch);
            let mut matches = Vec::new();
            for registration in &registrations {
                if resolve_location(&registration.path)? == location
                    || registration.branch.as_deref() == Some(&reference)
                {
                    matches.push(registration);
                }
            }
            if !exists(&tree.path)? {
                ensure!(
                    matches.is_empty(),
                    "Requested worktree path or branch already registered; no force/prune/repair attempted"
                );
                missing.push(tree.path.clone());
                continue;
            }
            directory(&tree.path)?;
            ensure!(
                matches.len() == 1,
                "Worktree registration is missing or ambiguous"
            );
            let registration = matches[0];
            ensure!(
                !registration.unavailable
                    && resolve_location(&registration.path)? == location
                    && registration.branch.as_deref() == Some(&reference),
                "Worktree registration conflicts or is locked/prunable"
            );
            self.git.kind(&tree.path, false)?;
            ensure!(
                self.git.path(&tree.path, "--show-toplevel")? == location
                    && self.git.path(&tree.path, "--git-common-dir")? == bare,
                "Worktree belongs to a different repository"
            );
            let git_dir = self.git.path(&tree.path, "--absolute-git-dir")?;
            ensure!(
                git_dir.starts_with(bare.join("worktrees")),
                "Unexpected linked-worktree administration path"
            );
            self.git.branch(&tree.path, &tree.branch)?;
            ensure!(
                registration.head.as_deref() == Some(&self.git.commit(&tree.path)?),
                "Worktree HEAD disagrees with its registration"
            );
        }
        Ok(missing)
    }

    fn storage(&self, storage: &Storage) -> Result<Vec<PathBuf>> {
        directory(&storage.mount).context("Storage root unavailable; no mount attempted")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = fs::metadata(&storage.mount)?;
            let identity = (metadata.dev(), metadata.ino());
            let mut observed = self
                .mount_identity
                .lock()
                .map_err(|_| anyhow::anyhow!("Storage observation lock unavailable"))?;
            if let Some(previous) = *observed {
                ensure!(
                    identity == previous,
                    "Storage root changed during selected setup"
                );
            } else {
                *observed = Some(identity);
            }
        }
        #[cfg(unix)]
        if !matches!(storage.kind, StorageKind::Internal) {
            use std::os::unix::fs::MetadataExt;
            let mount = storage.mount.canonicalize()?;
            let parent = mount.parent().context("Storage root has no mount parent")?;
            ensure!(
                fs::metadata(&mount)?.dev() != fs::metadata(parent)?.dev(),
                "External/network storage root is not an observed filesystem mount boundary"
            );
        }
        #[cfg(not(unix))]
        if !matches!(storage.kind, StorageKind::Internal) {
            anyhow::bail!("Mount observation unsupported on this platform");
        }
        let mut missing = Vec::new();
        for link in &storage.links {
            let source = link
                .source
                .canonicalize()
                .context("Storage link source unavailable")?;
            if exists(&link.target)? {
                ensure!(
                    fs::symlink_metadata(&link.target)?.file_type().is_symlink(),
                    "Storage target is not a symlink; preserved"
                );
                ensure!(
                    link.target
                        .canonicalize()
                        .context("Storage target is a dangling link; preserved")?
                        == source,
                    "Storage link has the wrong target; preserved"
                );
            } else {
                missing.push(link.target.clone());
            }
        }
        Ok(missing)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{ResourceApplyOptions, apply_resources};
    use declarative::{CommandOutput, ConfirmCallback, EffectStatus, NoProgress, SudoProvider};

    struct Unavailable;
    impl ProcessRunner for Unavailable {
        fn run(
            &self,
            _: &crate::CommandRequest,
            _: crate::IoMode,
        ) -> std::io::Result<std::process::Output> {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "injected launch failure",
            ))
        }
    }
    struct NoPrivilege;
    impl SudoProvider for NoPrivilege {
        fn run(&self, _: &str, _: &[&str]) -> Result<CommandOutput> {
            panic!("setup never requires privilege")
        }
    }
    struct ChangedBeforeApply(PathBuf);
    impl ConfirmCallback for ChangedBeforeApply {
        fn confirm(&mut self, _: &str) -> Result<bool> {
            fs::write(&self.0, "concurrent owner content")?;
            Ok(true)
        }
    }

    #[test]
    fn changed_storage_precondition_is_preserved_and_never_becomes_a_success() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mount = root.join("mount");
        fs::create_dir(&mount).unwrap();
        let source = mount.join("source");
        fs::write(&source, "source").unwrap();
        let target = root.join("target");
        let runner = Arc::new(Unavailable);
        let plan = plan(
            vec![NamedSetup {
                name: "disk".into(),
                setup: Setup::Storage(Storage {
                    mount,
                    kind: StorageKind::Internal,
                    links: vec![Link {
                        source,
                        target: target.clone(),
                    }],
                }),
            }],
            runner.clone(),
            runner,
            root.clone(),
        )
        .unwrap();
        let report = apply_resources(
            plan,
            ResourceApplyOptions::default(),
            &Unavailable,
            &root,
            || Ok(NoPrivilege),
            &mut NoProgress,
            &mut ChangedBeforeApply(target.clone()),
        )
        .unwrap();
        assert!(!report.is_success());
        assert_eq!(report.outcomes[0].effects, EffectStatus::None);
        assert_eq!(
            fs::read_to_string(target).unwrap(),
            "concurrent owner content"
        );
    }

    #[test]
    fn launch_failure_reports_retained_location_and_possible_partial_effects() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let destination = root.join("checkout");
        let native = git_runner(SystemRunner::new(std::collections::BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), root.clone().into_os_string()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ])));
        let plan = plan(
            vec![NamedSetup {
                name: "repo".into(),
                setup: Setup::Checkout(Checkout {
                    url: root.join("absent-source").to_str().unwrap().into(),
                    path: destination.clone(),
                    branch: "main".into(),
                    depth: 0,
                    single_branch: false,
                    options: vec![],
                }),
            }],
            Arc::new(native),
            Arc::new(Unavailable),
            root.clone(),
        )
        .unwrap();
        let report = apply_resources(
            plan,
            ResourceApplyOptions {
                yes: true,
                ..Default::default()
            },
            &Unavailable,
            &root,
            || Ok(NoPrivilege),
            &mut NoProgress,
            &mut declarative::AutoDecline,
        )
        .unwrap();
        assert!(!report.is_success());
        assert_eq!(report.outcomes[0].effects, EffectStatus::Unknown);
        assert!(
            report.outcomes[0]
                .diagnostic
                .as_ref()
                .unwrap()
                .contains(destination.to_str().unwrap())
        );
        assert!(destination.is_dir());
    }

    #[test]
    fn physical_overlaps_and_excessive_selected_scope_fail_before_execution() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let make = |name: String, target: PathBuf| NamedSetup {
            name,
            setup: Setup::Checkout(Checkout {
                url: "unused".into(),
                path: target,
                branch: "main".into(),
                depth: 0,
                single_branch: false,
                options: vec![],
            }),
        };
        let runner = Arc::new(Unavailable);
        std::os::unix::fs::symlink(&root, root.join("alias")).unwrap();
        assert!(
            plan(
                vec![
                    make("first".into(), root.join("repo")),
                    make("second".into(), root.join("alias/repo/child"))
                ],
                runner.clone(),
                runner.clone(),
                root.clone()
            )
            .is_err()
        );
        let specs = (0..1_001)
            .map(|n| make(n.to_string(), root.join(n.to_string())))
            .collect();
        assert!(plan(specs, runner.clone(), runner, root).is_err());
    }
}
