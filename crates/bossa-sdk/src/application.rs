//! Application policy over the reusable resource engine. Terminal UI stays in
//! the binary; callers supply confirmation, privilege and process adapters.

use anyhow::{Context, Result, bail};
use declarative::{ConfirmCallback, ExecuteSummary, ExecutionPlan, ProgressCallback, SudoProvider};
use std::path::Path;

use crate::{CommandRequest, IoMode, ProcessRunner};

#[derive(Debug, Clone)]
pub struct ResourceApplyOptions {
    pub dry_run: bool,
    pub jobs: usize,
    pub yes: bool,
    pub verbose: bool,
}

impl Default for ResourceApplyOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            jobs: 4,
            yes: false,
            verbose: false,
        }
    }
}

struct Consent<'a, C> {
    yes: bool,
    callback: &'a mut C,
}

impl<C: ConfirmCallback> ConfirmCallback for Consent<'_, C> {
    fn confirm(&mut self, prompt: &str) -> Result<bool> {
        confirm_apply(self.yes, prompt, self.callback)
    }
}

/// Request an explicit apply decision.
///
/// Yes bypasses the callback; a declined
/// decision stays false and unavailable input stays an error. Preview callers
/// must skip this boundary before requesting any mutation.
pub fn confirm_apply<C: ConfirmCallback + ?Sized>(
    yes: bool,
    prompt: &str,
    callback: &mut C,
) -> Result<bool> {
    if yes {
        Ok(true)
    } else {
        callback.confirm(prompt)
    }
}

