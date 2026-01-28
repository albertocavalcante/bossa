//! Disk management commands
//!
//! Commands:
//! - status: List all disks with partitions and space info
//! - backup: Copy directory with progress, skipping system files
//! - repartition: Interactive guided repartition for external drives
//!
//! # Safety
//!
//! The repartition command includes multiple safety measures:
//! - Refuses to operate on internal/boot disks
//! - Requires explicit `--confirm` flag to execute
//! - Supports `--dry-run` to preview operations

mod backup;
mod plist;
mod repartition;
mod status;

use anyhow::Result;

/// Disk command variants (matches cli::DiskCommand)
pub enum DiskCommand {
    Status,
    Backup {
        source: String,
        destination: String,
        dry_run: bool,
    },
    Repartition {
        disk: String,
        dry_run: bool,
        confirm: bool,
    },
}

/// Run a disk command
pub fn run(cmd: DiskCommand) -> Result<()> {
    match cmd {
        DiskCommand::Status => status::run(),
        DiskCommand::Backup {
            source,
            destination,
            dry_run,
        } => backup::run(&source, &destination, dry_run),
        DiskCommand::Repartition {
            disk,
            dry_run,
            confirm,
        } => repartition::run(&disk, dry_run, confirm),
    }
}
