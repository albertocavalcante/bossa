//! Selected-scope application with explicit observation and verification.
//!
//! Execution is deliberately serial. A resource's permission to parallelize is
//! insufficient evidence that its mutable state is independent of other types.

use crate::context::{ApplyContext, ConfirmCallback, ProgressCallback, SudoProvider};
use crate::diff::ResourceDiff;
use crate::planner::ExecutionPlan;
use crate::resource::Resource;
use crate::types::{
    ActionOutcome, ApplyResult, EffectStatus, ExecuteOptions, ExecuteSummary, ExecutionStatus,
    ResourceKey, ResourceState, SudoRequirement, VerificationStatus,
};
use anyhow::{Result, bail};
use std::collections::HashSet;

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
    execute_inner(plan, opts, sudo_provider, progress, confirm, None)
}

/// The follow-up adapter returns success only when its documented postcondition
/// holds. No adapter or no selected parents blocks the plan before any effects.
pub fn execute_with_followups<S, P, C>(
    plan: ExecutionPlan,
    opts: ExecuteOptions,
    sudo_provider: impl FnOnce() -> Result<S>,
    progress: &mut P,
    confirm: &mut C,
    follow_up: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<ExecuteSummary>
where
    S: SudoProvider,
    P: ProgressCallback,
    C: ConfirmCallback,
{
    execute_inner(
        plan,
        opts,
        sudo_provider,
        progress,
        confirm,
        Some(follow_up),
    )
}

struct Observation<'a> {
    resource: &'a dyn Resource,
    privileged: bool,
    current: Option<ResourceState>,
    desired: ResourceState,
    issue: Option<String>,
}

fn key(resource: &dyn Resource) -> ResourceKey {
    ResourceKey {
        resource_type: resource.resource_type().into(),
        id: resource.id(),
    }
}

fn outcome(resource: ResourceKey) -> ActionOutcome {
    ActionOutcome {
        resource,
        execution: ExecutionStatus::NotStarted,
        effects: EffectStatus::None,
        verification: VerificationStatus::Pending,
        diagnostic: None,
    }
}

fn observe(resource: &dyn Resource, privileged: bool) -> Observation<'_> {
    let desired = resource.desired_state();
    let (current, mut issue) = match resource.current_state() {
        Ok(state) => (Some(state), None),
        Err(error) => (None, Some(format!("Observation unavailable: {error:#}"))),
    };
    if current == Some(ResourceState::Unknown) || desired == ResourceState::Unknown {
        issue = Some("Selected resource state is unknown".into());
    }
    let declared = matches!(
        resource.sudo_requirement(),
        SudoRequirement::Required { .. }
    );
    if privileged != declared {
        issue = Some("Plan privilege policy disagrees with the resource adapter; no elevation or apply attempted".into());
    }
    Observation {
        resource,
        privileged,
        current,
        desired,
        issue,
    }
}

fn block_pending(summary: &mut ExecuteSummary, status: ExecutionStatus, diagnostic: &str) {
    for row in &mut summary.outcomes {
        if row.execution == ExecutionStatus::NotStarted
            && row.verification == VerificationStatus::Pending
        {
            row.execution = status;
            if row.diagnostic.is_none() {
                row.diagnostic = Some(diagnostic.into());
            }
        }
    }
}

fn counts(summary: &mut ExecuteSummary, results: &[Option<ApplyResult>]) {
    summary.created = 0;
    summary.modified = 0;
    summary.removed = 0;
    summary.skipped = 0;
    summary.failed = 0;
    summary.no_change = 0;
    summary.blocked = 0;
    summary.cancelled = 0;
    for (row, result) in summary.outcomes.iter().zip(results) {
        if row.verification == VerificationStatus::Failed
            || row.execution == ExecutionStatus::Failed
        {
            summary.failed += 1;
            continue;
        }
        match row.execution {
            ExecutionStatus::Blocked => summary.blocked += 1,
            ExecutionStatus::Cancelled => summary.cancelled += 1,
            ExecutionStatus::NotStarted if row.verification == VerificationStatus::Pending => {
                summary.skipped += 1;
            }
            _ => match result {
                Some(ApplyResult::Created) => summary.created += 1,
                Some(ApplyResult::Modified) => summary.modified += 1,
                Some(ApplyResult::Removed) => summary.removed += 1,
                _ => summary.no_change += 1,
            },
        }
    }
}

