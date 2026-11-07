# declarative

A framework for declarative resource management - define desired state, detect current state, converge.

## Core Concepts

- **Resource**: Something with state that can be managed (files, packages, settings)
- **ResourceState**: The current or desired state (`Present`, `Absent`, `Modified`)
- **ExecutionPlan**: Groups resources by privilege level for batched execution
- **Executor**: Preflights a selected scope, applies serially and verifies postconditions

## Design Principles

1. **Dependency Injection**: All external dependencies (sudo, progress, confirmation) are abstracted via traits
2. **No UI Dependencies**: The crate is UI-agnostic - implement `ProgressCallback` for your UI
3. **Privilege Separation**: Classification must agree with resource requirements before execution
4. **Lazy Privilege Acquisition**: Sudo is acquired only for the privileged batch after confirmation, and released before follow-ups
5. **Conservative Scheduling**: All effects are serial until mutable-state independence is demonstrated

## Usage

```rust
use declarative::{
    Resource, ResourceState, ApplyResult, ApplyContext,
    ExecutionPlan, ExecuteOptions, execute_simple,
};

// 1. Define a resource
#[derive(Debug)]
struct FileResource { path: String, content: String }

impl Resource for FileResource {
    fn id(&self) -> String { self.path.clone() }
    fn description(&self) -> String { format!("File: {}", self.path) }
    fn resource_type(&self) -> &'static str { "file" }

    fn current_state(&self) -> anyhow::Result<ResourceState> {
        if std::path::Path::new(&self.path).exists() {
            Ok(ResourceState::Present { details: None })
        } else {
            Ok(ResourceState::Absent)
        }
    }

    fn desired_state(&self) -> ResourceState {
        ResourceState::Present { details: None }
    }

    fn apply(&self, ctx: &mut ApplyContext) -> anyhow::Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped { reason: "Dry run".into() });
        }
        std::fs::write(&self.path, &self.content)?;
        Ok(ApplyResult::Created)
    }
}

// 2. Build an execution plan
let mut plan = ExecutionPlan::new();
plan.unprivileged.push(Box::new(FileResource {
    path: "/tmp/test.txt".into(),
    content: "hello".into(),
}));

// 3. Execute
let summary = execute_simple(plan, ExecuteOptions::default(), || {
    anyhow::bail!("No sudo needed")
})?;
```

## Execution contract

`current_state` must report unavailable observations as errors. Confirmed absence is distinct
from read failure or `ResourceState::Unknown`. All selected observations are checked before
any mutation; a preflight failure blocks the selected scope. Preconditions are rechecked
before each action, and satisfied postconditions are reobserved after effects.

`ExecuteSummary.outcomes` separates execution, effects and verification for every selected
action. Preview rows remain unexecuted; cancelled, blocked, skipped or unverified apply is
unsuccessful. A failed action may leave partial effects, and the report never claims rollback.
It is an ephemeral report, not a durable finish receipt. Unchanged observations are counted,
and verified-change totals include accepted follow-up actions.

Bind follow-ups with `add_post_action_for(action, parents)` using `ResourceKey`. Filtering a
plan removes follow-ups whose parents are all unselected. Execute them through
`execute_with_followups` with an adapter that verifies its documented contract. The legacy
unbound `add_post_action` and an unavailable adapter block execution before changes. Required
scope state is checked before and after each follow-up.

`jobs` remains a positive requested upper bound for API compatibility. This implementation
executes serially even when a resource permits parallelism; independent scheduling requires
further evidence. Applications supply confirmation and can render `ProgressCallback::on_plan`
before consent. Bare counter accumulation with `add_result` supplies no verification evidence
and cannot by itself establish a successful report.

## Provider Traits

Implement these traits to integrate with your application:

### `SudoProvider`

```rust
pub trait SudoProvider: Send + Sync {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput>;
}
```

### `SudoClassifier`

```rust
pub trait SudoClassifier: Send + Sync {
    fn requires_sudo(&self, resource_type: &str, resource_id: &str) -> bool;
}
```

### `ProgressCallback`

```rust
pub trait ProgressCallback: Send {
    fn on_batch_start(&mut self, count: usize, privileged: bool);
    fn on_resource_start(&mut self, id: &str, description: &str);
    fn on_resource_complete(&mut self, id: &str, result: &ApplyResult);
    fn on_batch_complete(&mut self);
}
```

### `ConfirmCallback`

```rust
pub trait ConfirmCallback: Send {
    fn confirm(&mut self, prompt: &str) -> Result<bool>;
}
```

## Built-in Implementations

- `NoSudo`: Classifier that never requires sudo
- `NoProgress`: No-op progress callback
- `AutoConfirm`: Always confirms
- `AutoDecline`: Always declines

## License

MIT
