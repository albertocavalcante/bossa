use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::time::Duration;

/// Create a spinner for indeterminate progress
pub fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Create a progress bar for known-length operations
pub fn bar(len: u64, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{msg}\n{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("━━╸"),
    );
    pb.set_message(msg.to_string());
    pb
}

/// Create a multi-progress container
pub fn multi() -> MultiProgress {
    MultiProgress::new()
}

/// Create a progress bar for parallel clone operations
pub fn clone_bar(len: u64) -> ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{prefix:.bold.dim} {spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {wide_msg}",
            )
            .unwrap()
            .progress_chars("━━╸"),
    );
    pb.set_prefix("Cloning");
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}

/// Style for individual clone tasks in parallel mode
pub fn clone_item_style() -> ProgressStyle {
    ProgressStyle::default_spinner()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
        .template("  {spinner:.cyan} {wide_msg}")
        .unwrap()
}

/// Finish a progress bar with success
pub fn finish_success(pb: &ProgressBar, msg: &str) {
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{msg}")
            .unwrap(),
    );
    pb.finish_with_message(format!("✓ {}", msg));
}

/// Finish a progress bar with error
pub fn finish_error(pb: &ProgressBar, msg: &str) {
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{msg}")
            .unwrap(),
    );
    pb.finish_with_message(format!("✗ {}", msg));
}

/// Finish a progress bar with warning
pub fn finish_warn(pb: &ProgressBar, msg: &str) {
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{msg}")
            .unwrap(),
    );
    pb.finish_with_message(format!("⚠ {}", msg));
}

/// Stage progress for multi-step operations like nova
pub struct StageProgress {
    current: usize,
    total: usize,
}

impl StageProgress {
    pub fn new(total: usize) -> Self {
        Self { current: 0, total }
    }

    pub fn next(&mut self, name: &str) -> ProgressBar {
        self.current += 1;
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
                .template(&format!(
                    "{{spinner:.cyan}} [{{pos}}/{}] {{msg}}",
                    self.total
                ))
                .unwrap(),
        );
        pb.set_position(self.current as u64);
        pb.set_message(name.to_string());
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    }

    pub fn skip(&mut self, name: &str) {
        self.current += 1;
        println!(
            "  {} [{}/{}] {} (skipped)",
            "○".dimmed(),
            self.current,
            self.total,
            name
        );
    }
}

use colored::Colorize;

impl std::fmt::Display for StageProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}/{}]", self.current, self.total)
    }
}
