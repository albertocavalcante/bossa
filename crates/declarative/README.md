# declarative

A framework for declarative resource management - define desired state, detect current state, converge.

## Core Concepts

- **Resource**: Something with state that can be managed (files, packages, settings)
- **ResourceState**: The current or desired state (`Present`, `Absent`, `Modified`)
- **ExecutionPlan**: Groups resources by privilege level for batched execution
- **Executor**: Applies resources with parallelism and privilege batching

## Design Principles

1. **Dependency Injection**: All external dependencies (sudo, progress, confirmation) are abstracted via traits
2. **No UI Dependencies**: The crate is UI-agnostic - implement `ProgressCallback` for your UI
3. **Privilege Separation**: Resources are classified as privileged/unprivileged, executed in batches
4. **Lazy Privilege Acquisition**: Sudo is only acquired when needed

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
