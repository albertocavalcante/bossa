//! Terminal rendering and native adapters for the shared resource application.

use anyhow::Result;
use colored::Colorize;
use declarative::{ApplyResult, ConfirmCallback, ExecuteSummary, ProgressCallback, ResourceDiff};

use super::ExecutionPlan;
use super::differ::{display_diff, display_sudo_boundary};
use crate::sudo::SudoContext;

pub use bossa::ResourceApplyOptions as ExecuteOptions;

struct TerminalConfirm;
impl ConfirmCallback for TerminalConfirm {
    fn confirm(&mut self, prompt: &str) -> Result<bool> {
        Ok(dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()?)
    }
}

struct TerminalProgress {
    quiet: bool,
}
impl ProgressCallback for TerminalProgress {
    fn on_plan(&mut self, diffs: &[ResourceDiff]) {
        if self.quiet {
            return;
        }
        display_diff(diffs);
        display_sudo_boundary(
            &diffs
                .iter()
                .filter(|diff| diff.requires_sudo)
                .cloned()
                .collect::<Vec<_>>(),
        );
    }
    fn on_batch_start(&mut self, _: usize, _: bool) {}
    fn on_resource_start(&mut self, id: &str, _: &str) {
        if !self.quiet {
            println!("  Applying {id}");
        }
    }
    fn on_resource_complete(&mut self, _: &str, _: &ApplyResult) {}
    fn on_batch_complete(&mut self) {}
}

pub fn execute(
    plan: ExecutionPlan,
    options: ExecuteOptions,
    quiet: bool,
) -> Result<ExecuteSummary> {
    let runner = bossa::SystemRunner::new(crate::paths::environment().variables().clone());
    let summary = bossa::apply_resources(
        plan,
        options,
        &runner,
        &std::env::current_dir()?,
        || SudoContext::acquire("Apply selected privileged configuration"),
        &mut TerminalProgress { quiet },
        &mut TerminalConfirm,
    )?;
    for row in &summary.outcomes {
        if let Some(diagnostic) = &row.diagnostic
            && (!quiet
                || row.verification == declarative::VerificationStatus::Failed
                || matches!(
                    row.execution,
                    declarative::ExecutionStatus::Blocked
                        | declarative::ExecutionStatus::Cancelled
                        | declarative::ExecutionStatus::Failed
                ))
        {
            eprintln!(
                "  {} {}: {diagnostic}",
                row.resource.resource_type, row.resource.id
            );
        }
    }
    if summary.preview && !quiet {
        println!("  Preview: no changes attempted");
    } else if summary.is_success() && !quiet {
        println!(
            "  {} Selected postconditions verified (no persisted finish receipt)",
            "✓".green()
        );
    } else if !summary.is_success() {
        eprintln!(
            "  Incomplete: {} failed, {} blocked, {} cancelled; possible changes are retained",
            summary.failed, summary.blocked, summary.cancelled
        );
    }
    Ok(summary)
}
