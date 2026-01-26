//! Diff computation for resources

use crate::resource::Resource;
use crate::types::{ResourceState, SudoRequirement};
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A diff between current and desired state of a resource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDiff {
    /// Unique identifier of the resource
    pub resource_id: String,
    /// Type of the resource
    pub resource_type: String,
    /// Human-readable description
    pub description: String,
    /// Current state
    pub current: ResourceState,
    /// Desired state
    pub desired: ResourceState,
    /// Whether this resource requires sudo
    pub requires_sudo: bool,
}

impl ResourceDiff {
    /// Create a diff from a resource, returning None if no changes needed
    pub fn from_resource(resource: &dyn Resource) -> Result<Option<Self>> {
        let current = resource.current_state()?;
        let desired = resource.desired_state();

        if current == desired {
            return Ok(None);
        }

        Ok(Some(Self {
            resource_id: resource.id(),
            resource_type: resource.resource_type().to_string(),
            description: resource.description(),
            current,
            desired,
            requires_sudo: matches!(
                resource.sudo_requirement(),
                SudoRequirement::Required { .. }
            ),
        }))
    }

    /// Check if this diff represents an addition
    pub fn is_addition(&self) -> bool {
        matches!(
            (&self.current, &self.desired),
            (ResourceState::Absent, ResourceState::Present { .. })
        )
    }

    /// Check if this diff represents a removal
    pub fn is_removal(&self) -> bool {
        matches!(
            (&self.current, &self.desired),
            (ResourceState::Present { .. }, ResourceState::Absent)
        )
    }

    /// Check if this diff represents a modification
    pub fn is_modification(&self) -> bool {
        matches!(
            (&self.current, &self.desired),
            (ResourceState::Modified { .. }, _) | (_, ResourceState::Modified { .. })
        ) || matches!(
            (&self.current, &self.desired),
            (
                ResourceState::Present { details: Some(_) },
                ResourceState::Present { details: Some(_) }
            )
        )
    }
}

/// Compute diffs for a list of resources
///
/// Returns only resources that have differences between current and desired state.
pub fn compute_diffs(resources: &[Box<dyn Resource>]) -> Vec<ResourceDiff> {
    resources
        .iter()
        .filter_map(|r| ResourceDiff::from_resource(r.as_ref()).ok().flatten())
        .collect()
}

/// Diff summary statistics
#[derive(Debug, Clone, Default)]
pub struct DiffSummary {
    /// Number of resources to add
    pub additions: usize,
    /// Number of resources to remove
    pub removals: usize,
    /// Number of resources to modify
    pub modifications: usize,
    /// Number of resources requiring sudo
    pub sudo_required: usize,
}

impl DiffSummary {
    /// Create a summary from a list of diffs
    pub fn from_diffs(diffs: &[ResourceDiff]) -> Self {
        let mut summary = Self::default();
        for diff in diffs {
            if diff.is_addition() {
                summary.additions += 1;
            } else if diff.is_removal() {
                summary.removals += 1;
            } else {
                summary.modifications += 1;
            }
            if diff.requires_sudo {
                summary.sudo_required += 1;
            }
        }
        summary
    }

    /// Total number of changes
    pub fn total(&self) -> usize {
        self.additions + self.removals + self.modifications
    }

    /// Check if there are any changes
    pub fn has_changes(&self) -> bool {
        self.total() > 0
    }
}

/// Group diffs by resource type
pub fn group_by_type(
    diffs: &[ResourceDiff],
) -> std::collections::HashMap<String, Vec<&ResourceDiff>> {
    let mut groups: std::collections::HashMap<String, Vec<&ResourceDiff>> =
        std::collections::HashMap::new();
    for diff in diffs {
        groups
            .entry(diff.resource_type.clone())
            .or_default()
            .push(diff);
    }
    groups
}
