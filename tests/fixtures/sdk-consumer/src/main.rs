//! Path-based preparation, not the independent immutable-source proof.
use anyhow::Result;
use bossa_sdk::{AppEnvironment, ConfigFormat, decode_bossa_config};
use declarative::{ApplyContext, ApplyResult, ExecutionPlan, Resource, ResourceState};

#[derive(Debug)]
struct MemoryResource {
    id: String,
}
impl Resource for MemoryResource {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn description(&self) -> String {
        "Caller-owned in-memory state".into()
    }
    fn resource_type(&self) -> &'static str {
        "memory"
    }
    fn current_state(&self) -> Result<ResourceState> {
        Ok(ResourceState::Absent)
    }
    fn desired_state(&self) -> ResourceState {
        ResourceState::Present { details: None }
    }
    fn apply(&self, _: &mut ApplyContext) -> Result<ApplyResult> {
        Ok(ApplyResult::NoChange)
    }
}

fn main() -> Result<()> {
    let desired =
        decode_bossa_config(ConfigFormat::Toml, b"[collections.refs]\npath='~/refs'\n")?.desired;
    let environment = AppEnvironment::new(Some("/synthetic-home".into()), []);
    let path = environment.expand(&desired.collections["refs"].path);
    assert_eq!(path, std::path::Path::new("/synthetic-home/refs"));
    let mut plan = ExecutionPlan::new();
    plan.add_resource_explicit(Box::new(MemoryResource { id: "refs".into() }));
    assert_eq!(plan.unprivileged.len(), 1);
    let resource = &plan.unprivileged[0];
    assert_eq!(resource.id(), "refs");
    assert_eq!(resource.current_state()?, ResourceState::Absent);
    assert_eq!(
        resource.desired_state(),
        ResourceState::Present { details: None }
    );
    Ok(())
}
