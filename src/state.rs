#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

// ============================================================================
// State Structures
// ============================================================================

/// Main state structure tracking all bossa operations
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BossaState {
    /// State for each collection (refs collections)
    #[serde(default)]
    pub collections: HashMap<String, CollectionState>,

    /// State for workspaces
    #[serde(default)]
    pub workspaces: WorkspacesState,

    /// State for storage volumes (e.g., T9)
    #[serde(default)]
    pub storage: HashMap<String, StorageState>,

    /// Last time the state was updated
    pub last_updated: DateTime<Utc>,
}

/// State for a single refs collection
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CollectionState {
    /// Last time this collection was synced
    pub last_sync: Option<DateTime<Utc>>,

    /// Repositories that were successfully cloned
    #[serde(default)]
    pub repos_cloned: Vec<String>,

    /// Repositories that failed to clone (repo name, error message)
    #[serde(default)]
    pub repos_failed: Vec<(String, String)>,

    /// Whether the collection path has been verified to exist
    #[serde(default)]
    pub path_verified: bool,
}

/// State for workspaces
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct WorkspacesState {
    /// Last time workspaces were synced
    pub last_sync: Option<DateTime<Utc>>,

    /// Repositories that were successfully set up
    #[serde(default)]
    pub repos_setup: Vec<String>,

    /// Repositories that failed to set up (repo name, error message)
    #[serde(default)]
    pub repos_failed: Vec<(String, String)>,
}

/// State for a storage volume
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct StorageState {
    /// Last time the volume was seen mounted
    pub last_seen: Option<DateTime<Utc>>,

    /// Symlinks that were created (full paths)
    #[serde(default)]
    pub symlinks_created: Vec<String>,

    /// Whether the volume is currently mounted
    #[serde(default)]
    pub is_mounted: bool,
}

// ============================================================================
// BossaState Implementation
// ============================================================================

impl BossaState {
    /// Get the state directory path (~/.local/state/bossa)
    pub fn state_dir() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".local").join("state").join("bossa"))
    }

    /// Get the state file path
    fn state_file() -> Result<PathBuf> {
        Ok(Self::state_dir()?.join("state.toml"))
    }

    /// Load state from disk, or return default if file doesn't exist
    pub fn load() -> Result<Self> {
        let path = Self::state_file()?;

        if !path.exists() {
            log::debug!("State file does not exist, using default state");
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read state file: {}", path.display()))?;

        let state: BossaState = toml::from_str(&content)
            .with_context(|| format!("Failed to parse state file: {}", path.display()))?;

        log::debug!("Loaded state from {}", path.display());
        Ok(state)
    }

    /// Save state to disk
    pub fn save(&self) -> Result<()> {
        let dir = Self::state_dir()?;
        fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create state directory: {}", dir.display()))?;

        let path = Self::state_file()?;
        let content = toml::to_string_pretty(&self).context("Failed to serialize state to TOML")?;

        fs::write(&path, &content)
            .with_context(|| format!("Failed to write state file: {}", path.display()))?;

        log::debug!("Saved state to {}", path.display());
        Ok(())
    }

    /// Update the last_updated timestamp and save
    pub fn touch(&mut self) -> Result<()> {
        self.last_updated = Utc::now();
        self.save()
    }

    // ========================================================================
    // Collection State Helpers
    // ========================================================================

    /// Get or create collection state
    pub fn get_collection_mut(&mut self, name: &str) -> &mut CollectionState {
        self.collections.entry(name.to_string()).or_default()
    }

    /// Mark a repository as successfully cloned in a collection
    pub fn mark_repo_cloned(&mut self, collection: &str, repo_name: &str) {
        let state = self.get_collection_mut(collection);
        if !state.repos_cloned.contains(&repo_name.to_string()) {
            state.repos_cloned.push(repo_name.to_string());
        }
        // Remove from failed list if present
        state.repos_failed.retain(|(name, _)| name != repo_name);
    }

    /// Mark a repository as failed in a collection
    pub fn mark_repo_failed(&mut self, collection: &str, repo_name: &str, error: &str) {
        let state = self.get_collection_mut(collection);
        // Remove from cloned list if present
        state.repos_cloned.retain(|name| name != repo_name);
        // Update or add to failed list
        state.repos_failed.retain(|(name, _)| name != repo_name);
        state
            .repos_failed
            .push((repo_name.to_string(), error.to_string()));
    }

    /// Mark collection path as verified
    pub fn mark_collection_path_verified(&mut self, collection: &str) {
        let state = self.get_collection_mut(collection);
        state.path_verified = true;
    }

    /// Mark collection as synced
    pub fn mark_collection_synced(&mut self, collection: &str) {
        let state = self.get_collection_mut(collection);
        state.last_sync = Some(Utc::now());
    }

    // ========================================================================
    // Workspace State Helpers
    // ========================================================================

    /// Mark a workspace repository as successfully set up
    pub fn mark_workspace_setup(&mut self, repo_name: &str) {
        if !self.workspaces.repos_setup.contains(&repo_name.to_string()) {
            self.workspaces.repos_setup.push(repo_name.to_string());
        }
        // Remove from failed list if present
        self.workspaces
            .repos_failed
            .retain(|(name, _)| name != repo_name);
    }

    /// Mark a workspace repository as failed
    pub fn mark_workspace_failed(&mut self, repo_name: &str, error: &str) {
        // Remove from setup list if present
        self.workspaces.repos_setup.retain(|name| name != repo_name);
        // Update or add to failed list
        self.workspaces
            .repos_failed
            .retain(|(name, _)| name != repo_name);
        self.workspaces
            .repos_failed
            .push((repo_name.to_string(), error.to_string()));
    }

    /// Mark workspaces as synced
    pub fn mark_workspaces_synced(&mut self) {
        self.workspaces.last_sync = Some(Utc::now());
    }

    // ========================================================================
    // Storage State Helpers
    // ========================================================================

    /// Get or create storage state
    pub fn get_storage_mut(&mut self, name: &str) -> &mut StorageState {
        self.storage.entry(name.to_string()).or_default()
    }

    /// Mark a storage volume as mounted
    pub fn mark_storage_mounted(&mut self, name: &str) {
        let state = self.get_storage_mut(name);
        state.is_mounted = true;
        state.last_seen = Some(Utc::now());
    }

    /// Mark a storage volume as unmounted
    pub fn mark_storage_unmounted(&mut self, name: &str) {
        let state = self.get_storage_mut(name);
        state.is_mounted = false;
    }

    /// Add a symlink to storage state
    pub fn add_storage_symlink(&mut self, storage_name: &str, symlink_path: &str) {
        let state = self.get_storage_mut(storage_name);
        if !state.symlinks_created.contains(&symlink_path.to_string()) {
            state.symlinks_created.push(symlink_path.to_string());
        }
    }

    /// Remove a symlink from storage state
    pub fn remove_storage_symlink(&mut self, storage_name: &str, symlink_path: &str) {
        let state = self.get_storage_mut(storage_name);
        state.symlinks_created.retain(|path| path != symlink_path);
    }
}

