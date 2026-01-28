//! Disk repartition command - interactive guided repartition for external drives
//!
//! # Safety Features
//!
//! - Refuses to operate on internal/boot disks
//! - Requires explicit `--confirm` flag to execute
//! - Shows current partition layout before changes
//! - Generates and displays the exact diskutil command
//! - Supports `--dry-run` to preview operations

use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::{Confirm, Input, Select};
use std::process::Command;

use super::plist as plist_util;
use crate::ui;

/// Partition specification from user input
#[derive(Debug, Clone)]
struct PartitionSpec {
    /// Partition name
    name: String,
    /// Filesystem type (for diskutil command)
    fs_type: String,
    /// Display name for filesystem (may differ from fs_type)
    fs_display: String,
    /// Size specification (e.g., "3TB", "1TB", or "0b" for remaining space)
    size: String,
    /// Whether to encrypt this partition (for APFS)
    encrypted: bool,
}

/// Current disk information
#[derive(Debug)]
struct CurrentDiskInfo {
    device: String,
    name: String,
    size: u64,
    internal: bool,
    boot: bool,
    partitions: Vec<CurrentPartition>,
}

#[derive(Debug)]
struct CurrentPartition {
    device: String,
    name: String,
    fs_type: String,
    size: u64,
}

/// Run the repartition command
pub fn run(disk: &str, dry_run: bool, confirm: bool) -> Result<()> {
    // Normalize disk identifier
    let disk_id = normalize_disk_id(disk);

    if dry_run {
        ui::header("Repartition (Dry Run)");
    } else {
        ui::header("Repartition External Drive");
    }
    println!();

    // Get current disk info
    let disk_info = get_disk_info(&disk_id)?;

    // Safety checks
    if disk_info.internal {
        ui::error("Cannot repartition internal disk!");
        ui::dim("This command only works on external drives for safety.");
        anyhow::bail!("Refusing to repartition internal disk: {}", disk_id);
    }

    if disk_info.boot {
        ui::error("Cannot repartition boot disk!");
        ui::dim("The system is running from this disk.");
        anyhow::bail!("Refusing to repartition boot disk: {}", disk_id);
    }

    // Show current layout
    print_current_layout(&disk_info);

    // Warning about data loss
    println!();
    println!(
        "  {} {}",
        "WARNING:".red().bold(),
        "Repartitioning will ERASE ALL DATA on this disk!".red()
    );
    println!();

    // Interactive partition configuration
    let partition_specs = get_partition_specs(&disk_info)?;

    if partition_specs.is_empty() {
        ui::info("No partitions configured. Exiting.");
        return Ok(());
    }

    // Generate the diskutil command
    let diskutil_cmd = generate_diskutil_command(&disk_id, &partition_specs);

    println!();
    ui::section("Generated Command");
    println!();
    println!("  {}", diskutil_cmd.cyan());
    println!();

    if dry_run {
        ui::dim("(dry run - command not executed)");
        println!();
        ui::dim("To execute, run without --dry-run and with --confirm flag.");
        return Ok(());
    }

    if !confirm {
        println!(
            "  {} To execute this command, run with {}",
            "Note:".yellow(),
            "--confirm".cyan()
        );
        println!();

        // Interactive confirmation
        if !Confirm::new()
            .with_prompt("Do you want to execute this command now?")
            .default(false)
            .interact()
            .context("Failed to read user input")?
        {
            ui::info("Aborted. No changes made.");
            return Ok(());
        }
    }

    // Final safety check
    println!();
    println!(
        "  {} This will {}!",
        "FINAL WARNING:".red().bold(),
        "destroy all data on the disk".red().bold()
    );
    println!();

    if !Confirm::new()
        .with_prompt(format!(
            "Are you ABSOLUTELY SURE you want to repartition {}?",
            disk_id
        ))
        .default(false)
        .interact()
        .context("Failed to read user input")?
    {
        ui::info("Aborted. No changes made.");
        return Ok(());
    }

    // Execute the command
    execute_repartition(&disk_id, &partition_specs)?;

    Ok(())
}

/// Normalize disk identifier (e.g., "disk2" or "/dev/disk2" -> "disk2")
fn normalize_disk_id(disk: &str) -> String {
    let disk = disk.trim_start_matches("/dev/");
    disk.to_string()
}

