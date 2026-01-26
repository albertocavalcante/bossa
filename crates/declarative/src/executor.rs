//! Execution engine - applies resources with parallelism and privilege batching

use crate::context::{ApplyContext, ConfirmCallback, ProgressCallback, SudoProvider};
use crate::diff::compute_diffs;
use crate::planner::ExecutionPlan;
use crate::resource::Resource;
use crate::types::{ApplyResult, ExecuteOptions, ExecuteSummary};
use anyhow::Result;
use rayon::prelude::*;
use std::sync::{Arc, Mutex};

/// Execute a plan with the given options and callbacks
///
/// # Type Parameters
/// * `S` - Sudo provider type
/// * `P` - Progress callback type
/// * `C` - Confirm callback type
///
/// # Arguments
/// * `plan` - The execution plan to run
/// * `opts` - Execution options (dry_run, jobs, verbose)
/// * `sudo_provider` - Provider for privileged operations (called lazily if needed)
/// * `progress` - Progress callback
/// * `confirm` - Confirmation callback
///
/// # Returns
/// Summary of execution results
pub fn execute<S, P, C>(
    plan: ExecutionPlan,
    opts: ExecuteOptions,
    sudo_provider: impl FnOnce() -> Result<S>,
    progress: &mut P,
    confirm: &mut C,
) -> Result<ExecuteSummary>
where
    S: SudoProvider,
    P: ProgressCallback,
    C: ConfirmCallback,
{
    // Compute diffs for reporting
    let unprivileged_diffs = compute_diffs(&plan.unprivileged);
    let privileged_diffs = compute_diffs(&plan.privileged);
    let total_changes = unprivileged_diffs.len() + privileged_diffs.len();

    if total_changes == 0 {
        return Ok(ExecuteSummary::default());
    }

    // Confirm before proceeding (unless dry_run)
    if !opts.dry_run && !confirm.confirm("Apply changes?")? {
        return Ok(ExecuteSummary {
            skipped: total_changes,
            ..Default::default()
        });
    }

    if opts.dry_run {
        return Ok(ExecuteSummary::default());
    }

    let mut summary = ExecuteSummary::default();

    // Execute unprivileged resources in parallel
    if !plan.unprivileged.is_empty() {
        progress.on_batch_start(plan.unprivileged.len(), false);
        let results = execute_batch(&plan.unprivileged, opts.jobs, opts.verbose, None, progress)?;
        for result in &results {
            summary.add_result(result);
        }
        progress.on_batch_complete();
    }

    // Execute privileged resources (sequentially, with sudo)
    if !plan.privileged.is_empty() {
        // Acquire sudo only when needed
        let sudo = sudo_provider()?;

        progress.on_batch_start(plan.privileged.len(), true);
        let results = execute_batch(
            &plan.privileged,
            1, // Sequential for sudo
            opts.verbose,
            Some(&sudo),
            progress,
        )?;
        for result in &results {
            summary.add_result(result);
        }
        progress.on_batch_complete();
    }

    Ok(summary)
}

/// Execute a batch of resources
fn execute_batch<P: ProgressCallback>(
    resources: &[Box<dyn Resource>],
    jobs: usize,
    verbose: bool,
    sudo: Option<&dyn SudoProvider>,
    progress: &mut P,
) -> Result<Vec<ApplyResult>> {
    if jobs == 1 || resources.len() == 1 {
        // Sequential execution
        let mut results = Vec::with_capacity(resources.len());
        for resource in resources {
            progress.on_resource_start(&resource.id(), &resource.description());
            let result = apply_resource(resource.as_ref(), verbose, sudo);
            progress.on_resource_complete(&resource.id(), &result);
            results.push(result);
        }
        Ok(results)
    } else {
        // Parallel execution
        execute_parallel(resources, jobs, verbose, sudo, progress)
    }
}

