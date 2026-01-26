//! Platform-specific disk space utilities

use anyhow::{Context, Result};

use super::types::DiskSpace;

/// Get disk space for a given path
#[cfg(unix)]
pub fn get_disk_space(path: &str) -> Result<DiskSpace> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = CString::new(path).context("Invalid path")?;

    // SAFETY: statvfs is a standard POSIX call. We check the return value
    // before using the result.
    unsafe {
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        let result = libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr());

        if result != 0 {
            anyhow::bail!("statvfs failed for {}", path);
        }

        let stat = stat.assume_init();

        Ok(DiskSpace {
            total: u64::from(stat.f_blocks) * stat.f_frsize,
            available: u64::from(stat.f_bavail) * stat.f_frsize,
        })
    }
}

#[cfg(not(unix))]
pub fn get_disk_space(_path: &str) -> Result<DiskSpace> {
    anyhow::bail!("Disk space detection not supported on this platform")
}

/// Calculate percentage safely, avoiding division by zero
///
/// Returns a whole number percentage (0-100), truncated (not rounded).
pub fn calc_percent(part: u64, total: u64) -> u32 {
    if total == 0 {
        0
    } else {
        (part as f64 / total as f64 * 100.0) as u32
    }
}

/// Format disk usage as "X / Y (Z%)"
pub fn format_disk_usage(used: u64, total: u64, format_size: impl Fn(u64) -> String) -> String {
    let percent = calc_percent(used, total);
    format!(
        "{} / {} ({}%)",
        format_size(used),
        format_size(total),
        percent
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calc_percent_normal() {
        assert_eq!(calc_percent(50, 100), 50);
        assert_eq!(calc_percent(1, 4), 25);
        assert_eq!(calc_percent(100, 100), 100);
    }

    #[test]
    fn test_calc_percent_zero_total() {
        assert_eq!(calc_percent(50, 0), 0);
        assert_eq!(calc_percent(0, 0), 0);
    }

    #[test]
    fn test_calc_percent_zero_part() {
        assert_eq!(calc_percent(0, 100), 0);
    }

    #[test]
    fn test_format_disk_usage() {
        let format = |n: u64| format!("{} B", n);
        assert_eq!(format_disk_usage(50, 100, format), "50 B / 100 B (50%)");
    }

    #[test]
    fn test_disk_space_used() {
        let ds = DiskSpace {
            total: 100,
            available: 30,
        };
        assert_eq!(ds.used(), 70);
    }

    #[test]
    fn test_disk_space_used_underflow() {
        // Edge case: available > total (shouldn't happen, but handle gracefully)
        let ds = DiskSpace {
            total: 30,
            available: 100,
        };
        assert_eq!(ds.used(), 0);
    }
}
