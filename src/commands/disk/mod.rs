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

impl From<crate::cli::DiskCommand> for DiskCommand {
    fn from(cmd: crate::cli::DiskCommand) -> Self {
        match cmd {
            crate::cli::DiskCommand::Status => Self::Status,
            crate::cli::DiskCommand::Backup {
                source,
                destination,
                dry_run,
            } => Self::Backup {
                source,
                destination,
                dry_run,
            },
            crate::cli::DiskCommand::Repartition {
                disk,
                dry_run,
                confirm,
            } => Self::Repartition {
                disk,
                dry_run,
                confirm,
            },
        }
    }
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

#[cfg(test)]
mod tests {
    use super::DiskCommand;
    use crate::cli::DiskCommand as CliDiskCommand;

    #[test]
    fn cli_disk_backup_maps_fields() {
        let cli_cmd = CliDiskCommand::Backup {
            source: "/tmp/source".to_string(),
            destination: "/tmp/destination".to_string(),
            dry_run: true,
        };

        let mapped: DiskCommand = cli_cmd.into();
        match mapped {
            DiskCommand::Backup {
                source,
                destination,
                dry_run,
            } => {
                assert_eq!(source, "/tmp/source");
                assert_eq!(destination, "/tmp/destination");
                assert!(dry_run);
            }
            _ => panic!("expected backup mapping"),
        }
    }
}
