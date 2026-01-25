//! Execution planner - builds resource execution plans

use crate::context::SudoClassifier;
use crate::resource::{BoxedResource, Resource};

/// An execution plan with resources grouped by privilege level
pub struct ExecutionPlan {
    /// Resources that don't need elevated privileges
    pub unprivileged: Vec<BoxedResource>,
    /// Resources that need elevated privileges
    pub privileged: Vec<BoxedResource>,
    /// Post-apply actions (e.g., services to restart)
    pub post_actions: Vec<String>,
}

impl ExecutionPlan {
    /// Create a new empty plan
    pub fn new() -> Self {
        Self {
            unprivileged: Vec::new(),
            privileged: Vec::new(),
            post_actions: Vec::new(),
        }
    }

    /// Add a resource to the plan, classifying by sudo requirement
    ///
    /// Uses the provided classifier to determine if sudo is needed.
    pub fn add_resource<C: SudoClassifier>(&mut self, resource: BoxedResource, classifier: &C) {
        let requires_sudo = classifier.requires_sudo(resource.resource_type(), &resource.id());

        if requires_sudo {
            self.privileged.push(resource);
        } else {
            self.unprivileged.push(resource);
        }
    }

    /// Add a resource that explicitly declares its sudo requirement
    pub fn add_resource_explicit(&mut self, resource: BoxedResource) {
        use crate::types::SudoRequirement;

        if matches!(resource.sudo_requirement(), SudoRequirement::Required { .. }) {
            self.privileged.push(resource);
        } else {
            self.unprivileged.push(resource);
        }
    }

    /// Add a post-apply action
    pub fn add_post_action(&mut self, action: String) {
        if !self.post_actions.contains(&action) {
            self.post_actions.push(action);
        }
    }

    /// Filter plan to only include resources matching a predicate
    pub fn filter<F>(self, predicate: F) -> Self
    where
        F: Fn(&dyn Resource) -> bool,
    {
        Self {
            unprivileged: self
                .unprivileged
                .into_iter()
                .filter(|r| predicate(r.as_ref()))
                .collect(),
            privileged: self
                .privileged
                .into_iter()
                .filter(|r| predicate(r.as_ref()))
                .collect(),
            post_actions: self.post_actions,
        }
    }

    /// Filter plan to only include resources matching a target pattern
    ///
    /// Target format: "type" or "type.name"
    pub fn filter_by_target(self, target: Option<&str>) -> Self {
        match target {
            None => self,
            Some(t) => {
                let (resource_type, name) = parse_target(t);
                self.filter(|r| matches_filter(r, resource_type.as_deref(), name.as_deref()))
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

    /// Check if plan has any privileged resources
    pub fn has_privileged(&self) -> bool {
        !self.privileged.is_empty()
    }
}

impl Default for ExecutionPlan {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a target string like "type.name" into (type, name)
fn parse_target(target: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = target.split('.').collect();
    match parts.len() {
        1 => (Some(parts[0].to_string()), None),
        2 => (Some(parts[0].to_string()), Some(parts[1].to_string())),
        _ => (None, Some(target.to_string())),
    }
}

/// Check if a resource matches the filter criteria
fn matches_filter(
    resource: &dyn Resource,
    resource_type: Option<&str>,
    name: Option<&str>,
) -> bool {
    if let Some(rt) = resource_type {
        // Allow common aliases
        let matches_type = match rt {
            "packages" | "brew" => resource.resource_type().starts_with("brew"),
            "defaults" => resource.resource_type() == "macos_default",
            "symlinks" => resource.resource_type() == "symlink",
            "services" => resource.resource_type() == "service",
            _ => resource.resource_type() == rt || resource.resource_type().starts_with(rt),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_target() {
        assert_eq!(
            parse_target("brew"),
            (Some("brew".to_string()), None)
        );
        assert_eq!(
            parse_target("brew.ripgrep"),
            (Some("brew".to_string()), Some("ripgrep".to_string()))
        );
        assert_eq!(
            parse_target("a.b.c"),
            (None, Some("a.b.c".to_string()))
        );
    }
}