/// Execute resources in parallel using rayon
fn execute_parallel<P: ProgressCallback>(
    resources: &[Box<dyn Resource>],
    jobs: usize,
    verbose: bool,
    sudo: Option<&dyn SudoProvider>,
    progress: &mut P,
) -> Result<Vec<ApplyResult>> {
    // For parallel execution, we can't use the progress callback during iteration
    // because it's not thread-safe. We collect results and report after.
    let results: Arc<Mutex<Vec<(String, ApplyResult)>>> = Arc::new(Mutex::new(Vec::new()));

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create thread pool: {}", e))?;

    pool.install(|| {
        resources.par_iter().for_each(|resource| {
            let result = apply_resource(resource.as_ref(), verbose, sudo);
            results.lock().unwrap().push((resource.id(), result));
        });
    });

    let results = Arc::try_unwrap(results)
        .map_err(|_| anyhow::anyhow!("Failed to unwrap results"))?
        .into_inner()
        .unwrap();

    // Report results to progress callback
    for (id, result) in &results {
        progress.on_resource_complete(id, result);
    }

    Ok(results.into_iter().map(|(_, r)| r).collect())
}

/// Apply a single resource
fn apply_resource(
    resource: &dyn Resource,
    verbose: bool,
    sudo: Option<&dyn SudoProvider>,
) -> ApplyResult {
    let mut ctx = match sudo {
        Some(s) => ApplyContext::with_sudo(false, verbose, s),
        None => ApplyContext::new(false, verbose),
    };

    match resource.apply(&mut ctx) {
        Ok(result) => result,
        Err(e) => ApplyResult::Failed {
            error: e.to_string(),
        },
    }
}

/// Simple execution without callbacks
///
/// For basic use cases where you don't need progress or confirmation.
pub fn execute_simple<S: SudoProvider>(
    plan: ExecutionPlan,
    opts: ExecuteOptions,
    sudo_provider: impl FnOnce() -> Result<S>,
) -> Result<ExecuteSummary> {
    use crate::context::{AutoConfirm, NoProgress};

    execute(plan, opts, sudo_provider, &mut NoProgress, &mut AutoConfirm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{AutoConfirm, NoProgress};
    use crate::types::{CommandOutput, ResourceState};

    /// Mock sudo provider for tests
    struct MockSudo;

    impl SudoProvider for MockSudo {
        fn run(&self, _cmd: &str, _args: &[&str]) -> Result<CommandOutput> {
            Ok(CommandOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
                success: true,
            })
        }
    }

    #[derive(Debug)]
    struct TestResource {
        id: String,
        should_change: bool,
    }

    impl Resource for TestResource {
        fn id(&self) -> String {
            self.id.clone()
        }

        fn description(&self) -> String {
            format!("Test resource {}", self.id)
        }

        fn resource_type(&self) -> &'static str {
            "test"
        }

        fn current_state(&self) -> Result<ResourceState> {
            if self.should_change {
                Ok(ResourceState::Absent)
            } else {
                Ok(ResourceState::Present { details: None })
            }
        }

        fn desired_state(&self) -> ResourceState {
            ResourceState::Present { details: None }
        }

        fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
            if ctx.dry_run {
                return Ok(ApplyResult::Skipped {
                    reason: "Dry run".into(),
                });
            }
            if self.should_change {
                Ok(ApplyResult::Created)
            } else {
                Ok(ApplyResult::NoChange)
            }
        }
    }

    #[test]
    fn test_execute_empty_plan() {
        let plan = ExecutionPlan::new();
        let opts = ExecuteOptions::default();
        let result = execute(
            plan,
            opts,
            || -> Result<MockSudo> { Ok(MockSudo) },
            &mut NoProgress,
            &mut AutoConfirm,
        )
        .unwrap();

        assert_eq!(result.total(), 0);
    }

    #[test]
    fn test_execute_no_changes() {
        let mut plan = ExecutionPlan::new();
        plan.unprivileged.push(Box::new(TestResource {
            id: "test1".into(),
            should_change: false,
        }));

        let opts = ExecuteOptions::default();
        let result = execute(
            plan,
            opts,
            || -> Result<MockSudo> { Ok(MockSudo) },
            &mut NoProgress,
            &mut AutoConfirm,
        )
        .unwrap();

        // No diff means no execution
        assert_eq!(result.total(), 0);
    }

    #[test]
    fn test_execute_with_changes() {
        let mut plan = ExecutionPlan::new();
        plan.unprivileged.push(Box::new(TestResource {
            id: "test1".into(),
            should_change: true,
        }));

        let opts = ExecuteOptions::default();
        let result = execute(
            plan,
            opts,
            || -> Result<MockSudo> { Ok(MockSudo) },
            &mut NoProgress,
            &mut AutoConfirm,
        )
        .unwrap();

        assert_eq!(result.created, 1);
    }
}
