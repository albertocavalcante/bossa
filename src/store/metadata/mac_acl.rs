//! Descriptor-bound Darwin ACL serialization. No nameservice/text conversion.
#![allow(unsafe_code)] // Native opaque ACL allocation, bounded serialization and free.

use std::ffi::{c_int, c_void};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd as _;
use std::ptr::NonNull;

// Signatures from the macOS SDK's sys/acl.h. acl_t is an opaque pointer;
// ssize_t has the native isize width on the qualified macOS targets.
unsafe extern "C" {
    fn acl_get_fd(fd: c_int) -> *mut c_void;
    fn acl_size(acl: *mut c_void) -> isize;
    fn acl_copy_ext(buffer: *mut c_void, acl: *mut c_void, size: isize) -> isize;
    fn acl_free(object: *mut c_void) -> c_int;
}

struct Acl(NonNull<c_void>);
impl Drop for Acl {
    fn drop(&mut self) {
        // SAFETY: this uniquely owned allocation came from acl_get_fd and is
        // freed once after all serialization calls; it never escapes this module.
        unsafe {
            acl_free(self.0.as_ptr());
        }
    }
}

pub(super) fn read(file: &File) -> io::Result<Vec<u8>> {
    // SAFETY: File retains a valid descriptor for the synchronous native call.
    let pointer = unsafe { acl_get_fd(file.as_raw_fd()) };
    let Some(pointer) = NonNull::new(pointer) else {
        let error = io::Error::last_os_error();
        // Darwin's descriptor implementation returns ENOENT when FILESEC_ACL is
        // absent (Libc posix1e/acl_file.c -> gen/filesec.c). File keeps the FD live.
        if error.raw_os_error() == Some(rustix::io::Errno::NOENT.raw_os_error()) {
            return Ok(Vec::new());
        }
        return Err(error);
    };
    let acl = Acl(pointer);
    // SAFETY: the opaque ACL is live and has not been freed.
    let size = unsafe { acl_size(acl.0.as_ptr()) };
    if size < 0 {
        return Err(io::Error::last_os_error());
    }
    let size = usize::try_from(size).map_err(io::Error::other)?;
    if size > 64 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "ACL exceeds 64 KiB bound",
        ));
    }
    // kauth_filesec contains integer fields/ACEs: provide explicit 8-byte
    // alignment rather than relying on the allocator alignment of Vec<u8>.
    let mut words = vec![0u64; size.div_ceil(8)];
    // SAFETY: storage is aligned and initialized/writable for at least the
    // bounded byte length supplied. The live ACL and buffer are not shared.
    let copied = unsafe {
        acl_copy_ext(
            words.as_mut_ptr().cast(),
            acl.0.as_ptr(),
            isize::try_from(size).map_err(io::Error::other)?,
        )
    };
    if copied < 0 {
        return Err(io::Error::last_os_error());
    }
    if usize::try_from(copied).map_err(io::Error::other)? != size {
        return Err(io::Error::other("Incomplete ACL serialization"));
    }
    let mut bytes: Vec<u8> = words.into_iter().flat_map(u64::to_ne_bytes).collect();
    bytes.truncate(size);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CommandRequest, DocumentStore, IoMode, ProcessLimits, ProcessRunner, SystemRunner,
    };
    use std::time::Duration;

    #[test]
    fn real_descriptor_acl_survives_document_replacement() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("config.toml");
        std::fs::write(&path, "value=1").unwrap();
        assert_eq!(read(&File::open(&path).unwrap()).unwrap(), [] as [u8; 0]);
        let output = SystemRunner::new(std::collections::BTreeMap::new())
            .with_limits(ProcessLimits {
                timeout: Duration::from_secs(5),
                max_output_bytes: 64 * 1024,
            })
            .run(
                &CommandRequest {
                    program: "/bin/chmod".into(),
                    args: vec![
                        "+a".into(),
                        "group:everyone allow read".into(),
                        path.clone().into_os_string(),
                    ],
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
        let before = read(&File::open(&path).unwrap()).unwrap();
        assert!(!before.is_empty(), "fixture must contain a real ACL");
        let store = DocumentStore::config(root.path(), "config").unwrap();
        store
            .commit(&store.read().unwrap().revision, "value=2")
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "value=2");
        assert_eq!(read(&File::open(&path).unwrap()).unwrap(), before);
    }
}
