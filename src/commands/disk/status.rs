//! Disk status command - list all disks with partitions and space info
//!
//! Uses macOS `diskutil list` and `diskutil info` commands to gather disk information.

use anyhow::{Context, Result};
use colored::Colorize;
use std::collections::HashMap;
use std::process::Command;

use super::plist as plist_util;
use crate::ui;
use plist::Value;

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

/// Disk list entry with partition identifiers from `diskutil list -plist`.
#[derive(Debug)]
struct DiskListEntry {
    device: String,
    partitions: Vec<String>,
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
    let disk_entries = parse_disk_list(&plist_str)?;

    // Get detailed info for each disk
    let mut disks = Vec::new();
    for entry in disk_entries {
        if let Ok(disk_info) = get_disk_details(&entry.device, &entry.partitions) {
            disks.push(disk_info);
        }
    }

    Ok(disks)
}

/// Parse disk identifiers and partitions from `diskutil list -plist` output.
fn parse_disk_list(plist: &str) -> Result<Vec<DiskListEntry>> {
    let dict = plist_util::parse_plist_dict(plist)?;
    let mut entries = Vec::new();
    let mut all_partitions: HashMap<String, Vec<String>> = HashMap::new();

    if let Some(all_disks) = plist_util::dict_get_array(&dict, "AllDisks") {
        for item in all_disks {
            if let Value::String(disk_id) = item
                && is_partition_identifier(disk_id)
                && let Some(parent) = parent_disk_identifier(disk_id)
            {
                all_partitions
                    .entry(parent.to_string())
                    .or_default()
                    .push(disk_id.clone());
            }
        }
    }

    if let Some(all_disks) = plist_util::dict_get_array(&dict, "AllDisksAndPartitions") {
        for item in all_disks {
            let Value::Dictionary(disk_dict) = item else {
                continue;
            };

            let Some(device) = plist_util::dict_get_string(disk_dict, "DeviceIdentifier") else {
                continue;
            };

            let mut partitions = Vec::new();

            for key in ["Partitions", "APFSVolumes"] {
                if let Some(items) = plist_util::dict_get_array(disk_dict, key) {
                    for part in items {
                        if let Value::Dictionary(part_dict) = part
                            && let Some(part_id) =
                                plist_util::dict_get_string(part_dict, "DeviceIdentifier")
                        {
                            partitions.push(part_id);
                        }
                    }
                }
            }

            partitions.sort();
            partitions.dedup();

            entries.push(DiskListEntry { device, partitions });
        }
    }

    // Enrich with any partitions found in AllDisks (captures snapshots and extras).
    for entry in &mut entries {
        if let Some(extra_parts) = all_partitions.get(&entry.device) {
            entry.partitions.extend(extra_parts.iter().cloned());
            entry.partitions.sort();
            entry.partitions.dedup();
        }
    }

    // Fallback: derive whole disks from AllDisks if structured data is missing.
    if entries.is_empty()
        && let Some(all_disks) = plist_util::dict_get_array(&dict, "AllDisks")
    {
        for item in all_disks {
            if let Value::String(device) = item
                && !is_partition_identifier(device)
            {
                let mut partitions = all_partitions.get(device).cloned().unwrap_or_default();
                partitions.sort();
                partitions.dedup();
                entries.push(DiskListEntry {
                    device: device.clone(),
                    partitions,
                });
            }
        }
    }

    entries.sort_by(|a, b| a.device.cmp(&b.device));
    entries.dedup_by(|a, b| a.device == b.device);

    Ok(entries)
}

fn is_partition_identifier(disk_id: &str) -> bool {
    disk_id
        .strip_prefix("disk")
        .and_then(|rest| {
            rest.find('s').map(|pos| {
                rest.chars()
                    .nth(pos + 1)
                    .is_some_and(|c| c.is_ascii_digit())
            })
        })
        .unwrap_or(false)
}

fn parent_disk_identifier(disk_id: &str) -> Option<&str> {
    let rest = disk_id.strip_prefix("disk")?;
    let pos = rest.find('s')?;
    let parent_len = "disk".len() + pos;
    disk_id.get(..parent_len)
}

