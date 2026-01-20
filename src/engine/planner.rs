//! Execution planner - builds resource graph from config

use crate::resource::Resource;
use crate::sudo::SudoConfig;

/// An execution plan with resources grouped by privilege level
pub struct ExecutionPlan {
    /// Resources that don't need sudo
    pub unprivileged: Vec<Box<dyn Resource>>,
    /// Resources that need sudo (from config allowlist)
    pub privileged: Vec<Box<dyn Resource>>,
    /// Services to restart after apply
    pub restart_services: Vec<String>,
}

impl ExecutionPlan {
    /// Create a new empty plan
    pub fn new() -> Self {
        Self {
            unprivileged: Vec::new(),
            privileged: Vec::new(),
            restart_services: Vec::new(),
        }
    }

    /// Add a resource to the plan, classifying by sudo requirement
    pub fn add_resource(&mut self, resource: Box<dyn Resource>, sudo_config: &SudoConfig) {
        let requires_sudo = match resource.resource_type() {
            "brew_cask" => sudo_config.cask_requires_sudo(&resource.id()),
            "macos_default" => sudo_config.default_requires_sudo(&resource.id()),
            _ => false,
        };

        if requires_sudo {
            self.privileged.push(resource);
        } else {
            self.unprivileged.push(resource);
        }
    }

    /// Add a service to restart after apply
    pub fn add_restart_service(&mut self, service: String) {
        if !self.restart_services.contains(&service) {
            self.restart_services.push(service);
        }
    }

    /// Filter plan to only include resources matching a target
    pub fn filter_by_target(self, target: Option<&str>) -> Self {
        match target {
            None => self,
            Some(t) => {
                let (resource_type, name) = parse_target(t);
                Self {
                    unprivileged: self
                        .unprivileged
                        .into_iter()
                        .filter(|r| {
                            matches_filter(r.as_ref(), resource_type.as_deref(), name.as_deref())
                        })
                        .collect(),
                    privileged: self
                        .privileged
                        .into_iter()
                        .filter(|r| {
                            matches_filter(r.as_ref(), resource_type.as_deref(), name.as_deref())
                        })
                        .collect(),
                    restart_services: self.restart_services,
                }
            }
        }
    }

    /// Total number of resources in the plan
    pub fn total_resources(&self) -> usize {
        self.unprivileged.len() + self.privileged.len()
    }

    /// Check if plan is empty
    pub fn is_empty(&self) -> bool {
        self.unprivileged.is_empty() && self.privileged.is_empty()
    }
}

impl Default for ExecutionPlan {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a target string like "collections.refs" into (resource_type, name)
fn parse_target(target: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = target.split('.').collect();
    match parts.len() {
        1 => (Some(parts[0].to_string()), None),
        2 => (Some(parts[0].to_string()), Some(parts[1].to_string())),
        _ => (None, Some(target.to_string())),
    }
}

/// Check if a resource matches the filter
fn matches_filter(
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

    if let Some(n) = name {
        if !resource.id().contains(n) {
            return false;
        }
    }

    true
}
