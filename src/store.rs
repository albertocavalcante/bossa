//! Coordinated document replacement. Locks cover participating writers on one
//! host; they do not provide atomic content-CAS against external editors.

mod metadata;

use crate::{ConfigDocument, ConfigFormat, confirm_missing_parent};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_NAME: &str = ".bossa-store.lock";

fn reserved(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(LOCK_NAME))
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Document revision or selected source changed; reload before retrying")]
    Conflict,
    #[error("Document store is busy; no document replacement attempted")]
    Busy,
    #[error("Document was not promoted (parent/lock artifacts may remain): {0}")]
    BeforePromotion(#[source] io::Error),
    #[error("Document at {} was promoted but directory durability is unknown: {source}", path.display())]
    DurabilityUnknown {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}
impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        Self::BeforePromotion(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Config(String),
    File {
        name: std::ffi::OsString,
        format: ConfigFormat,
    },
}

/// Native store binding. Reads create no directories or lock files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentStore {
    directory: PathBuf,
    selection: Selection,
}

/// Opaque read revision; contains no document bytes and cannot be deserialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreRevision {
    store: DocumentStore,
    directory: PathBuf,
    selected: Option<ObservedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedFile {
    path: PathBuf,
    target: PathBuf,
    stamp: metadata::Stamp,
    digest: [u8; 32],
}

pub struct StoreSnapshot {
    pub document: Option<ConfigDocument>,
    pub revision: StoreRevision,
}

/// File descriptors retain native locks until the whole guarded update ends.
pub struct StoreGuard {
    store: DocumentStore,
    directories: Vec<PathBuf>,
    locks: Vec<File>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn physical(path: &Path) -> io::Result<PathBuf> {
    crate::filesystem::resolve_location(path).map_err(io::Error::other)
}

impl DocumentStore {
    pub fn default_path(&self) -> PathBuf {
        self.directory.join(match &self.selection {
            Selection::Config(name) => format!("{name}.toml").into(),
            Selection::File { name, .. } => name.clone(),
        })
    }
    pub fn config(directory: &Path, name: &str) -> io::Result<Self> {
        if name.is_empty()
            || Path::new(name).components().count() != 1
            || name.contains(['/', '\\'])
            || matches!(name, "." | "..")
        {
            return Err(invalid("Invalid configuration store name"));
        }
        Self::new(directory, Selection::Config(name.into()))
    }
    pub fn file(path: &Path, format: ConfigFormat) -> io::Result<Self> {
        if reserved(path) {
            return Err(invalid(
                "Store coordination files cannot be document targets",
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| invalid("Document has no parent"))?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid("Document has no filename"))?;
        Self::new(
            parent,
            Selection::File {
                name: name.into(),
                format,
            },
        )
    }
    fn new(directory: &Path, selection: Selection) -> io::Result<Self> {
        if !directory.is_absolute() {
            return Err(invalid("Document stores require an absolute directory"));
        }
        Ok(Self {
            directory: directory.into(),
            selection,
        })
    }
    pub fn read(&self) -> Result<StoreSnapshot, StoreError> {
        let directory = physical(&self.directory)?;
        let candidates = match &self.selection {
            Selection::Config(name) => vec![
                (
                    self.directory.join(format!("{name}.toml")),
                    ConfigFormat::Toml,
                ),
                (
                    self.directory.join(format!("{name}.json")),
                    ConfigFormat::Json,
                ),
            ],
            Selection::File { name, format } => vec![(self.directory.join(name), *format)],
        };
        for (path, format) in candidates {
            match fs::symlink_metadata(&path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    confirm_missing_parent(&path)?;
                    continue;
                }
                Err(e) => return Err(e.into()),
                Ok(_) => {}
            }
            let target = path.canonicalize()?;
            if reserved(&target) {
                return Err(invalid("A document cannot target a store coordination file").into());
            }
            let mut file = metadata::open_regular(&target)?;
            let before = metadata::stamp(&file)?;
            let mut bytes = Vec::new();
            (&mut file)
                .take((MAX_BYTES + 1) as u64)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_BYTES {
                return Err(invalid("Document exceeds 8 MiB limit").into());
            }
            if metadata::stamp(&file)? != before || path.canonicalize()? != target {
                return Err(StoreError::Conflict);
            }
            let digest = *blake3::hash(&bytes).as_bytes();
            let contents =
                String::from_utf8(bytes).map_err(|_| invalid("Document must be UTF-8"))?;
            let observed = ObservedFile {
                path: path.clone(),
                target,
                stamp: before,
                digest,
            };
            return Ok(StoreSnapshot {
                document: Some(ConfigDocument::from_contents(path, format, contents)),
                revision: StoreRevision {
                    store: self.clone(),
                    directory,
                    selected: Some(observed),
                },
            });
        }
        Ok(StoreSnapshot {
            document: None,
            revision: StoreRevision {
                store: self.clone(),
                directory,
                selected: None,
            },
        })
    }

