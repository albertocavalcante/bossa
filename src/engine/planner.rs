//! Execution planner - bossa-specific extensions

use crate::resource::Resource;
use crate::sudo::SudoConfig;
use declarative::ExecutionPlan;

/// Extension trait for ExecutionPlan with bossa-specific functionality
pub trait ExecutionPlanExt {
    /// Add a resource using bossa's SudoConfig for classification
    fn add_resource_with_config(&mut self, resource: Box<dyn Resource>, sudo_config: &SudoConfig);

    /// Add a service to restart after apply
    fn add_restart_service(&mut self, service: String);

    /// Get services to restart
    fn restart_services(&self) -> &[String];
}

impl ExecutionPlanExt for ExecutionPlan {
    fn add_resource_with_config(&mut self, resource: Box<dyn Resource>, sudo_config: &SudoConfig) {
        self.add_resource(resource, sudo_config);
    }

    fn add_restart_service(&mut self, service: String) {
        self.add_post_action(service);
    }

    fn restart_services(&self) -> &[String] {
        &self.post_actions
    }
}

/// Parse a target string like "collections.refs" into (resource_type, name)
pub fn parse_target(target: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = target.split('.').collect();
    match parts.len() {
        1 => (Some(parts[0].to_string()), None),
        2 => (Some(parts[0].to_string()), Some(parts[1].to_string())),
        _ => (None, Some(target.to_string())),
    }
}

/// Check if a resource matches the filter
pub fn matches_filter(
    resource: &dyn Resource,
    resource_type: Option<&str>,
    name: Option<&str>,
) -> bool {
    if let Some(rt) = resource_type {
        // Map resource_type filter to actual types
        let matches_type = match rt {
            "packages" | "brew" => resource.resource_type().starts_with("brew"),
            "defaults" => resource.resource_type() == "macos_default",
            "symlinks" => resource.resource_type() == "symlink",
            "services" => resource.resource_type() == "service",
            _ => resource.resource_type() == rt,
        };
        if !matches_type {
            return false;
        }
    }

    if let Some(n) = name
        && !resource.id().contains(n)
    {
        return false;
    }

    true
}