fn verify(observation: &Observation<'_>, row: &mut ActionOutcome) -> bool {
    match observation.resource.current_state() {
        Ok(current) if current == observation.desired && current != ResourceState::Unknown => {
            row.verification = VerificationStatus::Satisfied;
            true
        }
        result => {
            row.verification = VerificationStatus::Failed;
            row.effects = EffectStatus::Unknown;
            row.diagnostic = Some(match result {
                Ok(_) => "Required postcondition is not satisfied; preserve possible partial changes and reobserve before retrying".into(),
                Err(error) => format!("Postcondition observation unavailable: {error:#}; effects may be partial"),
            });
            false
        }
    }
}

fn apply_one<P: ProgressCallback>(
    observation: &Observation<'_>,
    row: &mut ActionOutcome,
    options: &ExecuteOptions,
    sudo: Option<&dyn SudoProvider>,
    progress: &mut P,
) -> Option<ApplyResult> {
    let current = observation.resource.current_state();
    if !matches!(current, Ok(ref state) if Some(state) == observation.current.as_ref()) {
        row.execution = ExecutionStatus::Blocked;
        row.diagnostic = Some(
            "Observation changed or became unavailable since preflight; no apply attempted".into(),
        );
        return None;
    }
    progress.on_batch_start(1, observation.privileged);
    progress.on_resource_start(
        &observation.resource.id(),
        &observation.resource.description(),
    );
    let mut context = ApplyContext {
        dry_run: false,
        verbose: options.verbose,
        sudo: if observation.privileged { sudo } else { None },
    };
    let result = observation
        .resource
        .apply(&mut context)
        .unwrap_or_else(|error| ApplyResult::Failed {
            error: format!("{error:#}"),
        });
    progress.on_resource_complete(&observation.resource.id(), &result);
    progress.on_batch_complete();
    row.execution = ExecutionStatus::Completed;
    match &result {
        ApplyResult::Failed { error } => {
            row.execution = ExecutionStatus::Failed;
            row.effects = EffectStatus::Unknown;
            row.diagnostic = Some(format!(
                "{error}; effects may be partial; reobserve before retrying"
            ));
        }
        ApplyResult::Skipped { reason } => {
            row.execution = ExecutionStatus::Blocked;
            row.effects = EffectStatus::Unknown;
            row.diagnostic = Some(format!("Apply did not complete: {reason}"));
        }
        _ => {
            row.effects = if result.is_change() {
                EffectStatus::VerifiedChange
            } else {
                EffectStatus::None
            };
            verify(observation, row);
        }
    }
    Some(result)
}

