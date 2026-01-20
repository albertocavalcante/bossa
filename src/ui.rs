use colored::Colorize;

/// Print an info message
pub fn info(msg: &str) {
    println!("{} {}", "ℹ".blue(), msg);
}

/// Print a success message
pub fn success(msg: &str) {
    println!("{} {}", "✓".green(), msg);
}

/// Print a warning message
pub fn warn(msg: &str) {
    println!("{} {}", "⚠".yellow(), msg);
}

/// Print an error message
pub fn error(msg: &str) {
    eprintln!("{} {}", "✗".red(), msg);
}

/// Print a dim/muted message
pub fn dim(msg: &str) {
    println!("  {}", msg.dimmed());
}

/// Print a header/title
pub fn header(title: &str) {
    println!();
    println!("{}", title.bold());
    println!("{}", "─".repeat(title.len()).dimmed());
}

/// Print a section header
pub fn section(title: &str) {
    println!();
    println!("{}", title.cyan().bold());
}

/// Print a key-value pair
pub fn kv(key: &str, value: &str) {
    println!("  {}: {}", key.dimmed(), value);
}

/// Print a step indicator
pub fn step(num: usize, total: usize, msg: &str) {
    println!("{} {}", format!("[{}/{}]", num, total).blue().bold(), msg);
}

/// Print the bossa banner
pub fn banner() {
    println!(
        "{}",
        r#"
  ██████╗  ██████╗ ███████╗███████╗ █████╗
  ██╔══██╗██╔═══██╗██╔════╝██╔════╝██╔══██╗
  ██████╔╝██║   ██║███████╗███████╗███████║
  ██╔══██╗██║   ██║╚════██║╚════██║██╔══██║
  ██████╔╝╚██████╔╝███████║███████║██║  ██║
  ╚═════╝  ╚═════╝ ╚══════╝╚══════╝╚═╝  ╚═╝
"#
        .cyan()
    );
}
