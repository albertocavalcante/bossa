//! Dock configuration resources

use anyhow::{Context, Result, bail};
use std::process::Command;

use super::{ApplyContext, ApplyResult, Resource, ResourceState};

/// A Dock app resource
#[derive(Debug, Clone)]
pub struct DockApp {
    pub app_path: String,
    pub position: Option<usize>,
}

impl DockApp {
    pub fn new(app_path: &str) -> Self {
        Self {
            app_path: app_path.to_string(),
            position: None,
        }
    }

    pub fn at_position(mut self, position: usize) -> Self {
        self.position = Some(position);
        self
    }

    /// Check if app is in dock
    fn is_in_dock(&self) -> Result<bool> {
        let output = Command::new("defaults")
            .args(["read", "com.apple.dock", "persistent-apps"])
            .output()
            .context("Failed to read dock apps")?;

        if !output.status.success() {
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.contains(&self.app_path))
    }

    /// Add app to dock using dockutil
    fn add_to_dock(&self, _ctx: &ApplyContext) -> Result<()> {
        let mut args = vec!["--add", &self.app_path, "--no-restart"];

        let position_str;
        if let Some(pos) = self.position {
            position_str = pos.to_string();
            args.extend_from_slice(&["--position", &position_str]);
        }

        let output = Command::new("dockutil")
            .args(&args)
            .output()
            .context("Failed to run dockutil")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("dockutil add failed: {}", stderr.trim());
        }

        Ok(())
    }
}

impl Resource for DockApp {
    fn id(&self) -> String {
        format!("dock:app:{}", self.app_path)
    }

    fn description(&self) -> String {
        if let Some(pos) = self.position {
            format!("Add {} to Dock at position {}", self.app_path, pos)
        } else {
            format!("Add {} to Dock", self.app_path)
        }
    }

    fn resource_type(&self) -> &'static str {
        "dock_app"
    }

    fn current_state(&self) -> Result<ResourceState> {
        if self.is_in_dock()? {
            Ok(ResourceState::Present { details: None })
        } else {
            Ok(ResourceState::Absent)
        }
    }

    fn desired_state(&self) -> ResourceState {
        ResourceState::Present { details: None }
    }

    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped {
                reason: "Dry run".to_string(),
            });
        }

        if self.is_in_dock()? {
            return Ok(ApplyResult::NoChange);
        }

        self.add_to_dock(ctx)?;
        Ok(ApplyResult::Created)
    }

    fn can_parallelize(&self) -> bool {
        false // Dock modifications should be sequential
    }
}

/// A Dock folder resource
#[derive(Debug, Clone)]
pub struct DockFolder {
    pub path: String,
    pub view: String,
    pub display: String,
    pub sort: String,
}

impl DockFolder {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            view: "grid".to_string(),
            display: "stack".to_string(),
            sort: "dateadded".to_string(),
        }
    }

    /// Check if folder is in dock
    fn is_in_dock(&self) -> Result<bool> {
        let output = Command::new("defaults")
            .args(["read", "com.apple.dock", "persistent-others"])
            .output()
            .context("Failed to read dock folders")?;

        if !output.status.success() {
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let expanded_path = crate::paths::expand(&self.path);
        Ok(stdout.contains(expanded_path.to_string_lossy().as_ref()))
    }

    /// Add folder to dock using dockutil
    fn add_to_dock(&self, _ctx: &ApplyContext) -> Result<()> {
        let expanded_path = crate::paths::expand(&self.path)
            .to_string_lossy()
            .to_string();

        let mut args = vec!["--add", &expanded_path, "--no-restart"];
        args.extend_from_slice(&["--view", &self.view]);
        args.extend_from_slice(&["--display", &self.display]);
        args.extend_from_slice(&["--sort", &self.sort]);

        let output = Command::new("dockutil")
            .args(&args)
            .output()
            .context("Failed to run dockutil")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("dockutil add folder failed: {}", stderr.trim());
        }

        Ok(())
    }
}

impl Resource for DockFolder {
    fn id(&self) -> String {
        format!("dock:folder:{}", self.path)
    }

    fn description(&self) -> String {
        format!("Add folder {} to Dock", self.path)
    }

    fn resource_type(&self) -> &'static str {
        "dock_folder"
    }

    fn current_state(&self) -> Result<ResourceState> {
        if self.is_in_dock()? {
            Ok(ResourceState::Present { details: None })
        } else {
            Ok(ResourceState::Absent)
        }
    }

    fn desired_state(&self) -> ResourceState {
        ResourceState::Present { details: None }
    }

    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped {
                reason: "Dry run".to_string(),
            });
        }

        if self.is_in_dock()? {
            return Ok(ApplyResult::NoChange);
        }

        self.add_to_dock(ctx)?;
        Ok(ApplyResult::Created)
    }

    fn can_parallelize(&self) -> bool {
        false // Dock modifications should be sequential
    }
}
