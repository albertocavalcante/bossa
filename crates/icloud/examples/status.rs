//! Example: Check status of iCloud files
//!
//! Run with: cargo run -p icloud --example status

use icloud::Client;

fn main() -> icloud::Result<()> {
    let client = Client::new()?;

    // Get iCloud root
    let root = client.icloud_root()?;
    println!("iCloud Drive root: {}", root.display());

    // List files in iCloud Drive root
    println!("\nFiles in iCloud Drive:");
    println!("{:-<60}", "");

    let files = client.list(&root)?;
    for file in &files {
        let state = match file.state {
            icloud::DownloadState::Local => "LOCAL ",
            icloud::DownloadState::Cloud => "CLOUD ",
            icloud::DownloadState::Downloading { percent } => &format!("DL {percent}%"),
            icloud::DownloadState::Uploading { percent } => &format!("UP {percent}%"),
            icloud::DownloadState::Unknown => "???   ",
        };

        let size = file
            .size
            .map(format_size)
            .unwrap_or_else(|| "    -".to_string());

        let type_indicator = if file.is_dir { "📁" } else { "📄" };

        println!(
            "{} {:>8} {} {}",
            state,
            size,
            type_indicator,
            file.path.file_name().unwrap_or_default().to_string_lossy()
        );
    }

    // Find large evictable files
    println!("\n\nLarge local files (>100MB) that could be evicted:");
    println!("{:-<60}", "");

    let evictable = client.find_evictable(&root, 100 * 1024 * 1024)?;
    if evictable.is_empty() {
        println!("(none found)");
    } else {
        for file in evictable {
            println!(
                "{:>10} {}",
                format_size(file.size.unwrap_or(0)),
                file.path.display()
            );
        }
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}K", bytes as f64 / KB as f64)
    } else {
        format!("{bytes}B")
    }
}
