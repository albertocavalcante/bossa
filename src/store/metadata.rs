//! Qualified native file metadata handling; unsupported inputs remain intact.

use std::fs::{self, File};
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Stamp {
    device: u64,
    inode: u64,
    length: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    links: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

fn unsupported(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn open_regular(path: &Path) -> io::Result<File> {
    let file: File = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?
    .into();
    if !file.metadata()?.is_file() {
        return Err(unsupported("Expected a regular document"));
    }
    Ok(file)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn open_lock(path: &Path) -> io::Result<File> {
    Ok(rustix::fs::open(
        path,
        rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::from_bits_truncate(0o600),
    )?
    .into())
}

#[cfg(unix)]
pub(super) fn stamp(file: &File) -> io::Result<Stamp> {
    use std::os::unix::fs::MetadataExt as _;
    let m = file.metadata()?;
    Ok(Stamp {
        device: m.dev(),
        inode: m.ino(),
        length: m.len(),
        mode: m.mode(),
        uid: m.uid(),
        gid: m.gid(),
        links: m.nlink(),
        modified: (m.mtime(), m.mtime_nsec()),
        changed: (m.ctime(), m.ctime_nsec()),
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn validate_lock(file: &File, path: &Path) -> io::Result<()> {
    let held = stamp(file)?;
    let observed = fs::symlink_metadata(path)?;
    use std::os::unix::fs::MetadataExt as _;
    if !observed.is_file()
        || held.inode != observed.ino()
        || held.device != observed.dev()
        || held.links != 1
        || held.uid != rustix::process::geteuid().as_raw()
        || held.mode & 0o777 != 0o600
    {
        return Err(unsupported(
            "Store lock is not a private owned stable regular file; retained",
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn attributes(file: &File) -> io::Result<Vec<(std::ffi::OsString, Vec<u8>)>> {
    use std::os::unix::ffi::OsStrExt as _;
    let mut names = vec![0u8; 64 * 1024];
    let count = rustix::fs::flistxattr(file, names.as_mut_slice())?;
    names.truncate(count);
    if names.is_empty() {
        return Ok(Vec::new());
    }
    if names.last() != Some(&0) {
        return Err(unsupported("Invalid extended attribute list"));
    }
    let mut result = Vec::new();
    let mut remaining = 8 * 1024 * 1024;
    let mut buffer = vec![0; remaining];
    for name in names[..count - 1].split(|b| *b == 0) {
        if name.is_empty() || result.len() >= 1024 {
            return Err(unsupported("Unsupported extended attribute count"));
        }
        let name = std::ffi::OsStr::from_bytes(name);
        let length = rustix::fs::fgetxattr(file, name, &mut buffer[..remaining])?;
        if length > remaining {
            return Err(unsupported("Extended attributes exceed 8 MiB budget"));
        }
        remaining -= length;
        result.push((name.to_os_string(), buffer[..length].to_vec()));
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn copy(source: &File, target: &File) -> io::Result<()> {
    let original = stamp(source)?;
    if original.links != 1
        || original.uid != rustix::process::geteuid().as_raw()
        || original.mode & 0o7000 != 0
        || original.mode & 0o200 == 0
        || source.metadata()?.permissions().readonly()
    {
        return Err(unsupported(
            "Replacement requires an owned writable single-link file without special mode bits",
        ));
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;
        if source.metadata()?.st_flags() != 0 {
            return Err(unsupported(
                "File flags require an explicitly qualified replacement policy",
            ));
        }
    }
    #[cfg(target_os = "linux")]
    {
        validate_linux_flags(source)?;
        validate_linux_flags(target)?;
    }
    let attrs = attributes(source)?;
    rustix::fs::fchown(
        target,
        None,
        Some(rustix::process::Gid::from_raw(original.gid)),
    )?;
    target.set_permissions(source.metadata()?.permissions())?;
    for (name, _) in attributes(target)? {
        if !attrs.iter().any(|(expected, _)| *expected == name) {
            rustix::fs::fremovexattr(target, &name)?;
        }
    }
    for (name, value) in &attrs {
        rustix::fs::fsetxattr(target, name, value, rustix::fs::XattrFlags::empty())?;
    }
    #[cfg(target_os = "macos")]
    copy_acl(source, target)?;
    #[cfg(target_os = "macos")]
    if mac_acl::read(source)? != mac_acl::read(target)? {
        return Err(unsupported("Extended ACL could not be preserved"));
    }
    let copied = stamp(target)?;
    if copied.uid != original.uid
        || copied.gid != original.gid
        || copied.mode != original.mode
        || attributes(source)? != attrs
        || attributes(target)? != attrs
        || stamp(source)? != original
    {
        return Err(unsupported(
            "Metadata changed or could not be preserved before promotion",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_linux_flags(file: &File) -> io::Result<()> {
    // ext4's EXTENTS bit describes storage layout, not user policy. All other
    // flags (including unknown future bits) require separate qualification.
    // https://www.kernel.org/doc/html/latest/filesystems/ext4/inodes.html
    const EXTENTS: u32 = 0x0008_0000;
    let flags = rustix::fs::ioctl_getflags(file)?.bits();
    if flags & !EXTENTS != 0 {
        return Err(unsupported(
            "Linux inode flags require an explicitly qualified replacement policy",
        ));
    }
    Ok(())
}

pub(super) fn same_file(a: &File, b: &File) -> io::Result<bool> {
    let a = stamp(a)?;
    let b = stamp(b)?;
    Ok(a.device == b.device && a.inode == b.inode)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn verify_writable(path: &Path, source: &File) -> io::Result<()> {
    let opened: File = rustix::fs::open(
        path,
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?
    .into();
    if !same_file(source, &opened)? {
        return Err(unsupported("Writable document identity changed"));
    }
    Ok(())
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn verify_writable(_: &Path, _: &File) -> io::Result<()> {
    Err(unsupported("Native document writes support Linux/macOS"))
}

#[cfg(target_os = "macos")]
mod mac_acl;

#[cfg(target_os = "macos")]
#[allow(unsafe_code)] // rustix's state-pointer API requires a locally owned allocation.
fn copy_acl(source: &File, target: &File) -> io::Result<()> {
    let state = rustix::fs::copyfile_state_alloc()?;
    // SAFETY: state was allocated above, remains live, is not shared and has
    // not been freed. Both File borrows remain valid for this synchronous call.
    let copied =
        unsafe { rustix::fs::fcopyfile(source, target, state, rustix::fs::CopyfileFlags::ACL) };
    // SAFETY: free the same uniquely owned allocation once, after its last use,
    // on both the success and error paths. No operation can panic between uses.
    let freed = unsafe { rustix::fs::copyfile_state_free(state) };
    copied?;
    freed?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn open_regular(_: &Path) -> io::Result<File> {
    Err(unsupported(
        "Native document persistence supports Linux/macOS",
    ))
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn open_lock(_: &Path) -> io::Result<File> {
    Err(unsupported("Native document locking supports Linux/macOS"))
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn validate_lock(_: &File, _: &Path) -> io::Result<()> {
    Err(unsupported("Native document locking supports Linux/macOS"))
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn copy(_: &File, _: &File) -> io::Result<()> {
    Err(unsupported(
        "Native metadata preservation supports Linux/macOS",
    ))
}
#[cfg(not(unix))]
pub(super) fn stamp(_: &File) -> io::Result<Stamp> {
    Err(unsupported("Native document stamps support Linux/macOS"))
}