fn apply_batch<P: ProgressCallback>(
    observations: &[Observation<'_>],
    privileged: bool,
    summary: &mut ExecuteSummary,
    results: &mut [Option<ApplyResult>],
    options: &ExecuteOptions,
    sudo: Option<&dyn SudoProvider>,
    progress: &mut P,
) -> bool {
    for (index, observation) in observations
        .iter()
        .enumerate()
        .filter(|(_, value)| value.privileged == privileged)
    {
        let row = &mut summary.outcomes[index];
        if row.verification == VerificationStatus::Satisfied {
            continue;
        }
        results[index] = apply_one(observation, row, options, sudo, progress);
        if row.execution != ExecutionStatus::Completed
            || row.verification != VerificationStatus::Satisfied
        {
            block_pending(
                summary,
                ExecutionStatus::Blocked,
                "Selected scope stopped after an incomplete action; prior effects are retained",
            );
            return false;
        }
    }
    true
}

fn verify_scope(observations: &[Observation<'_>], summary: &mut ExecuteSummary) -> bool {
    for (index, observation) in observations.iter().enumerate() {
        if summary.outcomes[index].verification == VerificationStatus::Satisfied {
            verify(observation, &mut summary.outcomes[index]);
        }
    }
    summary
        .outcomes
        .iter()
        .all(|row| row.verification != VerificationStatus::Failed)
}

struct PreparedScope<'a> {
    observations: Vec<Observation<'a>>,
    summary: ExecuteSummary,
    diffs: Vec<ResourceDiff>,
    results: Vec<Option<ApplyResult>>,
}

fn prepare_scope(plan: &ExecutionPlan, preview: bool, has_followup: bool) -> PreparedScope<'_> {
    let mut observations: Vec<_> = plan
        .unprivileged
        .iter()
        .map(|r| observe(r.as_ref(), false))
        .chain(plan.privileged.iter().map(|r| observe(r.as_ref(), true)))
        .collect();
    let mut selected = HashSet::new();
    for observation in &mut observations {
        if !selected.insert(key(observation.resource)) {
            observation.issue = Some("Duplicate resource identity in selected plan".into());
        }
    }
    let mut summary = ExecuteSummary {
        preview,
        ..Default::default()
    };
    let mut diffs = Vec::new();
    for observation in &observations {
        let mut row = outcome(key(observation.resource));
        row.diagnostic.clone_from(&observation.issue);
        if observation.issue.is_none() {
            if observation.current.as_ref() == Some(&observation.desired) {
                row.execution = ExecutionStatus::Completed;
                row.verification = VerificationStatus::Satisfied;
            } else {
                diffs.push(ResourceDiff {
                    resource_id: observation.resource.id(),
                    resource_type: observation.resource.resource_type().into(),
                    description: observation.resource.description(),
                    current: observation.current.clone().expect("successful observation"),
                    desired: observation.desired.clone(),
                    requires_sudo: observation.privileged,
                });
            }
        }
        summary.outcomes.push(row);
    }
    let mut outcome_keys = selected.clone();
    for action in &plan.post_actions {
        let mut row = outcome(ResourceKey {
            resource_type: "follow_up".into(),
            id: action.action.clone(),
        });
        if !outcome_keys.insert(row.resource.clone()) {
            row.diagnostic = Some("Duplicate follow-up identity in selected plan".into());
        } else if action.parents.is_empty()
            || action
                .parents
                .iter()
                .any(|parent| !selected.contains(parent))
        {
            row.diagnostic = Some("Follow-up has no valid selected parent binding".into());
        } else if !has_followup {
            row.diagnostic = Some("Selected follow-up has no execution adapter".into());
        }
        summary.outcomes.push(row);
    }
    let results = vec![None; summary.outcomes.len()];
    PreparedScope {
        observations,
        summary,
        diffs,
        results,
    }
}

type FollowUp<'a> = &'a mut dyn FnMut(&str) -> Result<()>;

