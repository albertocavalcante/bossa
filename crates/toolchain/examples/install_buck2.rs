//! Example: Install Buck2
//!
//! Run with: cargo run -p toolchain --example install_buck2

use toolchain::{Client, InstallOptions, Tool};

fn main() {
    println!("Buck2 Installer");
    println!("===============\n");

    let client = Client::new();

    // Check if already installed
    match client.is_installed(Tool::Buck2) {
        Ok(true) => {
            println!("Buck2 is already installed.");
            if let Ok(Some(version)) = client.version(Tool::Buck2) {
                println!("Current version: {}", version);
            }
            println!("\nUse --force to reinstall.");
        }
        Ok(false) => {
            println!("Buck2 is not installed. Installing...");
        }
        Err(e) => {
            println!("Error checking installation: {}", e);
        }
    }

    // Install (force for demo)
    println!("\nInstalling Buck2 (latest)...");

    let options = InstallOptions::default().force(true);

    match client.install(Tool::Buck2, options) {
        Ok(result) => {
            println!("\nInstallation successful!");
            println!("  Tool:    {}", result.tool);
            println!("  Version: {}", result.version);
            println!("  Path:    {}", result.path.display());

            if result.was_upgrade {
                if let Some(prev) = result.previous_version {
                    println!("  Upgraded from: {}", prev);
                }
            }
        }
        Err(e) => {
            eprintln!("\nInstallation failed: {}", e);
            std::process::exit(1);
        }
    }
}
