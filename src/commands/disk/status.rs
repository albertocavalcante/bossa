//! Disk status command - list all disks with partitions and space info
//!
//! Uses macOS `diskutil list` and `diskutil info` commands to gather disk information.

use anyhow::{Context, Result};
use colored::Colorize;
use std::collections::HashMap;
use std::process::Command;

use crate::ui;

/// Information about a disk
#[derive(Debug)]
struct DiskInfo {
    /// Device identifier (e.g., "disk0", "disk2")
    device: String,
    /// Disk name/model
    name: String,
    /// Total size in bytes
    size: u64,
    /// Whether this is an internal disk
    internal: bool,
    /// Whether this is the boot disk
    boot: bool,
    /// List of partitions
    partitions: Vec<PartitionInfo>,
}

/// Information about a partition
#[derive(Debug)]
struct PartitionInfo {
    /// Device identifier (e.g., "disk0s1", "disk2s2")
    device: String,
    /// Partition name/label
    name: String,
    /// Filesystem type (APFS, ExFAT, etc.)
    fs_type: String,
    /// Size in bytes
    size: u64,
    /// Mount point (if mounted)
    mount_point: Option<String>,
    /// Used space in bytes (if available)
    used: Option<u64>,
    /// Available space in bytes (if available)
    available: Option<u64>,
}

/// Run the disk status command
pub fn run() -> Result<()> {
    ui::header("Disk Status");
    println!();

    let disks = collect_disk_info()?;

    if disks.is_empty() {
        ui::dim("No disks found");
        return Ok(());
    }

    for disk in &disks {
        print_disk(disk);
    }

    Ok(())
}