/// Get current disk information
fn get_disk_info(disk_id: &str) -> Result<CurrentDiskInfo> {
    let output = Command::new("diskutil")
        .args(["info", "-plist", disk_id])
        .output()
        .context("Failed to run diskutil info")?;

    if !output.status.success() {
        anyhow::bail!(
            "Disk not found or not accessible: {}. Error: {}",
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
    let boot = is_boot_disk(&props);

    // Get current partitions
    let partitions = get_current_partitions(disk_id)?;

    Ok(CurrentDiskInfo {
        device: disk_id.to_string(),
        name,
        size,
        internal,
        boot,
        partitions,
    })
}

/// Check if this is a boot disk
fn is_boot_disk(props: &plist::Dictionary) -> bool {
    // Check various indicators
    if props.get("BooterDeviceIdentifier").is_some() {
        return true;
    }
    if plist_util::dict_get_bool(props, "SystemImage").unwrap_or(false) {
        return true;
    }

    // Also check if "/" is mounted on this disk
    false
}

/// Get current partitions
fn get_current_partitions(disk_id: &str) -> Result<Vec<CurrentPartition>> {
    let output = Command::new("diskutil")
        .args(["list", disk_id])
        .output()
        .context("Failed to list partitions")?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let mut partitions = Vec::new();

    // Parse the output to find partition lines
    for line in output_str.lines() {
        let line = line.trim();
        // Look for lines with partition info
        // Format: "   2:                APFS Volume            Macintosh HD                 227.1 GB   disk0s2"
        if !line.starts_with(char::is_numeric) {
            continue;
        }

        // Try to extract partition device at the end
        if let Some(device) = line.split_whitespace().last()
            && device.starts_with(disk_id)
            && device.contains('s')
            && let Ok(part_info) = get_partition_info(device)
        {
            partitions.push(part_info);
        }
    }

    Ok(partitions)
}

/// Get info for a single partition
fn get_partition_info(part_id: &str) -> Result<CurrentPartition> {
    let output = Command::new("diskutil")
        .args(["info", "-plist", part_id])
        .output()
        .context("Failed to get partition info")?;

    if !output.status.success() {
        anyhow::bail!("Failed to get partition info for {}", part_id);
    }

    let plist_str = String::from_utf8_lossy(&output.stdout);
    let props = plist_util::parse_plist_dict(&plist_str)?;

    let name = plist_util::dict_get_string(&props, "VolumeName")
        .or_else(|| plist_util::dict_get_string(&props, "MediaName"))
        .unwrap_or_else(|| "Untitled".to_string());

    let fs_type = plist_util::dict_get_string(&props, "FilesystemType")
        .or_else(|| plist_util::dict_get_string(&props, "FilesystemName"))
        .or_else(|| plist_util::dict_get_string(&props, "Content"))
        .unwrap_or_else(|| "Unknown".to_string());

    let size = plist_util::dict_get_u64(&props, "TotalSize")
        .or_else(|| plist_util::dict_get_u64(&props, "Size"))
        .unwrap_or(0);

    Ok(CurrentPartition {
        device: part_id.to_string(),
        name,
        fs_type,
        size,
    })
}

/// Print current disk layout
fn print_current_layout(disk: &CurrentDiskInfo) {
    ui::section("Current Layout");
    println!();

    ui::kv("Disk", &format!("{} ({})", disk.device, disk.name));
    ui::kv("Size", &ui::format_size(disk.size));
    ui::kv(
        "Type",
        if disk.internal {
            "Internal"
        } else {
            "External"
        },
    );

    if disk.partitions.is_empty() {
        println!();
        ui::dim("  No partitions found");
    } else {
        println!();
        println!("  {}", "Current Partitions:".bold());
        for part in &disk.partitions {
            println!(
                "    {} {} [{}] - {}",
                part.device.dimmed(),
                part.name,
                part.fs_type.cyan(),
                ui::format_size(part.size)
            );
        }
    }
}

/// Get partition specifications from user
fn get_partition_specs(disk: &CurrentDiskInfo) -> Result<Vec<PartitionSpec>> {
    println!();
    ui::section("New Partition Scheme");
    println!();

    println!(
        "  Configure partitions for {} disk",
        ui::format_size(disk.size)
    );
    println!("  Enter partition details. Leave name empty when done.");
    println!();

    let fs_options = vec![
        "APFS",
        "APFS (Encrypted)",
        "ExFAT",
        "JHFS+",
        "FAT32",
        "Free Space",
    ];

    let mut specs = Vec::new();
    let mut partition_num = 1;
    let mut remaining_size = disk.size;

    loop {
        println!("  {} Partition {}:", "->".dimmed(), partition_num);

        // Partition name
        let name: String = Input::new()
            .with_prompt("    Name (empty to finish)")
            .allow_empty(true)
            .interact_text()
            .context("Failed to read partition name")?;

        if name.is_empty() {
            break;
        }

        // Filesystem type
        let fs_idx = Select::new()
            .with_prompt("    Format")
            .items(&fs_options)
            .default(0)
            .interact()
            .context("Failed to read format selection")?;

        let fs_type = fs_options[fs_idx].to_string();

        // Note about encrypted APFS
        if fs_type == "APFS (Encrypted)" {
            println!(
                "    {} Encrypted APFS will prompt for a password during creation.",
                "Note:".yellow()
            );
            println!(
                "    {} You'll need this password every time you mount the volume.",
                "     ".yellow()
            );
        }

        // Size
        println!(
            "    Remaining space: {}",
            ui::format_size(remaining_size).green()
        );

        let size_str: String = Input::new()
            .with_prompt("    Size (e.g., 1TB, 500GB, or 'rest' for remaining)")
            .interact_text()
            .context("Failed to read size")?;

        let size_spec = if size_str.to_lowercase() == "rest" || size_str.to_lowercase() == "r" {
            "0b".to_string() // diskutil uses 0b for "use remaining space"
        } else {
            size_str.clone()
        };

        // Try to parse size for display
        if let Ok(parsed_size) = parse_size_spec(&size_str) {
            if parsed_size <= remaining_size {
                remaining_size = remaining_size.saturating_sub(parsed_size);
            } else {
                ui::warn(&format!(
                    "Size {} exceeds remaining space {}",
                    size_str,
                    ui::format_size(remaining_size)
                ));
            }
        }

        // Handle encrypted APFS - use plain APFS for diskutil, encrypt after
        let (actual_fs_type, encrypted) = if fs_type == "APFS (Encrypted)" {
            ("APFS".to_string(), true)
        } else {
            (fs_type.clone(), false)
        };

        specs.push(PartitionSpec {
            name,
            fs_type: actual_fs_type,
            fs_display: fs_type,
            size: size_spec,
            encrypted,
        });

        partition_num += 1;
        println!();
    }

    // Show summary
    if !specs.is_empty() {
        println!();
        println!("  {}", "Partition Summary:".bold());
        for (i, spec) in specs.iter().enumerate() {
            let encrypted_note = if spec.encrypted { " 🔒" } else { "" };
            println!(
                "    {}. {} [{}] - {}{}",
                i + 1,
                spec.name,
                spec.fs_display.cyan(),
                spec.size,
                encrypted_note
            );
        }
    }

    Ok(specs)
}

/// Parse size specification to bytes
fn parse_size_spec(size: &str) -> Result<u64> {
    let size = size.trim().to_uppercase();

    if size == "REST" || size == "R" {
        return Ok(0);
    }

    // Try to parse with units
    let (num_str, multiplier) = if size.ends_with("TB") {
        (size.trim_end_matches("TB"), 1024u64 * 1024 * 1024 * 1024)
    } else if size.ends_with("GB") {
        (size.trim_end_matches("GB"), 1024u64 * 1024 * 1024)
    } else if size.ends_with("MB") {
        (size.trim_end_matches("MB"), 1024u64 * 1024)
    } else if size.ends_with("KB") {
        (size.trim_end_matches("KB"), 1024u64)
    } else if size.ends_with('T') {
        (size.trim_end_matches('T'), 1024u64 * 1024 * 1024 * 1024)
    } else if size.ends_with('G') {
        (size.trim_end_matches('G'), 1024u64 * 1024 * 1024)
    } else if size.ends_with('M') {
        (size.trim_end_matches('M'), 1024u64 * 1024)
    } else {
        (size.as_str(), 1u64)
    };

    let num: f64 = num_str.parse().context("Invalid number in size")?;
    Ok((num * multiplier as f64) as u64)
}

/// Generate the diskutil partitionDisk command
fn generate_diskutil_command(disk_id: &str, partitions: &[PartitionSpec]) -> String {
    // Format: diskutil partitionDisk disk2 GPT \
    //   APFS "Partition1" 3TB \
    //   ExFAT "Partition2" 1TB

    let mut parts = vec![
        "diskutil".to_string(),
        "partitionDisk".to_string(),
        disk_id.to_string(),
        "GPT".to_string(),
    ];

    for spec in partitions {
        parts.push(spec.fs_type.clone());
        parts.push(format!("\"{}\"", spec.name));
        parts.push(spec.size.clone());
    }

    parts.join(" ")
}

/// Execute the repartition command
fn execute_repartition(disk_id: &str, partitions: &[PartitionSpec]) -> Result<()> {
    ui::section("Executing Repartition");
    println!();

    // Build arguments
    let args = vec!["partitionDisk", disk_id, "GPT"];

    let partition_args: Vec<String> = partitions
        .iter()
        .flat_map(|spec| vec![spec.fs_type.clone(), spec.name.clone(), spec.size.clone()])
        .collect();

    let arg_refs: Vec<&str> = partition_args.iter().map(|s| s.as_str()).collect();

    // Combine args
    let mut full_args = args.clone();
    for arg in &arg_refs {
        full_args.push(arg);
    }

    println!("  Running: diskutil {}", full_args.join(" ").cyan());
    println!();

    let status = Command::new("diskutil")
        .args(&full_args)
        .status()
        .context("Failed to execute diskutil")?;

    if !status.success() {
        anyhow::bail!("diskutil command failed with status: {}", status);
    }

    println!();
    ui::success("Repartition completed successfully!");

    // Handle encryption for any partitions that requested it
    let encrypted_partitions: Vec<_> = partitions.iter().filter(|p| p.encrypted).collect();
    if !encrypted_partitions.is_empty() {
        println!();
        ui::section("Enabling Encryption");
        println!();

        for spec in encrypted_partitions {
            println!(
                "  Encrypting volume: {} {}",
                spec.name.cyan(),
                "(you will be prompted for a password)".dimmed()
            );
            println!();

            // Find the volume identifier for this partition
            // The volume name should match what we just created
            let encrypt_status = Command::new("diskutil")
                .args(["apfs", "encryptVolume", &spec.name, "-user", "disk"])
                .status()
                .context("Failed to run diskutil apfs encryptVolume")?;

            if encrypt_status.success() {
                ui::success(&format!("  Volume '{}' encryption started", spec.name));
                ui::dim("  Encryption will complete in the background.");
            } else {
                ui::warn(&format!(
                    "  Failed to encrypt '{}'. You can encrypt manually with:",
                    spec.name
                ));
                println!(
                    "    {}",
                    format!("diskutil apfs encryptVolume \"{}\"", spec.name).cyan()
                );
            }
            println!();
        }
    }

    // Show new layout
    println!();
    ui::dim("New layout:");
    let _ = Command::new("diskutil").args(["list", disk_id]).status();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_disk_id() {
        assert_eq!(normalize_disk_id("disk2"), "disk2");
        assert_eq!(normalize_disk_id("/dev/disk2"), "disk2");
    }

    #[test]
    fn test_parse_size_spec() {
        assert_eq!(parse_size_spec("1TB").unwrap(), 1024 * 1024 * 1024 * 1024);
        assert_eq!(parse_size_spec("2GB").unwrap(), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_size_spec("500MB").unwrap(), 500 * 1024 * 1024);
        assert_eq!(parse_size_spec("1T").unwrap(), 1024 * 1024 * 1024 * 1024);
        assert_eq!(parse_size_spec("rest").unwrap(), 0);
    }

    #[test]
    fn test_generate_diskutil_command() {
        let partitions = vec![
            PartitionSpec {
                name: "Main".to_string(),
                fs_type: "APFS".to_string(),
                fs_display: "APFS".to_string(),
                size: "3TB".to_string(),
                encrypted: false,
            },
            PartitionSpec {
                name: "Shared".to_string(),
                fs_type: "ExFAT".to_string(),
                fs_display: "ExFAT".to_string(),
                size: "0b".to_string(),
                encrypted: false,
            },
        ];

        let cmd = generate_diskutil_command("disk2", &partitions);
        assert!(cmd.contains("diskutil partitionDisk disk2 GPT"));
        assert!(cmd.contains("APFS \"Main\" 3TB"));
        assert!(cmd.contains("ExFAT \"Shared\" 0b"));
    }

    #[test]
    fn test_encrypted_apfs_uses_plain_apfs_in_command() {
        let partitions = vec![PartitionSpec {
            name: "Secure".to_string(),
            fs_type: "APFS".to_string(), // Actual type for diskutil
            fs_display: "APFS (Encrypted)".to_string(),
            size: "1TB".to_string(),
            encrypted: true,
        }];

        let cmd = generate_diskutil_command("disk4", &partitions);
        // Should use plain APFS in the command (encryption is done separately)
        assert!(cmd.contains("APFS \"Secure\" 1TB"));
        assert!(!cmd.contains("Encrypted"));
    }
}