/// Get detailed information about a specific disk
fn get_disk_details(disk_id: &str, partition_ids: &[String]) -> Result<DiskInfo> {
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
    let props = plist_util::parse_plist_dict(&plist_str)?;

    let name = plist_util::dict_get_string(&props, "MediaName")
        .or_else(|| plist_util::dict_get_string(&props, "IORegistryEntryName"))
        .unwrap_or_else(|| "Unknown".to_string());

    let size = plist_util::dict_get_u64(&props, "TotalSize")
        .or_else(|| plist_util::dict_get_u64(&props, "Size"))
        .unwrap_or(0);

    let internal = plist_util::dict_get_bool(&props, "Internal").unwrap_or(false);
    let boot = props.contains_key("BooterDeviceIdentifier")
        || plist_util::dict_get_bool(&props, "SystemImage").unwrap_or(false);

    let mut partitions = Vec::new();
    for part_id in partition_ids {
        if let Ok(part_info) = get_partition_details(part_id) {
            partitions.push(part_info);
        }
    }

    Ok(DiskInfo {
        device: disk_id.to_string(),
        name,
        size,
        internal,
        boot,
        partitions,
    })
}

/// Get detailed information about a partition
fn get_partition_details(part_id: &str) -> Result<PartitionInfo> {
    let output = Command::new("diskutil")
        .args(["info", "-plist", part_id])
        .output()
        .context("Failed to run diskutil info for partition")?;

    if !output.status.success() {
        anyhow::bail!("diskutil info failed for {part_id}");
    }

    let plist_str = String::from_utf8_lossy(&output.stdout);
    let props = plist_util::parse_plist_dict(&plist_str)?;

    let name = plist_util::dict_get_string(&props, "VolumeName")
        .or_else(|| plist_util::dict_get_string(&props, "MediaName"))
        .unwrap_or_else(|| "Untitled".to_string());

    let fs_type = plist_util::dict_get_string(&props, "FilesystemType")
        .or_else(|| plist_util::dict_get_string(&props, "FilesystemName"))
        .unwrap_or_else(|| "Unknown".to_string());

    let size = plist_util::dict_get_u64(&props, "TotalSize")
        .or_else(|| plist_util::dict_get_u64(&props, "VolumeSize"))
        .or_else(|| plist_util::dict_get_u64(&props, "Size"))
        .unwrap_or(0);

    let mount_point = plist_util::dict_get_string(&props, "MountPoint").filter(|s| !s.is_empty());

    // Get space info - prefer diskutil values over statvfs (statvfs is broken for ExFAT)
    let (used, available) = get_space_from_diskutil(&props, size);

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

/// Get space info from diskutil plist properties
/// This is more reliable than statvfs, especially for ExFAT volumes
fn get_space_from_diskutil(
    props: &plist::Dictionary,
    total_size: u64,
) -> (Option<u64>, Option<u64>) {
    let free_space = plist_util::dict_get_u64(props, "FreeSpace");
    let container_free = plist_util::dict_get_u64(props, "APFSContainerFree")
        .or_else(|| plist_util::dict_get_u64(props, "ContainerFree"));

    // APFS volumes often report FreeSpace = 0; prefer container free when available.
    let effective_free = if let Some(free) = free_space {
        if free > 0 {
            Some(free)
        } else if let Some(container) = container_free {
            Some(container)
        } else {
            Some(free)
        }
    } else {
        container_free
    };

    if let Some(free_space) = effective_free {
        let used = total_size.saturating_sub(free_space);
        return (Some(used), Some(free_space));
    }

    // No space info available from diskutil
    (None, None)
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
            format!("{percent}%").red()
        } else if percent > 75 {
            format!("{percent}%").yellow()
        } else {
            format!("{percent}%").green()
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
    fn test_parse_disk_list() {
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>AllDisksAndPartitions</key>
  <array>
    <dict>
      <key>DeviceIdentifier</key>
      <string>disk4</string>
      <key>Partitions</key>
      <array>
        <dict>
          <key>DeviceIdentifier</key>
          <string>disk4s1</string>
        </dict>
        <dict>
          <key>DeviceIdentifier</key>
          <string>disk4s2</string>
        </dict>
      </array>
    </dict>
    <dict>
      <key>DeviceIdentifier</key>
      <string>disk3</string>
      <key>APFSVolumes</key>
      <array>
        <dict>
          <key>DeviceIdentifier</key>
          <string>disk3s1</string>
        </dict>
        <dict>
          <key>DeviceIdentifier</key>
          <string>disk3s2</string>
        </dict>
      </array>
    </dict>
  </array>
</dict>
</plist>"#;

        let disks = parse_disk_list(plist).unwrap();
        let disk4 = disks.iter().find(|disk| disk.device == "disk4").unwrap();
        let disk3 = disks.iter().find(|disk| disk.device == "disk3").unwrap();

        assert_eq!(
            disk4.partitions,
            vec!["disk4s1".to_string(), "disk4s2".to_string()]
        );
        assert_eq!(
            disk3.partitions,
            vec!["disk3s1".to_string(), "disk3s2".to_string()]
        );
    }

    #[test]
    fn test_parse_disk_list_adds_all_disks_partitions() {
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>AllDisks</key>
  <array>
    <string>disk3</string>
    <string>disk3s1s1</string>
  </array>
  <key>AllDisksAndPartitions</key>
  <array>
    <dict>
      <key>DeviceIdentifier</key>
      <string>disk3</string>
    </dict>
  </array>
</dict>
</plist>"#;

        let disks = parse_disk_list(plist).unwrap();
        let disk3 = disks.iter().find(|disk| disk.device == "disk3").unwrap();
        assert_eq!(disk3.partitions, vec!["disk3s1s1".to_string()]);
    }

    #[test]
    fn test_parent_disk_identifier() {
        assert_eq!(parent_disk_identifier("disk4s2"), Some("disk4"));
        assert_eq!(parent_disk_identifier("disk3s1s1"), Some("disk3"));
        assert_eq!(parent_disk_identifier("disk0"), None);
    }

    #[test]
    fn test_parse_plist_dict() {
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>VolumeName</key>
    <string>Macintosh HD</string>
    <key>TotalSize</key>
    <integer>500000000000</integer>
    <key>Internal</key>
    <true/>
  </dict>
</plist>"#;

        let props = plist_util::parse_plist_dict(plist).unwrap();
        assert_eq!(
            plist_util::dict_get_string(&props, "VolumeName"),
            Some("Macintosh HD".to_string())
        );
        assert_eq!(
            plist_util::dict_get_u64(&props, "TotalSize"),
            Some(500_000_000_000)
        );
        assert_eq!(plist_util::dict_get_bool(&props, "Internal"), Some(true));
    }

    #[test]
    fn test_get_space_from_diskutil_exfat() {
        // Real values from a 4TB ExFAT volume (from diskutil info -plist)
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>TotalSize</key>
    <integer>4000645775360</integer>
    <key>FreeSpace</key>
    <integer>3983334572032</integer>
  </dict>
</plist>"#;
        let props = plist_util::parse_plist_dict(plist).unwrap();

        let total_size: u64 = 4_000_645_775_360; // ~4TB
        let expected_free: u64 = 3_983_334_572_032; // ~3.98TB
        let expected_used: u64 = total_size - expected_free; // ~17GB

        let (used, available) = get_space_from_diskutil(&props, total_size);

        assert_eq!(used, Some(expected_used));
        assert_eq!(available, Some(expected_free));

        // Verify the values make sense
        let used_val = used.unwrap();
        let avail_val = available.unwrap();
        assert!(avail_val > 3_900_000_000_000, "Available should be ~3.9TB");
        assert!(used_val < 50_000_000_000, "Used should be < 50GB");
        assert_eq!(
            used_val + avail_val,
            total_size,
            "Used + Available should equal total"
        );
    }

    #[test]
    fn test_get_space_from_diskutil_apfs() {
        // APFS volume with container-level free space
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>TotalSize</key>
    <integer>245107195904</integer>
    <key>FreeSpace</key>
    <integer>0</integer>
    <key>APFSContainerFree</key>
    <integer>30000000000</integer>
  </dict>
</plist>"#;
        let props = plist_util::parse_plist_dict(plist).unwrap();

        let total_size: u64 = 245_107_195_904; // ~245GB
        let (used, available) = get_space_from_diskutil(&props, total_size);

        assert!(available.is_some());
        assert!(used.is_some());
        assert_eq!(available.unwrap(), 30_000_000_000);
        assert_eq!(used.unwrap(), total_size - 30_000_000_000);
    }

    #[test]
    fn test_get_space_from_diskutil_prefers_free_space_when_present() {
        // If FreeSpace is present and non-zero, prefer it over container free.
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>TotalSize</key>
    <integer>1000</integer>
    <key>FreeSpace</key>
    <integer>250</integer>
    <key>APFSContainerFree</key>
    <integer>900</integer>
  </dict>
</plist>"#;
        let props = plist_util::parse_plist_dict(plist).unwrap();

        let (used, available) = get_space_from_diskutil(&props, 1000);
        assert_eq!(available, Some(250));
        assert_eq!(used, Some(750));
    }

    #[test]
    fn test_get_space_from_diskutil_uses_container_when_free_zero() {
        // FreeSpace=0 should fall back to container free.
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>TotalSize</key>
    <integer>1000</integer>
    <key>FreeSpace</key>
    <integer>0</integer>
    <key>ContainerFree</key>
    <integer>400</integer>
  </dict>
</plist>"#;
        let props = plist_util::parse_plist_dict(plist).unwrap();

        let (used, available) = get_space_from_diskutil(&props, 1000);
        assert_eq!(available, Some(400));
        assert_eq!(used, Some(600));
    }

    #[test]
    fn test_get_space_from_diskutil_container_only() {
        // When FreeSpace is missing, use container free.
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>TotalSize</key>
    <integer>1000</integer>
    <key>APFSContainerFree</key>
    <integer>900</integer>
  </dict>
</plist>"#;
        let props = plist_util::parse_plist_dict(plist).unwrap();

        let (used, available) = get_space_from_diskutil(&props, 1000);
        assert_eq!(available, Some(900));
        assert_eq!(used, Some(100));
    }

    #[test]
    fn test_get_space_from_diskutil_no_space_info() {
        // Unmounted partition with no space info
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict></dict>
</plist>"#;
        let props = plist_util::parse_plist_dict(plist).unwrap();
        let total_size: u64 = 1_000_000_000;

        let (used, available) = get_space_from_diskutil(&props, total_size);

        assert!(used.is_none());
        assert!(available.is_none());
    }

    #[test]
    fn test_get_space_from_diskutil_large_volume() {
        // Test with very large volume (8TB) to ensure no overflow
        let total_size: u64 = 8_000_000_000_000; // 8TB
        let free_space: u64 = 7_500_000_000_000; // 7.5TB
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>FreeSpace</key>
    <integer>{free_space}</integer>
  </dict>
</plist>"#
        );
        let props = plist_util::parse_plist_dict(&plist).unwrap();

        let (used, available) = get_space_from_diskutil(&props, total_size);

        assert_eq!(available, Some(free_space));
        assert_eq!(used, Some(500_000_000_000)); // 500GB used
    }

    #[test]
    fn test_parse_plist_dict_with_real_diskutil_output() {
        // Actual diskutil info -plist output structure for an ExFAT volume
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>DeviceBlockSize</key>
	<integer>512</integer>
	<key>DeviceIdentifier</key>
	<string>disk4s2</string>
	<key>FilesystemType</key>
	<string>exfat</string>
	<key>FreeSpace</key>
	<integer>3983334572032</integer>
	<key>MountPoint</key>
	<string>/Volumes/T9</string>
	<key>TotalSize</key>
	<integer>4000645775360</integer>
	<key>VolumeName</key>
	<string>T9</string>
	<key>Internal</key>
	<false/>
</dict>
</plist>"#;

        let props = plist_util::parse_plist_dict(plist).unwrap();

        assert_eq!(
            plist_util::dict_get_string(&props, "VolumeName"),
            Some("T9".to_string())
        );
        assert_eq!(
            plist_util::dict_get_string(&props, "FilesystemType"),
            Some("exfat".to_string())
        );
        assert_eq!(
            plist_util::dict_get_u64(&props, "TotalSize"),
            Some(4_000_645_775_360)
        );
        assert_eq!(
            plist_util::dict_get_u64(&props, "FreeSpace"),
            Some(3_983_334_572_032)
        );
        assert_eq!(
            plist_util::dict_get_string(&props, "MountPoint"),
            Some("/Volumes/T9".to_string())
        );
        assert_eq!(plist_util::dict_get_bool(&props, "Internal"), Some(false));

        // Now test that space calculation works correctly with these values
        let total_size: u64 = plist_util::dict_get_u64(&props, "TotalSize").unwrap();
        let (used, available) = get_space_from_diskutil(&props, total_size);

        // ~4TB total, ~4TB free, ~17GB used
        assert!(
            available.unwrap() > 3_900_000_000_000,
            "Should have ~4TB free"
        );
        assert!(used.unwrap() < 50_000_000_000, "Should have < 50GB used");
    }
}
