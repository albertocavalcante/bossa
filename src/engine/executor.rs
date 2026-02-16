//! Execution engine - bossa-specific executor with UI integration

use anyhow::{Context as AnyhowContext, Result};
use colored::Colorize;
use rayon::prelude::*;
use std::sync::Arc;

use crate::progress;
use crate::resource::{ApplyContext, ApplyResult, Resource};
use crate::sudo::SudoContext;
use declarative::{ExecutionPlan, SudoProvider};

use super::differ::{compute_diffs, display_diff, display_sudo_boundary};

/// Options for execution (bossa-specific, includes `yes` for confirmation skip)
#[derive(Debug, Clone)]
pub struct ExecuteOptions {
    /// Don't make changes, just show what would happen
    pub dry_run: bool,
    /// Number of parallel jobs
    pub jobs: usize,
    /// Skip confirmation prompts
    pub yes: bool,
    /// Verbose output
    pub verbose: bool,
}

impl Default for ExecuteOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            jobs: 4,
            yes: false,
            verbose: false,
        }
    }
}

/// Summary of execution results
#[derive(Debug, Default)]
pub struct ExecuteSummary {
    pub created: usize,
    pub modified: usize,
    pub removed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub no_change: usize,
}

impl ExecuteSummary {
    pub fn total_changes(&self) -> usize {
        self.created + self.modified + self.removed
    }

    pub fn is_success(&self) -> bool {
        self.failed == 0
    }
}

/// Execute the plan with bossa's UI integration
pub fn execute(plan: ExecutionPlan, opts: ExecuteOptions) -> Result<ExecuteSummary> {
    // 1. Compute diffs for all resources
    let unprivileged_diffs = compute_diffs(&plan.unprivileged);
    let privileged_diffs = compute_diffs(&plan.privileged);
    let all_diffs: Vec<_> = unprivileged_diffs
        .iter()
        .chain(privileged_diffs.iter())
        .cloned()
        .collect();

    // 2. Display what will change
    display_diff(&all_diffs);

    if all_diffs.is_empty() {
        return Ok(ExecuteSummary::default());
    }

    // 3. Confirm (unless --yes)
    if !opts.yes && !opts.dry_run && !confirm_proceed()? {
        println!();
        println!("  {} Aborted", "✗".red());
        return Ok(ExecuteSummary {
            skipped: all_diffs.len(),
            ..Default::default()
        });
    }

    if opts.dry_run {
        println!();
        println!("  {} Dry run - no changes made", "ℹ".blue());
        return Ok(ExecuteSummary::default());
    }

    let mut summary = ExecuteSummary::default();

    // 4. Execute unprivileged in parallel
    if !plan.unprivileged.is_empty() {
        println!();
        println!(
            "  {} Applying {} unprivileged resources...",
            "→".cyan(),
            plan.unprivileged.len()
        );

        let results = execute_parallel(&plan.unprivileged, opts.jobs, opts.verbose, None)?;
        merge_summary(&mut summary, &results);
    }

    // 5. If any privileged operations, acquire sudo ONCE
    if !plan.privileged.is_empty() {
        display_sudo_boundary(&privileged_diffs);

        if !opts.yes && !confirm_proceed()? {
            summary.skipped += plan.privileged.len();
            return Ok(summary);
        }

        let sudo = SudoContext::acquire("Apply privileged system configuration")?;

        println!();
        println!(
            "  {} Applying {} privileged resources...",
            "→".cyan(),
            plan.privileged.len()
        );

        let results = execute_parallel(&plan.privileged, 1, opts.verbose, Some(&sudo))?; // Sequential for sudo
        merge_summary(&mut summary, &results);

        // sudo dropped here automatically
    }

    // 6. Restart services
    if !plan.post_actions.is_empty() {
        println!();
        println!("  {} Restarting services...", "→".cyan());
        for service in &plan.post_actions {
            restart_service(service)?;
        }
    }

    // 7. Summary
    print_summary(&summary);

    Ok(summary)
}

/// Execute resources in parallel
fn execute_parallel(
    resources: &[Box<dyn Resource>],
    jobs: usize,
    verbose: bool,
    sudo: Option<&SudoContext>,
) -> Result<Vec<ApplyResult>> {
    let pb = progress::clone_bar(resources.len() as u64, "Applying");
    let results: Arc<std::sync::Mutex<Vec<ApplyResult>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .context("Failed to create apply thread pool")?;

    pool.install(|| {
        resources.par_iter().for_each(|resource| {
            // Convert SudoContext to trait object for ApplyContext
            let sudo_provider: Option<&dyn SudoProvider> = sudo.map(|s| s as &dyn SudoProvider);

            let mut ctx = ApplyContext {
                dry_run: false,
                verbose,
                sudo: sudo_provider,
            };

            let result = match resource.apply(&mut ctx) {
                Ok(r) => r,
                Err(e) => ApplyResult::Failed {
                    error: e.to_string(),
                },
            };

            let symbol = match &result {
                ApplyResult::NoChange => "○",
                ApplyResult::Created | ApplyResult::Modified | ApplyResult::Removed => "✓",
                ApplyResult::Failed { .. } => "✗",
                ApplyResult::Skipped { .. } => "⊘",
            };

            pb.set_message(format!("{} {}", symbol, resource.id()));
            pb.inc(1);

            push_apply_result(&results, result);
        });
    });

    pb.finish_and_clear();

    into_apply_results(results)
}