impl Default for BossaState {
    fn default() -> Self {
        Self {
            collections: HashMap::new(),
            workspaces: WorkspacesState::default(),
            storage: HashMap::new(),
            last_updated: Utc::now(),
        }
    }
}

// ============================================================================
// CollectionState Implementation
// ============================================================================

// ============================================================================
// StorageState Implementation
// ============================================================================

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_state() {
        let state = BossaState::default();
        assert!(state.collections.is_empty());
        assert!(state.workspaces.repos_setup.is_empty());
        assert!(state.storage.is_empty());
    }

    #[test]
    fn test_collection_state() {
        let mut state = BossaState::default();

        // Mark repo as cloned
        state.mark_repo_cloned("refs", "rust");
        assert!(
            state
                .collections
                .get("refs")
                .unwrap()
                .repos_cloned
                .contains(&"rust".to_string())
        );

        // Mark repo as failed
        state.mark_repo_failed("refs", "rust", "clone failed");
        assert!(
            !state
                .collections
                .get("refs")
                .unwrap()
                .repos_cloned
                .contains(&"rust".to_string())
        );
        assert_eq!(state.collections.get("refs").unwrap().repos_failed.len(), 1);

        // Mark as cloned again (should remove from failed)
        state.mark_repo_cloned("refs", "rust");
        assert!(
            state
                .collections
                .get("refs")
                .unwrap()
                .repos_cloned
                .contains(&"rust".to_string())
        );
        assert!(
            state
                .collections
                .get("refs")
                .unwrap()
                .repos_failed
                .is_empty()
        );
    }

    #[test]
    fn test_workspace_state() {
        let mut state = BossaState::default();

        // Mark workspace as setup
        state.mark_workspace_setup("dotfiles");
        assert!(
            state
                .workspaces
                .repos_setup
                .contains(&"dotfiles".to_string())
        );

        // Mark as failed
        state.mark_workspace_failed("dotfiles", "setup failed");
        assert!(
            !state
                .workspaces
                .repos_setup
                .contains(&"dotfiles".to_string())
        );
        assert_eq!(state.workspaces.repos_failed.len(), 1);
    }

    #[test]
    fn test_storage_state() {
        let mut state = BossaState::default();

        // Mark storage as mounted
        state.mark_storage_mounted("t9");
        assert!(state.storage.get("t9").unwrap().is_mounted);

        // Add symlink
        state.add_storage_symlink("t9", "/home/user/dev/refs");
        assert!(
            state
                .storage
                .get("t9")
                .unwrap()
                .symlinks_created
                .contains(&"/home/user/dev/refs".to_string())
        );

        // Mark as unmounted
        state.mark_storage_unmounted("t9");
        assert!(!state.storage.get("t9").unwrap().is_mounted);
    }

    #[test]
    fn test_serialization() {
        let mut state = BossaState::default();
        state.mark_repo_cloned("refs", "rust");
        state.mark_collection_path_verified("refs");
        state.mark_workspace_setup("dotfiles");
        state.mark_storage_mounted("t9");
        state.add_storage_symlink("t9", "~/dev/refs");

        // Serialize to TOML
        let toml_str = toml::to_string_pretty(&state).unwrap();
        assert!(toml_str.contains("rust"));
        assert!(toml_str.contains("dotfiles"));
        assert!(toml_str.contains("t9"));

        // Deserialize back
        let deserialized: BossaState = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            deserialized
                .collections
                .get("refs")
                .unwrap()
                .repos_cloned
                .len(),
            1
        );
        assert_eq!(deserialized.workspaces.repos_setup.len(), 1);
        assert_eq!(
            deserialized
                .storage
                .get("t9")
                .unwrap()
                .symlinks_created
                .len(),
            1
        );
    }
}