/// Apply only after full observation and explicit consent.
///
/// A restart follow-up means that the OS accepted the signal request,
/// not verified service health.
/// The returned report is ephemeral and must not authorize automated cleanup.
pub fn apply_resources<S, P, C>(
    plan: ExecutionPlan,
    options: ResourceApplyOptions,
    runner: &dyn ProcessRunner,
    working_dir: &Path,
    sudo_provider: impl FnOnce() -> Result<S>,
    progress: &mut P,
    confirm: &mut C,
) -> Result<ExecuteSummary>
where
    S: SudoProvider,
    P: ProgressCallback,
    C: ConfirmCallback,
{
    for action in &plan.post_actions {
        let service = &action.action;
        if service.is_empty() || service.starts_with('-') || service.chars().any(char::is_control) {
            bail!("Invalid service name; selected plan was not applied");
        }
    }
    if !plan.post_actions.is_empty() && !std::fs::metadata(working_dir)?.is_dir() {
        bail!("Follow-up working directory is unavailable; selected plan was not applied");
    }
    let mut consent = Consent {
        yes: options.yes,
        callback: confirm,
    };
    declarative::executor::execute_with_followups(
        plan,
        declarative::ExecuteOptions {
            dry_run: options.dry_run,
            jobs: options.jobs,
            verbose: options.verbose,
        },
        sudo_provider,
        progress,
        &mut consent,
        &mut |service| {
            let output = runner
                .run(
                    &CommandRequest {
                        program: "killall".into(),
                        args: vec![service.into()],
                        working_dir: working_dir.to_path_buf(),
                    },
                    IoMode::Capture,
                )
                .context("Could not request service restart")?;
            if !output.status.success() {
                bail!("Service restart request was not accepted");
            }
            Ok(())
        },
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::CommandRequest;
    use declarative::{
        ApplyContext, ApplyResult, AutoDecline, CommandOutput, EffectStatus, ExecutionStatus,
        NoProgress, Resource, ResourceKey, ResourceState,
    };
    use std::cell::RefCell;

    #[test]
    fn consent_preserves_decline_and_unavailable_input_without_implicit_approval() {
        struct Unavailable;
        impl ConfirmCallback for Unavailable {
            fn confirm(&mut self, _: &str) -> Result<bool> {
                bail!("fixture input is unavailable");
            }
        }
        assert!(!confirm_apply(false, "fixture", &mut AutoDecline).unwrap());
        assert!(confirm_apply(false, "fixture", &mut Unavailable).is_err());
        assert!(confirm_apply(true, "fixture", &mut Unavailable).unwrap());
    }
    use std::io;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Debug)]
    struct FileEffect(Arc<AtomicBool>);
    impl Resource for FileEffect {
        fn id(&self) -> String {
            "fixture".into()
        }
        fn description(&self) -> String {
            "fixture write".into()
        }
        fn resource_type(&self) -> &'static str {
            "file"
        }
        fn current_state(&self) -> Result<ResourceState> {
            Ok(if self.0.load(Ordering::SeqCst) {
                self.desired_state()
            } else {
                ResourceState::Absent
            })
        }
        fn desired_state(&self) -> ResourceState {
            ResourceState::Present { details: None }
        }
        fn apply(&self, _: &mut ApplyContext) -> Result<ApplyResult> {
            self.0.store(true, Ordering::SeqCst);
            Ok(ApplyResult::Created)
        }
    }
    struct NoSudo;
    impl SudoProvider for NoSudo {
        fn run(&self, _: &str, _: &[&str]) -> Result<CommandOutput> {
            panic!("unexpected privilege effect")
        }
    }
    struct Recorder {
        calls: RefCell<Vec<CommandRequest>>,
        success: bool,
    }
    impl ProcessRunner for Recorder {
        fn run(&self, request: &CommandRequest, _: IoMode) -> io::Result<Output> {
            self.calls.borrow_mut().push(CommandRequest {
                program: request.program.clone(),
                args: request.args.clone(),
                working_dir: request.working_dir.clone(),
            });
            Ok(Output {
                status: ExitStatus::from_raw(if self.success { 0 } else { 256 }),
                stdout: vec![],
                stderr: vec![],
            })
        }
    }
    fn plan(state: &Arc<AtomicBool>, action: &str) -> ExecutionPlan {
        let mut plan = ExecutionPlan::new();
        plan.unprivileged
            .push(Box::new(FileEffect(Arc::clone(state))));
        plan.add_post_action_for(
            action.into(),
            vec![ResourceKey {
                resource_type: "file".into(),
                id: "fixture".into(),
            }],
        );
        plan
    }
    fn apply(
        plan: ExecutionPlan,
        options: ResourceApplyOptions,
        runner: &Recorder,
    ) -> Result<ExecuteSummary> {
        apply_resources(
            plan,
            options,
            runner,
            Path::new("."),
            || -> Result<NoSudo> { panic!("unexpected privilege acquisition") },
            &mut NoProgress,
            &mut AutoDecline,
        )
    }

    #[test]
    fn explicit_yes_is_the_only_policy_that_bypasses_declined_confirmation() {
        for yes in [false, true] {
            let state = Arc::new(false.into());
            let runner = Recorder {
                calls: RefCell::new(vec![]),
                success: true,
            };
            let report = apply(
                plan(&state, "FixtureService"),
                ResourceApplyOptions {
                    yes,
                    ..Default::default()
                },
                &runner,
            )
            .unwrap();
            assert_eq!(report.is_success(), yes);
            assert_eq!(state.load(Ordering::SeqCst), yes);
            assert_eq!(runner.calls.borrow().len(), usize::from(yes));
            if yes {
                let calls = runner.calls.borrow();
                assert_eq!(calls[0].program, "killall");
                assert_eq!(calls[0].args, ["FixtureService"]);
                assert_eq!(calls[0].working_dir, Path::new("."));
            } else {
                assert!(
                    report
                        .outcomes
                        .iter()
                        .all(|row| row.execution == ExecutionStatus::Cancelled)
                );
            }
        }
    }

    #[test]
    fn preview_with_yes_never_calls_process_or_resource_effects() {
        let state = Arc::new(false.into());
        let runner = Recorder {
            calls: RefCell::new(vec![]),
            success: true,
        };
        let report = apply(
            plan(&state, "FixtureService"),
            ResourceApplyOptions {
                yes: true,
                dry_run: true,
                ..Default::default()
            },
            &runner,
        )
        .unwrap();
        assert!(report.preview && report.is_success());
        assert!(!state.load(Ordering::SeqCst));
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn failed_followup_retains_verified_parent_effects_and_unsuccessful_report() {
        let state = Arc::new(false.into());
        let runner = Recorder {
            calls: RefCell::new(vec![]),
            success: false,
        };
        let report = apply(
            plan(&state, "FixtureService"),
            ResourceApplyOptions {
                yes: true,
                ..Default::default()
            },
            &runner,
        )
        .unwrap();
        assert!(!report.is_success());
        assert!(state.load(Ordering::SeqCst));
        assert_eq!(report.outcomes[0].effects, EffectStatus::VerifiedChange);
        assert_eq!(report.outcomes[1].execution, ExecutionStatus::Failed);
        assert_eq!(report.outcomes[1].effects, EffectStatus::Unknown);
    }

    #[test]
    fn invalid_restart_intent_fails_before_any_selected_effect() {
        let state = Arc::new(false.into());
        let runner = Recorder {
            calls: RefCell::new(vec![]),
            success: true,
        };
        assert!(
            apply(
                plan(&state, "--all"),
                ResourceApplyOptions {
                    yes: true,
                    ..Default::default()
                },
                &runner
            )
            .is_err()
        );
        assert!(!state.load(Ordering::SeqCst));
        assert!(runner.calls.borrow().is_empty());
    }
}