    /// Directory-level locks deliberately serialize configuration format choices
    /// and physical file aliases. The stable lock files must never be unlinked.
    pub fn lock(&self) -> Result<StoreGuard, StoreError> {
        self.lock_for(LOCK_TIMEOUT)
    }
    fn lock_for(&self, timeout: Duration) -> Result<StoreGuard, StoreError> {
        let deadline = Instant::now() + timeout;
        for _ in 0..3 {
            let first = self.read()?;
            let mut directories = vec![first.revision.directory.clone()];
            if let Some(selected) = &first.revision.selected {
                directories.push(
                    selected
                        .target
                        .parent()
                        .ok_or_else(|| invalid("Target has no parent"))?
                        .into(),
                );
            }
            directories.sort();
            directories.dedup();
            let mut locks = Vec::new();
            for directory in &directories {
                create_directories(directory)?;
                let lock = metadata::open_lock(&directory.join(".bossa-store.lock"))?;
                loop {
                    match lock.try_lock() {
                        Ok(()) => break,
                        Err(std::fs::TryLockError::WouldBlock) => {
                            if Instant::now() >= deadline {
                                return Err(StoreError::Busy);
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
                    }
                }
                metadata::validate_lock(&lock, &directory.join(".bossa-store.lock"))?;
                locks.push(lock);
            }
            let current = self.read()?;
            let mut required = vec![current.revision.directory];
            if let Some(selected) = current.revision.selected {
                required.push(
                    selected
                        .target
                        .parent()
                        .ok_or_else(|| invalid("Target has no parent"))?
                        .into(),
                );
            }
            if required.iter().all(|path| directories.contains(path)) {
                return Ok(StoreGuard {
                    store: self.clone(),
                    directories,
                    locks,
                });
            }
        }
        Err(StoreError::Conflict)
    }
    /// Optimistic save: serialized with other writers and rejects a stale read.
    pub fn commit(
        &self,
        expected: &StoreRevision,
        contents: &str,
    ) -> Result<StoreRevision, StoreError> {
        self.lock()?.commit(expected, contents)
    }
}

impl StoreGuard {
    pub fn read(&self) -> Result<StoreSnapshot, StoreError> {
        self.store.read()
    }
    pub fn commit(
        &self,
        expected: &StoreRevision,
        contents: &str,
    ) -> Result<StoreRevision, StoreError> {
        if contents.len() > MAX_BYTES {
            return Err(invalid("Document exceeds 8 MiB limit").into());
        }
        self.replace(expected, |file| file.write_all(contents.as_bytes()))
    }
    pub(crate) fn replace(
        &self,
        expected: &StoreRevision,
        write: impl FnOnce(&mut File) -> io::Result<()>,
    ) -> Result<StoreRevision, StoreError> {
        self.replace_with_sync(expected, write, |parent| {
            File::open(parent).and_then(|file| file.sync_all())
        })
    }

    fn replace_with_sync(
        &self,
        expected: &StoreRevision,
        write: impl FnOnce(&mut File) -> io::Result<()>,
        sync_parent: impl FnOnce(&Path) -> io::Result<()>,
    ) -> Result<StoreRevision, StoreError> {
        if expected.store != self.store || self.read()?.revision != *expected {
            return Err(StoreError::Conflict);
        }
        for (directory, lock) in self.directories.iter().zip(&self.locks) {
            metadata::validate_lock(lock, &directory.join(".bossa-store.lock"))?;
        }
        let (path, target) = match &expected.selected {
            Some(selected) => (selected.path.clone(), selected.target.clone()),
            None => {
                let name = match &self.store.selection {
                    Selection::Config(name) => format!("{name}.toml").into(),
                    Selection::File { name, .. } => name.clone(),
                };
                (
                    self.store.directory.join(&name),
                    expected.directory.join(name),
                )
            }
        };
        let parent = target
            .parent()
            .ok_or_else(|| invalid("Target has no parent"))?;
        if !self.directories.contains(&parent.to_path_buf()) {
            return Err(StoreError::Conflict);
        }
        let source = if expected.selected.is_some() {
            Some(metadata::open_regular(&target)?)
        } else {
            None
        };
        if let Some(source) = &source {
            for lock in &self.locks {
                if metadata::same_file(source, lock)? {
                    return Err(invalid("Document aliases a live coordination file").into());
                }
            }
            metadata::verify_writable(&target, source)?;
        }
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        if let Some(source) = &source {
            metadata::copy(source, staged.as_file())?;
        }
        write(staged.as_file_mut())?;
        if let Some(source) = &source {
            metadata::copy(source, staged.as_file())?;
        }
        if staged.as_file().metadata()?.len() > MAX_BYTES as u64 {
            return Err(invalid("Document exceeds 8 MiB limit").into());
        }
        staged.as_file().sync_all()?;
        if self.read()?.revision != *expected
            || physical(&self.store.directory)? != expected.directory
        {
            return Err(StoreError::Conflict);
        }
        if expected.selected.is_some() {
            staged.persist(&target).map_err(|e| e.error)?;
        } else {
            staged.persist_noclobber(&target).map_err(|e| e.error)?;
        }
        sync_parent(parent).map_err(|source| StoreError::DurabilityUnknown {
            path: path.clone(),
            source,
        })?;
        self.read()
            .map(|snapshot| snapshot.revision)
            .map_err(|error| StoreError::DurabilityUnknown {
                path,
                source: io::Error::other(error),
            })
    }
}

fn create_directories(path: &Path) -> io::Result<()> {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        match fs::symlink_metadata(current) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(invalid("Store ancestor is not a real directory"));
                }
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                missing.push(current.to_path_buf());
                current = current
                    .parent()
                    .ok_or_else(|| invalid("No existing store ancestor"))?;
            }
            Err(e) => return Err(e),
        }
    }
    for directory in missing.iter().rev() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        match builder.create(directory) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && directory.is_dir() => {}
            Err(e) => return Err(e),
        }
        File::open(directory).and_then(|f| f.sync_all())?;
        File::open(
            directory
                .parent()
                .ok_or_else(|| invalid("Directory has no parent"))?,
        )
        .and_then(|f| f.sync_all())?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{CommandRequest, IoMode, ProcessLimits, ProcessRunner, SystemRunner};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    fn store(root: &Path) -> DocumentStore {
        DocumentStore::config(root, "config").unwrap()
    }

    #[test]
    fn snapshots_are_read_only_and_stale_or_cross_store_revisions_are_rejected() {
        let root = tempfile::TempDir::new().unwrap();
        let directory = root.path().join("new");
        let store = store(&directory);
        let empty = store.read().unwrap();
        assert!(!directory.exists());
        store.commit(&empty.revision, "value=1\n").unwrap();
        let a = store.read().unwrap();
        let b = store.read().unwrap();
        store.commit(&a.revision, "value=2\n").unwrap();
        assert!(matches!(
            store.commit(&b.revision, "value=3\n"),
            Err(StoreError::Conflict)
        ));
        let other = DocumentStore::config(&directory, "other").unwrap();
        assert!(matches!(
            other.commit(&a.revision, "value=9\n"),
            Err(StoreError::Conflict)
        ));
        assert_eq!(
            fs::read_to_string(directory.join("config.toml")).unwrap(),
            "value=2\n"
        );
    }

    #[test]
    fn changed_content_aliases_and_format_precedence_invalidate_the_old_read() {
        let root = tempfile::TempDir::new().unwrap();
        let directory = root.path().canonicalize().unwrap();
        let store = store(&directory);
        let json = directory.join("config.json");
        fs::write(&json, "{}").unwrap();
        let old = store.read().unwrap();
        fs::write(directory.join("config.toml"), "value=1").unwrap();
        assert!(matches!(
            store.commit(&old.revision, "{}"),
            Err(StoreError::Conflict)
        ));
        fs::remove_file(directory.join("config.toml")).unwrap();
        let old = store.read().unwrap();
        fs::write(&json, "{\"changed\":true}").unwrap();
        assert!(matches!(
            store.commit(&old.revision, "{}"),
            Err(StoreError::Conflict)
        ));
        let link = directory.join("alias.toml");
        std::os::unix::fs::symlink(&json, &link).unwrap();
        let alias = DocumentStore::file(&link, ConfigFormat::Json).unwrap();
        let prior = alias.read().unwrap();
        fs::remove_file(&link).unwrap();
        fs::write(&link, "new owner file").unwrap();
        assert!(matches!(
            alias.commit(&prior.revision, "{}"),
            Err(StoreError::Conflict)
        ));
        assert_eq!(fs::read_to_string(&link).unwrap(), "new owner file");
    }

    #[test]
    fn locks_survive_promotion_and_reserved_or_hard_linked_targets_are_preserved() {
        let root = tempfile::TempDir::new().unwrap();
        let directory = root.path().canonicalize().unwrap();
        let store = store(&directory);
        let guard = store.lock().unwrap();
        let initial = guard.read().unwrap();
        let inode = fs::metadata(directory.join(LOCK_NAME)).unwrap().ino();
        guard.commit(&initial.revision, "value=1").unwrap();
        assert_eq!(
            inode,
            fs::metadata(directory.join(LOCK_NAME)).unwrap().ino()
        );
        assert!(matches!(
            store.lock_for(Duration::from_millis(25)),
            Err(StoreError::Busy)
        ));
        assert!(DocumentStore::file(&directory.join(LOCK_NAME), ConfigFormat::Toml).is_err());
        assert!(
            DocumentStore::file(&directory.join(".BOSSA-STORE.LOCK"), ConfigFormat::Toml).is_err()
        );
        std::os::unix::fs::symlink(directory.join(LOCK_NAME), directory.join("reserved.toml"))
            .unwrap();
        assert!(
            DocumentStore::file(&directory.join("reserved.toml"), ConfigFormat::Toml)
                .unwrap()
                .read()
                .is_err()
        );
        drop(guard);
        fs::hard_link(directory.join("config.toml"), directory.join("copy")).unwrap();
        let before = store.read().unwrap();
        assert!(store.commit(&before.revision, "value=2").is_err());
        assert_eq!(
            fs::read_to_string(directory.join("copy")).unwrap(),
            "value=1"
        );
    }

    #[test]
    fn failed_staging_preserves_bytes_and_post_promotion_sync_failure_is_uncertain() {
        let root = tempfile::TempDir::new().unwrap();
        let store = store(root.path());
        let empty = store.read().unwrap();
        store.commit(&empty.revision, "value=1").unwrap();
        let guard = store.lock().unwrap();
        let old = guard.read().unwrap();
        let error = guard.replace(&old.revision, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected stage failure"))
        });
        assert!(matches!(error, Err(StoreError::BeforePromotion(_))));
        assert_eq!(
            fs::read_to_string(root.path().join("config.toml")).unwrap(),
            "value=1"
        );
        let error = guard.replace_with_sync(
            &old.revision,
            |file| file.write_all(b"value=2"),
            |_| Err(io::Error::other("injected directory sync failure")),
        );
        assert!(matches!(error, Err(StoreError::DurabilityUnknown { .. })));
        assert_eq!(
            fs::read_to_string(root.path().join("config.toml")).unwrap(),
            "value=2"
        );
        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            2,
            "staging artifact leaked"
        );
    }

    #[test]
    fn owned_modes_and_bounded_extended_attributes_survive_both_content_writes() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("config.toml");
        fs::write(&path, "value=1").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let original = metadata::open_regular(&path).unwrap();
        #[cfg(target_os = "linux")]
        let attr = "user.bossa-fixture";
        #[cfg(target_os = "macos")]
        let attr = "com.example.bossa-fixture";
        rustix::fs::fsetxattr(&original, attr, b"kept", rustix::fs::XattrFlags::empty()).unwrap();
        let store = store(root.path());
        let before = store.read().unwrap();
        store.commit(&before.revision, "value=2").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let mut value = [0; 4];
        assert_eq!(
            rustix::fs::fgetxattr(metadata::open_regular(&path).unwrap(), attr, &mut value[..])
                .unwrap(),
            4
        );
        assert_eq!(&value, b"kept");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o420)).unwrap();
        let before = store.read().unwrap();
        assert!(store.commit(&before.revision, "value=3").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "value=2");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn oversized_documents_and_special_files_are_preserved() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("config.toml");
        fs::write(&path, "value=1").unwrap();
        let store = store(root.path());
        let revision = store.read().unwrap().revision;
        assert!(store.commit(&revision, &"x".repeat(MAX_BYTES + 1)).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "value=1");
        fs::write(&path, vec![b'x'; MAX_BYTES + 1]).unwrap();
        assert!(store.read().is_err());
        fs::remove_file(&path).unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(store.read().is_err(), "socket must fail without blocking");
        assert!(fs::symlink_metadata(&path).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_policy_flags_are_rejected_without_replacing_the_document() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("config.toml");
        fs::write(&path, "value=1").unwrap();
        let file = metadata::open_regular(&path).unwrap();
        let flags = rustix::fs::ioctl_getflags(&file).unwrap();
        rustix::fs::ioctl_setflags(&file, flags | rustix::fs::IFlags::NODUMP).unwrap();
        let inode = file.metadata().unwrap().ino();
        let store = store(root.path());
        assert!(
            store
                .commit(&store.read().unwrap().revision, "value=2")
                .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "value=1");
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
        assert!(
            rustix::fs::ioctl_getflags(&file)
                .unwrap()
                .contains(rustix::fs::IFlags::NODUMP)
        );
    }

    #[test]
    fn guarded_child() {
        let Some(root) = std::env::var_os("BOSSA_STORE_CHILD") else {
            return;
        };
        let store = store(Path::new(&root));
        let guard = store.lock().unwrap();
        let snapshot = guard.read().unwrap();
        let value = snapshot
            .document
            .unwrap()
            .contents()
            .trim()
            .strip_prefix("value=")
            .unwrap()
            .parse::<u32>()
            .unwrap();
        std::thread::sleep(Duration::from_millis(50));
        guard
            .commit(&snapshot.revision, &format!("value={}\n", value + 1))
            .unwrap();
    }

    #[test]
    fn independent_processes_update_fresh_bytes_under_one_guard() {
        let root = tempfile::TempDir::new().unwrap();
        let store = store(root.path());
        store
            .commit(&store.read().unwrap().revision, "value=0\n")
            .unwrap();
        std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        let runner = SystemRunner::new(std::collections::BTreeMap::from([(
                            "BOSSA_STORE_CHILD".into(),
                            root.path().as_os_str().to_os_string(),
                        )]))
                        .with_limits(ProcessLimits {
                            timeout: Duration::from_secs(15),
                            max_output_bytes: 64 * 1024,
                        });
                        let output = runner
                            .run(
                                &CommandRequest {
                                    program: std::env::current_exe().unwrap().into_os_string(),
                                    args: ["--exact", "store::tests::guarded_child", "--nocapture"]
                                        .into_iter()
                                        .map(Into::into)
                                        .collect(),
                                    working_dir: root.path().into(),
                                },
                                IoMode::Capture,
                            )
                            .unwrap();
                        assert!(
                            output.status.success(),
                            "{}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                    })
                })
                .collect();
            for job in jobs {
                job.join().unwrap();
            }
        });
        assert_eq!(
            store.read().unwrap().document.unwrap().contents(),
            "value=2\n"
        );
    }
}