/// Collect information about all disks
fn collect_disk_info() -> Result<Vec<DiskInfo>> {
    // Get disk list using diskutil
    let output = Command::new("diskutil")
        .args(["list", "-plist"])
        .output()
        .context("Failed to run diskutil list")?;

    if !output.status.success() {
        anyhow::bail!(
            "diskutil list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Parse the plist output to get disk identifiers
    let plist_str = String::from_utf8_lossy(&output.stdout);
    let disk_ids = parse_disk_identifiers(&plist_str)?;

    // Get detailed info for each disk
    let mut disks = Vec::new();
    for disk_id in disk_ids {
        if let Ok(disk_info) = get_disk_details(&disk_id) {
            disks.push(disk_info);
        }
    }

    Ok(disks)
}

/// Parse disk identifiers from diskutil list -plist output
fn parse_disk_identifiers(plist: &str) -> Result<Vec<String>> {
    // Simple parsing - look for disk identifiers in the plist
    // Format: <string>diskN</string> where N is a number
    let mut identifiers = Vec::new();

    for line in plist.lines() {
        let line = line.trim();
        if line.starts_with("<string>disk") && line.ends_with("</string>") {
            let disk_id = line
                .trim_start_matches("<string>")
                .trim_end_matches("</string>");
            // Only include whole disks (not partitions like disk0s1)
            // Partitions have format diskNsM where N and M are numbers
            // Check if there's an 's' followed by a digit after "disk"
            let is_partition = disk_id
                .strip_prefix("disk")
                .and_then(|rest| {
                    // Find 's' followed by digit
                    rest.find('s').map(|pos| {
                        rest.chars()
                            .nth(pos + 1)
                            .is_some_and(|c| c.is_ascii_digit())
                    })
                })
                .unwrap_or(false);

            if !is_partition {
                identifiers.push(disk_id.to_string());
            }
        }
    }

    // Deduplicate
    identifiers.sort();
    identifiers.dedup();

    Ok(identifiers)
}

/// Get detailed information about a specific disk
fn get_disk_details(disk_id: &str) -> Result<DiskInfo> {
    // Get disk info
    let output = Command::new("diskutil")
        .args(["info", "-plist", disk_id])
        .output()
        .context("Failed to run diskutil info")?;

    if !output.status.success() {
        anyhow::bail!(
            "diskutil info failed for {}: {}",
            disk_id,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let plist_str = String::from_utf8_lossy(&output.stdout);
    let props = parse_plist_dict(&plist_str);

    let name = props
        .get("MediaName")
        .or(props.get("IORegistryEntryName"))
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());

    let size = props
        .get("TotalSize")
        .or(props.get("Size"))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let internal = props.get("Internal").map(|s| s == "true").unwrap_or(false);
    let boot = props.contains_key("BooterDeviceIdentifier")
        || props
            .get("SystemImage")
            .map(|s| s == "true")
            .unwrap_or(false);

    // Get partitions
    let partitions = get_partitions(disk_id)?;

    Ok(DiskInfo {
        device: disk_id.to_string(),
        name,
        size,
        internal,
        boot,
        partitions,
    })
}

/// Get partitions for a disk
fn get_partitions(disk_id: &str) -> Result<Vec<PartitionInfo>> {
    // List partitions using diskutil list
    let output = Command::new("diskutil")
        .args(["list", disk_id])
        .output()
        .context("Failed to run diskutil list for partitions")?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let partition_ids = parse_partition_ids(&output_str, disk_id);

    let mut partitions = Vec::new();
    for part_id in partition_ids {
        if let Ok(part_info) = get_partition_details(&part_id) {
            partitions.push(part_info);
        }
    }

    Ok(partitions)
}

/// Parse partition identifiers from diskutil list output
fn parse_partition_ids(output: &str, disk_id: &str) -> Vec<String> {
    let mut partition_ids = Vec::new();

    for line in output.lines() {
        // Look for lines containing partition identifiers like "disk0s1"
        let line = line.trim();
        if line.contains(disk_id)
            && line.contains('s')
            && let Some(dev) = line.split_whitespace().last()
            && dev.starts_with(disk_id)
            && dev.contains('s')
        {
            partition_ids.push(dev.to_string());
        }
    }

    partition_ids
}

/// Get detailed information about a partition
fn get_partition_details(part_id: &str) -> Result<PartitionInfo> {
    let output = Command::new("diskutil")
        .args(["info", "-plist", part_id])
        .output()
        .context("Failed to run diskutil info for partition")?;

    if !output.status.success() {
        anyhow::bail!("diskutil info failed for {}", part_id);
    }

    let plist_str = String::from_utf8_lossy(&output.stdout);
    let props = parse_plist_dict(&plist_str);

    let name = props
        .get("VolumeName")
        .or(props.get("MediaName"))
        .cloned()
        .unwrap_or_else(|| "Untitled".to_string());

    let fs_type = props
        .get("FilesystemType")
        .or(props.get("FilesystemName"))
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());

    let size = props
        .get("TotalSize")
        .or(props.get("Size"))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let mount_point = props.get("MountPoint").cloned().filter(|s| !s.is_empty());

    // Get space info if mounted
    let (used, available) = if let Some(ref mount) = mount_point {
        get_mount_space(mount).unwrap_or((None, None))
    } else {
        (None, None)
    };

    Ok(PartitionInfo {
        device: part_id.to_string(),
        name,
        fs_type,
        size,
        mount_point,
        used,
        available,
    })
}

/// Parse a simple plist dictionary into key-value pairs
fn parse_plist_dict(plist: &str) -> HashMap<String, String> {
    let mut props = HashMap::new();
    let mut current_key: Option<String> = None;

    for line in plist.lines() {
        let line = line.trim();

        if line.starts_with("<key>") && line.ends_with("</key>") {
            current_key = Some(
                line.trim_start_matches("<key>")
                    .trim_end_matches("</key>")
                    .to_string(),
            );
        } else if let Some(ref key) = current_key {
            // Handle different value types
            if line.starts_with("<string>") && line.ends_with("</string>") {
                let value = line
                    .trim_start_matches("<string>")
                    .trim_end_matches("</string>");
                props.insert(key.clone(), value.to_string());
            } else if line.starts_with("<integer>") && line.ends_with("</integer>") {
                let value = line
                    .trim_start_matches("<integer>")
                    .trim_end_matches("</integer>");
                props.insert(key.clone(), value.to_string());
            } else if line == "<true/>" {
                props.insert(key.clone(), "true".to_string());
            } else if line == "<false/>" {
                props.insert(key.clone(), "false".to_string());
            }
            current_key = None;
        }
    }

    props
}

/// Get space information for a mount point
#[cfg(unix)]
fn get_mount_space(path: &str) -> Result<(Option<u64>, Option<u64>)> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = CString::new(path).context("Invalid path")?;

    // SAFETY: statvfs is a standard POSIX call. We check the return value
    // before using the result.
    unsafe {
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        let result = libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr());

        if result != 0 {
            return Ok((None, None));
        }

        let stat = stat.assume_init();

        // Cast needed on macOS, not on Linux
        #[allow(clippy::unnecessary_cast)]
        let total = stat.f_blocks as u64 * stat.f_frsize;
        #[allow(clippy::unnecessary_cast)]
        let available = stat.f_bavail as u64 * stat.f_frsize;
        let used = total.saturating_sub(available);

        Ok((Some(used), Some(available)))
    }
}