fn apply_followups(
    plan: &ExecutionPlan,
    observations: &[Observation<'_>],
    summary: &mut ExecuteSummary,
    results: &mut [Option<ApplyResult>],
    mut follow_up: Option<FollowUp<'_>>,
) {
    for (offset, action) in plan.post_actions.iter().enumerate() {
        if !verify_scope(observations, summary) {
            block_pending(
                summary,
                ExecutionStatus::Blocked,
                "Follow-up scope changed or became unavailable; remaining follow-ups were not attempted",
            );
            break;
        }
        let index = observations.len() + offset;
        let changed = summary.outcomes[..observations.len()].iter().any(|row| {
            action.parents.contains(&row.resource)
                && row.effects == EffectStatus::VerifiedChange
                && row.verification == VerificationStatus::Satisfied
        });
        if !changed {
            summary.outcomes[index].verification = VerificationStatus::Satisfied;
            summary.outcomes[index].diagnostic =
                Some("No selected parent changed; follow-up not required".into());
            continue;
        }
        let result = follow_up.as_mut().expect("adapter preflighted")(&action.action);
        let row = &mut summary.outcomes[index];
        match result {
            Ok(()) => {
                row.execution = ExecutionStatus::Completed;
                row.effects = EffectStatus::VerifiedChange;
                row.verification = VerificationStatus::Satisfied;
                results[index] = Some(ApplyResult::Modified);
                if !verify_scope(observations, summary) {
                    block_pending(
                        summary,
                        ExecutionStatus::Blocked,
                        "Earlier follow-up invalidated the selected scope; remaining follow-ups were not attempted",
                    );
                    break;
                }
            }
            Err(error) => {
                row.execution = ExecutionStatus::Failed;
                row.effects = EffectStatus::Unknown;
                row.diagnostic = Some(format!(
                    "Follow-up failed: {error:#}; prior resource changes remain"
                ));
                block_pending(
                    summary,
                    ExecutionStatus::Blocked,
                    "Earlier follow-up failed",
                );
                break;
            }
        }
    }
}

fn execute_inner<S, P, C>(
    plan: ExecutionPlan,
    opts: ExecuteOptions,
    sudo_provider: impl FnOnce() -> Result<S>,
    progress: &mut P,
    confirm: &mut C,
    follow_up: Option<FollowUp<'_>>,
) -> Result<ExecuteSummary>
where
    S: SudoProvider,
    P: ProgressCallback,
    C: ConfirmCallback,
{
    if opts.jobs == 0 {
        bail!("The job count must be positive");
    }
    let PreparedScope {
        observations,
        mut summary,
        diffs,
        mut results,
    } = prepare_scope(&plan, opts.dry_run, follow_up.is_some());
    if summary.outcomes.iter().any(|row| row.diagnostic.is_some()) {
        block_pending(
            &mut summary,
            ExecutionStatus::Blocked,
            "Selected scope blocked by preflight; no changes attempted",
        );
        counts(&mut summary, &results);
        return Ok(summary);
    }
    progress.on_plan(&diffs);
    if opts.dry_run {
        counts(&mut summary, &results);
        return Ok(summary);
    }
    if !diffs.is_empty() {
        let decision = confirm.confirm("Apply the selected changes?");
        let (status, diagnostic) = match decision {
            Ok(true) => (None, String::new()),
            Ok(false) => (
                Some(ExecutionStatus::Cancelled),
                "Apply cancelled; no changes attempted".into(),
            ),
            Err(error) => (
                Some(ExecutionStatus::Blocked),
                format!("Confirmation unavailable: {error:#}; no changes attempted"),
            ),
        };
        if let Some(status) = status {
            block_pending(&mut summary, status, &diagnostic);
            counts(&mut summary, &results);
            return Ok(summary);
        }
    }
    let unprivileged_ok = apply_batch(
        &observations,
        false,
        &mut summary,
        &mut results,
        &opts,
        None,
        progress,
    );
    let scope_ok = verify_scope(&observations, &mut summary);
    if !scope_ok {
        block_pending(
            &mut summary,
            ExecutionStatus::Blocked,
            "Selected scope changed before privileged execution",
        );
    }
    let needs_sudo = observations.iter().enumerate().any(|(i, observation)| {
        observation.privileged && summary.outcomes[i].verification == VerificationStatus::Pending
    });
    if unprivileged_ok && scope_ok && needs_sudo {
        match sudo_provider() {
            Ok(sudo) => {
                apply_batch(
                    &observations,
                    true,
                    &mut summary,
                    &mut results,
                    &opts,
                    Some(&sudo),
                    progress,
                );
                // Drop the scoped provider before observations and follow-up callbacks.
            }
            Err(error) => block_pending(
                &mut summary,
                ExecutionStatus::Blocked,
                &format!(
                    "Privilege acquisition failed; prior unprivileged effects are retained: {error:#}"
                ),
            ),
        }
    }
    // Check the whole scope again: a later resource may affect an earlier one.
    verify_scope(&observations, &mut summary);
    counts(&mut summary, &results);
    if summary.failed > 0 || summary.blocked > 0 || summary.cancelled > 0 {
        block_pending(
            &mut summary,
            ExecutionStatus::Blocked,
            "Follow-up blocked by incomplete selected scope",
        );
        counts(&mut summary, &results);
        return Ok(summary);
    }
    apply_followups(&plan, &observations, &mut summary, &mut results, follow_up);
    verify_scope(&observations, &mut summary);
    counts(&mut summary, &results);
    Ok(summary)
}

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
        should_change: std::sync::atomic::AtomicBool,
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
            if self.should_change.load(std::sync::atomic::Ordering::SeqCst) {
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
            if self.should_change.load(std::sync::atomic::Ordering::SeqCst) {
                self.should_change
                    .store(false, std::sync::atomic::Ordering::SeqCst);
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
            should_change: false.into(),
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

        // An unchanged resource still has a verified observation in the report.
        assert_eq!(result.total(), 1);
        assert_eq!(result.no_change, 1);
    }

    #[test]
    fn test_execute_with_changes() {
        let mut plan = ExecutionPlan::new();
        plan.unprivileged.push(Box::new(TestResource {
            id: "test1".into(),
            should_change: true.into(),
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

#[cfg(test)]
mod outcome_regressions {
    use super::*;
    use crate::{AutoConfirm, AutoDecline, CommandOutput, NoProgress, ResourceState};

    struct NoSudo;
    impl SudoProvider for NoSudo {
        fn run(&self, _: &str, _: &[&str]) -> Result<CommandOutput> {
            panic!("fixture must never acquire or execute privileged operations")
        }
    }

    #[derive(Debug)]
    struct UnverifiedResource {
        observation_fails: bool,
    }
    impl Resource for UnverifiedResource {
        fn id(&self) -> String {
            "unverified".into()
        }
        fn description(&self) -> String {
            "fixture".into()
        }
        fn resource_type(&self) -> &'static str {
            "fixture"
        }
        fn current_state(&self) -> Result<ResourceState> {
            if self.observation_fails {
                anyhow::bail!("observation unavailable");
            }
            Ok(ResourceState::Absent)
        }
        fn desired_state(&self) -> ResourceState {
            ResourceState::Present { details: None }
        }
        fn apply(&self, _: &mut ApplyContext) -> Result<ApplyResult> {
            assert!(
                !self.observation_fails,
                "failed observation must block apply"
            );
            Ok(ApplyResult::Created)
        }
    }
    fn plan(observation_fails: bool) -> ExecutionPlan {
        let mut plan = ExecutionPlan::new();
        plan.unprivileged
            .push(Box::new(UnverifiedResource { observation_fails }));
        plan
    }
    #[test]
    fn failed_observation_is_never_a_successful_empty_apply() {
        let result = execute(
            plan(true),
            ExecuteOptions::default(),
            || Ok(NoSudo),
            &mut NoProgress,
            &mut AutoConfirm,
        )
        .unwrap();
        assert!(!result.is_success(), "unavailable observation was hidden");
    }
    #[test]
    fn declined_work_is_not_successful_completion() {
        let result = execute(
            plan(false),
            ExecuteOptions::default(),
            || Ok(NoSudo),
            &mut NoProgress,
            &mut AutoDecline,
        )
        .unwrap();
        assert!(!result.is_success(), "cancelled work was called successful");
    }
    #[test]
    fn successful_command_without_postcondition_is_not_applied() {
        let result = execute(
            plan(false),
            ExecuteOptions::default(),
            || Ok(NoSudo),
            &mut NoProgress,
            &mut AutoConfirm,
        )
        .unwrap();
        assert!(!result.is_success(), "unverified state was called applied");
    }
}

#[cfg(test)]
mod selected_scope_tests {
    use super::*;
    use crate::{AutoConfirm, AutoDecline, CommandOutput, NoProgress, SudoClassifier};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    type Trace = Arc<Mutex<Vec<String>>>;

    #[derive(Debug)]
    struct Stateful {
        id: &'static str,
        present: Arc<AtomicBool>,
        trace: Trace,
        unavailable: bool,
        unknown: bool,
        fail_after_effect: bool,
        change_precondition: bool,
        observations: AtomicUsize,
        invalidates: Option<Arc<AtomicBool>>,
        serial: bool,
        privileged: bool,
    }
    impl Stateful {
        fn new(id: &'static str, trace: &Trace) -> Self {
            Self {
                id,
                present: Arc::new(false.into()),
                trace: Arc::clone(trace),
                unavailable: false,
                unknown: false,
                fail_after_effect: false,
                change_precondition: false,
                observations: 0.into(),
                invalidates: None,
                serial: false,
                privileged: false,
            }
        }
    }
    impl Resource for Stateful {
        fn id(&self) -> String {
            self.id.into()
        }
        fn description(&self) -> String {
            self.id.into()
        }
        fn resource_type(&self) -> &'static str {
            "fixture"
        }
        fn current_state(&self) -> Result<ResourceState> {
            if self.unavailable {
                bail!("fixture read denied");
            }
            if self.unknown {
                return Ok(ResourceState::Unknown);
            }
            if self.change_precondition && self.observations.fetch_add(1, Ordering::SeqCst) > 0 {
                return Ok(ResourceState::Present {
                    details: Some("external change".into()),
                });
            }
            Ok(if self.present.load(Ordering::SeqCst) {
                self.desired_state()
            } else {
                ResourceState::Absent
            })
        }
        fn desired_state(&self) -> ResourceState {
            ResourceState::Present { details: None }
        }
        fn apply(&self, context: &mut ApplyContext) -> Result<ApplyResult> {
            assert_eq!(context.sudo.is_some(), self.privileged);
            self.trace.lock().unwrap().push(self.id.into());
            self.present.store(true, Ordering::SeqCst);
            if let Some(other) = &self.invalidates {
                other.store(false, Ordering::SeqCst);
            }
            if self.fail_after_effect {
                bail!(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "fixture timed out after write"
                ));
            }
            Ok(ApplyResult::Created)
        }
        fn sudo_requirement(&self) -> SudoRequirement {
            if self.privileged {
                SudoRequirement::Required {
                    reason: "fixture".into(),
                }
            } else {
                SudoRequirement::None
            }
        }
        fn can_parallelize(&self) -> bool {
            !self.serial
        }
    }
    struct NoSudo;
    impl SudoProvider for NoSudo {
        fn run(&self, _: &str, _: &[&str]) -> Result<CommandOutput> {
            panic!("unexpected privilege effect")
        }
    }
    fn no_sudo() -> Result<NoSudo> {
        panic!("unexpected privilege acquisition")
    }
    fn trace() -> Trace {
        Arc::new(Mutex::new(Vec::new()))
    }
    fn plan(resources: Vec<Stateful>) -> ExecutionPlan {
        let mut plan = ExecutionPlan::new();
        plan.unprivileged = resources
            .into_iter()
            .map(|value| Box::new(value) as Box<dyn Resource>)
            .collect();
        plan
    }
    fn parent(id: &str) -> ResourceKey {
        ResourceKey {
            resource_type: "fixture".into(),
            id: id.into(),
        }
    }
    fn run(plan: ExecutionPlan) -> ExecuteSummary {
        execute(
            plan,
            ExecuteOptions::default(),
            no_sudo,
            &mut NoProgress,
            &mut AutoConfirm,
        )
        .unwrap()
    }

    #[test]
    fn one_unavailable_or_unknown_observation_blocks_the_entire_scope() {
        for unknown in [true, false] {
            let trace = trace();
            let first = Stateful::new("first", &trace);
            let mut unavailable = Stateful::new("unavailable", &trace);
            unavailable.unknown = unknown;
            unavailable.unavailable = !unknown;
            let result = run(plan(vec![first, unavailable]));
            assert!(!result.is_success());
            assert_eq!(result.blocked, 2);
            assert!(trace.lock().unwrap().is_empty());
            assert!(
                result
                    .outcomes
                    .iter()
                    .all(|row| row.effects == EffectStatus::None)
            );
        }
    }

    #[test]
    fn diff_errors_retain_the_resource_identity() {
        let trace = trace();
        let mut unavailable = Stateful::new("unavailable", &trace);
        unavailable.unavailable = true;
        let resources = plan(vec![unavailable]);
        let error = crate::compute_diffs(&resources.unprivileged).unwrap_err();
        assert!(format!("{error:#}").contains("fixture unavailable"));
    }

    #[test]
    fn preview_never_confirms_or_applies_and_does_not_claim_verification() {
        struct UnexpectedConfirm;
        impl ConfirmCallback for UnexpectedConfirm {
            fn confirm(&mut self, _: &str) -> Result<bool> {
                panic!("preview prompted")
            }
        }
        let trace = trace();
        let result = execute(
            plan(vec![Stateful::new("first", &trace)]),
            ExecuteOptions {
                dry_run: true,
                ..Default::default()
            },
            no_sudo,
            &mut NoProgress,
            &mut UnexpectedConfirm,
        )
        .unwrap();
        assert!(result.preview && result.is_success());
        assert_eq!(result.created, 0);
        assert_eq!(result.outcomes[0].execution, ExecutionStatus::NotStarted);
        assert_eq!(result.outcomes[0].verification, VerificationStatus::Pending);
        assert!(trace.lock().unwrap().is_empty());
    }

    #[test]
    fn declined_and_unavailable_confirmation_leave_effects_unattempted() {
        struct UnavailableConfirm;
        impl ConfirmCallback for UnavailableConfirm {
            fn confirm(&mut self, _: &str) -> Result<bool> {
                bail!("stdin unavailable")
            }
        }
        let trace = trace();
        let declined = execute(
            plan(vec![Stateful::new("declined", &trace)]),
            ExecuteOptions::default(),
            no_sudo,
            &mut NoProgress,
            &mut AutoDecline,
        )
        .unwrap();
        let unavailable = execute(
            plan(vec![Stateful::new("unavailable", &trace)]),
            ExecuteOptions::default(),
            no_sudo,
            &mut NoProgress,
            &mut UnavailableConfirm,
        )
        .unwrap();
        assert_eq!(declined.cancelled, 1);
        assert_eq!(unavailable.blocked, 1);
        assert!(!declined.is_success() && !unavailable.is_success());
        assert!(trace.lock().unwrap().is_empty());
    }

    #[test]
    fn a_timeout_after_an_effect_does_not_become_no_change_or_rollback() {
        let trace = trace();
        let mut resource = Stateful::new("partial", &trace);
        resource.fail_after_effect = true;
        let actual = Arc::clone(&resource.present);
        let result = run(plan(vec![resource, Stateful::new("later", &trace)]));
        assert!(actual.load(Ordering::SeqCst));
        assert_eq!(result.failed, 1);
        assert_eq!(result.blocked, 1);
        assert_eq!(result.outcomes[0].execution, ExecutionStatus::Failed);
        assert_eq!(result.outcomes[0].effects, EffectStatus::Unknown);
        assert_eq!(result.outcomes[0].verification, VerificationStatus::Pending);
        assert_eq!(*trace.lock().unwrap(), ["partial"]);
    }

    #[test]
    fn changed_precondition_blocks_without_attempting_the_action() {
        let trace = trace();
        let mut resource = Stateful::new("changed", &trace);
        resource.change_precondition = true;
        let result = run(plan(vec![resource]));
        assert_eq!(result.blocked, 1);
        assert!(trace.lock().unwrap().is_empty());
        assert_eq!(result.outcomes[0].effects, EffectStatus::None);
    }

    #[test]
    fn serial_and_shared_mutations_preserve_plan_order_even_with_many_jobs() {
        let trace = trace();
        let mut first = Stateful::new("first", &trace);
        first.serial = true;
        let result = execute(
            plan(vec![first, Stateful::new("second", &trace)]),
            ExecuteOptions {
                jobs: 128,
                ..Default::default()
            },
            no_sudo,
            &mut NoProgress,
            &mut AutoConfirm,
        )
        .unwrap();
        assert!(result.is_success());
        assert_eq!(*trace.lock().unwrap(), ["first", "second"]);
    }

    #[test]
    fn whole_scope_verification_catches_a_later_action_invalidating_an_earlier_one() {
        let trace = trace();
        let first = Stateful::new("first", &trace);
        let mut second = Stateful::new("second", &trace);
        second.invalidates = Some(Arc::clone(&first.present));
        let mut plan = plan(vec![first, second]);
        plan.add_post_action_for("restart".into(), vec![parent("first")]);
        let result = execute_with_followups(
            plan,
            ExecuteOptions::default(),
            no_sudo,
            &mut NoProgress,
            &mut AutoConfirm,
            &mut |_| panic!("invalidated parent triggered follow-up"),
        )
        .unwrap();
        assert!(!result.is_success());
        assert_eq!(result.failed, 1);
        assert_eq!(result.outcomes[0].execution, ExecutionStatus::Completed);
        assert_eq!(result.outcomes[0].verification, VerificationStatus::Failed);
        assert_eq!(result.outcomes[2].execution, ExecutionStatus::Blocked);
    }

    #[test]
    fn only_selected_verified_changed_parents_trigger_followups() {
        let trace = trace();
        let mut original = plan(vec![
            Stateful::new("first", &trace),
            Stateful::new("second", &trace),
        ]);
        original.add_post_action_for("restart-first".into(), vec![parent("first")]);
        original.add_post_action_for("restart-second".into(), vec![parent("second")]);
        let selected = original.filter(|resource| resource.id() == "second");
        let result = execute_with_followups(
            selected,
            ExecuteOptions::default(),
            no_sudo,
            &mut NoProgress,
            &mut AutoConfirm,
            &mut |action| {
                trace.lock().unwrap().push(action.into());
                Ok(())
            },
        )
        .unwrap();
        assert!(result.is_success());
        assert_eq!(*trace.lock().unwrap(), ["second", "restart-second"]);
        assert_eq!(result.outcomes.len(), 2);
    }

    #[test]
    fn failed_parent_and_missing_followup_adapter_never_run_a_restart() {
        let trace = trace();
        let mut resource = Stateful::new("failed", &trace);
        resource.fail_after_effect = true;
        let mut selected = plan(vec![resource]);
        selected.add_post_action_for("restart".into(), vec![parent("failed")]);
        let result = execute_with_followups(
            selected,
            ExecuteOptions::default(),
            no_sudo,
            &mut NoProgress,
            &mut AutoConfirm,
            &mut |_| panic!("failed parent triggered follow-up"),
        )
        .unwrap();
        assert!(!result.is_success());
        assert_eq!(result.outcomes[1].execution, ExecutionStatus::Blocked);
        let mut selected = plan(vec![Stateful::new("not-applied", &trace)]);
        selected.add_post_action_for("restart".into(), vec![parent("not-applied")]);
        let result = run(selected);
        assert_eq!(result.blocked, 2);
        assert_eq!(*trace.lock().unwrap(), ["failed"]);
    }

    #[test]
    fn a_followup_that_invalidates_its_parent_blocks_the_next_followup() {
        let trace = trace();
        let resource = Stateful::new("parent", &trace);
        let actual = Arc::clone(&resource.present);
        let mut selected = plan(vec![resource]);
        selected.add_post_action_for("first".into(), vec![parent("parent")]);
        selected.add_post_action_for("second".into(), vec![parent("parent")]);
        let result = execute_with_followups(
            selected,
            ExecuteOptions::default(),
            no_sudo,
            &mut NoProgress,
            &mut AutoConfirm,
            &mut |action| {
                assert_eq!(
                    action, "first",
                    "stale verification allowed a later follow-up"
                );
                trace.lock().unwrap().push(action.into());
                actual.store(false, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
        assert!(!result.is_success());
        assert_eq!(*trace.lock().unwrap(), ["parent", "first"]);
        assert_eq!(result.outcomes[2].execution, ExecutionStatus::Blocked);
    }

    #[test]
    fn privilege_lifetime_excludes_unprivileged_and_followup_effects() {
        struct ScopedSudo(Trace);
        impl SudoProvider for ScopedSudo {
            fn run(&self, _: &str, _: &[&str]) -> Result<CommandOutput> {
                panic!("unexpected native privilege command")
            }
        }
        impl Drop for ScopedSudo {
            fn drop(&mut self) {
                self.0.lock().unwrap().push("release".into());
            }
        }
        let trace = trace();
        let mut selected = plan(vec![Stateful::new("unprivileged", &trace)]);
        let mut privileged = Stateful::new("privileged", &trace);
        privileged.privileged = true;
        selected.privileged.push(Box::new(privileged));
        selected.add_post_action_for("restart".into(), vec![parent("privileged")]);
        let result = execute_with_followups(
            selected,
            ExecuteOptions::default(),
            || {
                trace.lock().unwrap().push("acquire".into());
                Ok(ScopedSudo(Arc::clone(&trace)))
            },
            &mut NoProgress,
            &mut AutoConfirm,
            &mut |action| {
                assert_eq!(trace.lock().unwrap().last().unwrap(), "release");
                trace.lock().unwrap().push(action.into());
                Ok(())
            },
        )
        .unwrap();
        assert!(result.is_success());
        assert_eq!(
            *trace.lock().unwrap(),
            [
                "unprivileged",
                "acquire",
                "privileged",
                "release",
                "restart"
            ]
        );
    }

    #[test]
    fn failed_privilege_acquisition_preserves_prior_verified_effects() {
        let trace = trace();
        let unprivileged = Stateful::new("unprivileged", &trace);
        let actual = Arc::clone(&unprivileged.present);
        let mut selected = plan(vec![unprivileged]);
        let mut privileged = Stateful::new("privileged", &trace);
        privileged.privileged = true;
        selected.privileged.push(Box::new(privileged));
        let result = execute(
            selected,
            ExecuteOptions::default(),
            || -> Result<NoSudo> { bail!("fixture acquisition denied") },
            &mut NoProgress,
            &mut AutoConfirm,
        )
        .unwrap();
        assert!(!result.is_success());
        assert!(actual.load(Ordering::SeqCst));
        assert_eq!(result.outcomes[0].effects, EffectStatus::VerifiedChange);
        assert_eq!(result.outcomes[1].execution, ExecutionStatus::Blocked);
        assert_eq!(*trace.lock().unwrap(), ["unprivileged"]);
    }

    #[test]
    fn classifier_resource_privilege_disagreement_blocks_before_acquisition() {
        struct Elevated;
        impl SudoClassifier for Elevated {
            fn requires_sudo(&self, _: &str, _: &str) -> bool {
                true
            }
        }
        let trace = trace();
        let mut plan = ExecutionPlan::new();
        plan.add_resource(Box::new(Stateful::new("mismatch", &trace)), &Elevated);
        let result = run(plan);
        assert_eq!(result.blocked, 1);
        assert!(trace.lock().unwrap().is_empty());
    }
}