fn push_apply_result(results: &Arc<std::sync::Mutex<Vec<ApplyResult>>>, result: ApplyResult) {
    match results.lock() {
        Ok(mut locked) => locked.push(result),
        Err(poisoned) => poisoned.into_inner().push(result),
    }
}

fn into_apply_results(
    results: Arc<std::sync::Mutex<Vec<ApplyResult>>>,
) -> Result<Vec<ApplyResult>> {
    let mutex = Arc::try_unwrap(results)
        .map_err(|_| anyhow::anyhow!("Failed to collect apply results: shared result state"))?;

    match mutex.into_inner() {
        Ok(collected) => Ok(collected),
        Err(poisoned) => Ok(poisoned.into_inner()),
    }
}

/// Merge results into summary
fn merge_summary(summary: &mut ExecuteSummary, results: &[ApplyResult]) {
    for result in results {
        match result {
            ApplyResult::NoChange => summary.no_change += 1,
            ApplyResult::Created => summary.created += 1,
            ApplyResult::Modified => summary.modified += 1,
            ApplyResult::Removed => summary.removed += 1,
            ApplyResult::Failed { .. } => summary.failed += 1,
            ApplyResult::Skipped { .. } => summary.skipped += 1,
        }
    }
}

/// Confirm with user
fn confirm_proceed() -> Result<bool> {
    use dialoguer::Confirm;

    let confirmed = Confirm::new()
        .with_prompt("Continue?")
        .default(true)
        .interact()?;

    Ok(confirmed)
}

/// Restart a macOS service
fn restart_service(service: &str) -> Result<()> {
    use std::process::Command;

    let status = Command::new("killall").arg(service).status()?;

    if status.success() {
        println!("    {} Restarted {}", "✓".green(), service);
    } else {
        println!(
            "    {} Could not restart {} (may not be running)",
            "⚠".yellow(),
            service
        );
    }

    Ok(())
}

/// Print final summary
fn print_summary(summary: &ExecuteSummary) {
    println!();
    if summary.is_success() {
        println!(
            "  {} Configuration applied successfully!",
            "✓".green().bold()
        );
    } else {
        println!(
            "  {} Configuration applied with errors",
            "⚠".yellow().bold()
        );
    }

    if summary.created > 0 {
        println!("    • {} resources created", summary.created);
    }
    if summary.modified > 0 {
        println!("    • {} resources modified", summary.modified);
    }
    if summary.removed > 0 {
        println!("    • {} resources removed", summary.removed);
    }
    if summary.skipped > 0 {
        println!("    • {} resources skipped", summary.skipped);
    }
    if summary.failed > 0 {
        println!("    • {} {} failed", summary.failed, "resources".red());
    }
}

#[cfg(test)]
mod tests {
    use super::{into_apply_results, push_apply_result};
    use crate::resource::ApplyResult;
    use std::sync::{Arc, Mutex};

    #[test]
    fn push_apply_result_handles_poisoned_mutex() {
        let results: Arc<Mutex<Vec<ApplyResult>>> = Arc::new(Mutex::new(Vec::new()));
        let poisoned = Arc::clone(&results);

        let _ = std::thread::spawn(move || {
            let _guard = poisoned
                .lock()
                .expect("lock should succeed before poisoning");
            panic!("intentional poison");
        })
        .join();

        push_apply_result(&results, ApplyResult::NoChange);

        let len = match results.lock() {
            Ok(locked) => locked.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        };
        assert_eq!(len, 1);
    }

    #[test]
    fn into_apply_results_recovers_from_poisoned_mutex() {
        let results: Arc<Mutex<Vec<ApplyResult>>> = Arc::new(Mutex::new(Vec::new()));
        let poisoned = Arc::clone(&results);

        let _ = std::thread::spawn(move || {
            let mut guard = poisoned
                .lock()
                .expect("lock should succeed before poisoning");
            guard.push(ApplyResult::NoChange);
            panic!("intentional poison");
        })
        .join();

        let collected = into_apply_results(results).expect("poisoned mutex should be recovered");
        assert_eq!(collected.len(), 1);
    }
}