#[cfg(not(unix))]
fn get_mount_space(_path: &str) -> Result<(Option<u64>, Option<u64>)> {
    Ok((None, None))
}

/// Print disk information
fn print_disk(disk: &DiskInfo) {
    // Disk header with type indicator
    let disk_type = if disk.internal {
        "Internal".dimmed()
    } else {
        "External".cyan()
    };

    let boot_indicator = if disk.boot {
        format!(" {}", "(boot)".yellow())
    } else {
        String::new()
    };

    ui::section(&format!(
        "{} - {} [{}]{}",
        disk.device, disk.name, disk_type, boot_indicator
    ));

    ui::kv("Size", &ui::format_size(disk.size));

    if disk.partitions.is_empty() {
        ui::dim("  No partitions");
    } else {
        println!();
        println!("  {}", "Partitions:".bold());

        for part in &disk.partitions {
            print_partition(part);
        }
    }

    println!();
}

/// Print partition information
fn print_partition(part: &PartitionInfo) {
    let fs_colored = match part.fs_type.as_str() {
        "APFS" => part.fs_type.green(),
        "ExFAT" | "exfat" => part.fs_type.yellow(),
        "NTFS" | "ntfs" => part.fs_type.blue(),
        "HFS+" | "hfs" => part.fs_type.magenta(),
        "FAT32" | "msdos" => part.fs_type.yellow(),
        _ => part.fs_type.normal(),
    };

    let mount_info = if let Some(ref mount) = part.mount_point {
        format!(" @ {}", mount.cyan())
    } else {
        " (not mounted)".dimmed().to_string()
    };

    println!(
        "    {} {} [{}]{}",
        part.device.dimmed(),
        part.name.bold(),
        fs_colored,
        mount_info
    );

    // Show space usage if available
    if let (Some(used), Some(available)) = (part.used, part.available) {
        let total = used + available;
        let percent = if total > 0 {
            (used as f64 / total as f64 * 100.0) as u32
        } else {
            0
        };

        let percent_colored = if percent > 90 {
            format!("{}%", percent).red()
        } else if percent > 75 {
            format!("{}%", percent).yellow()
        } else {
            format!("{}%", percent).green()
        };

        println!(
            "      {} / {} ({}) - {} free",
            ui::format_size(used),
            ui::format_size(total),
            percent_colored,
            ui::format_size(available).green()
        );
    } else {
        println!("      {}", ui::format_size(part.size).dimmed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_disk_identifiers() {
        // Simulating diskutil list -plist output format
        let plist = "<string>disk0</string>
<string>disk0s1</string>
<string>disk0s2</string>
<string>disk2</string>
<string>disk2s1</string>";

        let ids = parse_disk_identifiers(plist).unwrap();
        assert!(ids.contains(&"disk0".to_string()));
        assert!(ids.contains(&"disk2".to_string()));
        assert!(!ids.contains(&"disk0s1".to_string())); // Should not include partitions
        assert!(!ids.contains(&"disk2s1".to_string())); // Should not include partitions
    }

    #[test]
    fn test_parse_plist_dict() {
        let plist = r#"
            <dict>
                <key>VolumeName</key>
                <string>Macintosh HD</string>
                <key>TotalSize</key>
                <integer>500000000000</integer>
                <key>Internal</key>
                <true/>
            </dict>
        "#;

        let props = parse_plist_dict(plist);
        assert_eq!(props.get("VolumeName"), Some(&"Macintosh HD".to_string()));
        assert_eq!(props.get("TotalSize"), Some(&"500000000000".to_string()));
        assert_eq!(props.get("Internal"), Some(&"true".to_string()));
    }
}
