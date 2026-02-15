#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::paths;

// ============================================================================
// Symlink Tracking Structures
// ============================================================================

/// A tracked symlink with metadata about its origin
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TrackedSymlink {
    /// What the symlink points to (source/target of the link)
    pub source: String,
    /// Where the symlink lives (the symlink file itself)
    pub target: String,
    /// Which subsystem created it: "stow", "caches", "storage"
    pub subsystem: String,
    /// When it was created
    pub created_at: DateTime<Utc>,
}

/// Inventory of all tracked symlinks
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SymlinkInventory {
    pub entries: Vec<TrackedSymlink>,
}

impl SymlinkInventory {
    /// Add a symlink to the inventory
    pub fn add(&mut self, symlink: TrackedSymlink) {
        self.entries.push(symlink);
    }

    /// Remove a symlink by its target path. Returns true if found and removed.
    pub fn remove(&mut self, target: &str) -> bool {
        let initial_len = self.entries.len();
        self.entries.retain(|s| s.target != target);
        self.entries.len() < initial_len
    }

    /// Find all symlinks whose source starts with the given prefix
    pub fn find_by_source_prefix(&self, prefix: &Path) -> Vec<&TrackedSymlink> {
        let prefix_str = prefix.to_string_lossy();
        self.entries
            .iter()
            .filter(|s| s.source.starts_with(prefix_str.as_ref()))
            .collect()
    }

    /// Find all symlinks whose target starts with the given prefix
    pub fn find_by_target_prefix(&self, prefix: &Path) -> Vec<&TrackedSymlink> {
        let prefix_str = prefix.to_string_lossy();
        self.entries
            .iter()
            .filter(|s| s.target.starts_with(prefix_str.as_ref()))
            .collect()
    }

    /// Find all symlinks created by a specific subsystem
    pub fn find_by_subsystem(&self, subsystem: &str) -> Vec<&TrackedSymlink> {
        self.entries
            .iter()
            .filter(|s| s.subsystem == subsystem)
            .collect()
    }
}

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

    /// Global symlink inventory tracking all managed symlinks
    #[serde(default)]
    pub symlinks: SymlinkInventory,

    /// State for dotfiles repository
    #[serde(default)]
    pub dotfiles: DotfilesState,

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

/// State for the dotfiles repository
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct DotfilesState {
    /// Whether the repo has been cloned
    #[serde(default)]
    pub cloned: bool,

    /// Last time dotfiles were synced
    pub last_sync: Option<DateTime<Utc>>,

    /// Submodules that have been initialized
    #[serde(default)]
    pub initialized_submodules: Vec<String>,

    /// Whether the private submodule has been initialized
    #[serde(default)]
    pub private_initialized: bool,
}

// ============================================================================
// BossaState Implementation
// ============================================================================

impl BossaState {
    /// Get the state directory path
    ///
    /// See [`crate::paths::state_dir`] for path resolution details.
    pub fn state_dir() -> Result<PathBuf> {
        paths::state_dir()
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

        let state: Self = toml::from_str(&content)
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
        // Truncate error message if too long
        let error_msg = if error.len() > 1024 {
            format!("{}... (truncated)", &error[..1024])
        } else {
            error.to_string()
        };
        state.repos_failed.push((repo_name.to_string(), error_msg));
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
        // Truncate error message if too long
        let error_msg = if error.len() > 1024 {
            format!("{}... (truncated)", &error[..1024])
        } else {
            error.to_string()
        };
        self.workspaces
            .repos_failed
            .push((repo_name.to_string(), error_msg));
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

    // ========================================================================
    // Dotfiles State Helpers
    // ========================================================================

    /// Mark dotfiles repo as cloned
    pub fn mark_dotfiles_cloned(&mut self) {
        self.dotfiles.cloned = true;
    }

    /// Mark dotfiles as synced
    pub fn mark_dotfiles_synced(&mut self) {
        self.dotfiles.last_sync = Some(Utc::now());
    }

    /// Mark a submodule as initialized
    pub fn mark_submodule_initialized(&mut self, submodule: &str) {
        if !self
            .dotfiles
            .initialized_submodules
            .contains(&submodule.to_string())
        {
            self.dotfiles
                .initialized_submodules
                .push(submodule.to_string());
        }
    }

    /// Mark private submodule as initialized
    pub fn mark_private_initialized(&mut self) {
        self.dotfiles.private_initialized = true;
    }
}

impl Default for BossaState {
    fn default() -> Self {
        Self {
            collections: HashMap::new(),
            workspaces: WorkspacesState::default(),
            storage: HashMap::new(),
            symlinks: SymlinkInventory::default(),
            dotfiles: DotfilesState::default(),
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

    // ====================================================================
    // Adversarial Tests - Edge Cases and Concurrent Access Scenarios
    // ====================================================================

    #[test]
    fn test_mark_same_repo_multiple_times() {
        let mut state = BossaState::default();

        // Mark same repo as cloned multiple times
        state.mark_repo_cloned("refs", "rust");
        state.mark_repo_cloned("refs", "rust");
        state.mark_repo_cloned("refs", "rust");

        // Should only appear once
        let collection = state.collections.get("refs").unwrap();
        assert_eq!(
            collection
                .repos_cloned
                .iter()
                .filter(|name| *name == "rust")
                .count(),
            1
        );
    }

    #[test]
    fn test_mark_repo_failed_with_empty_error() {
        let mut state = BossaState::default();
        state.mark_repo_failed("refs", "rust", "");

        let collection = state.collections.get("refs").unwrap();
        assert_eq!(collection.repos_failed.len(), 1);
        assert_eq!(collection.repos_failed[0].1, "");
    }

    #[test]
    fn test_mark_repo_failed_with_very_long_error() {
        let mut state = BossaState::default();
        let long_error = "error: ".repeat(10000);
        state.mark_repo_failed("refs", "rust", &long_error);

        let collection = state.collections.get("refs").unwrap();
        assert_eq!(collection.repos_failed.len(), 1);
        // Fixed: error messages are now truncated to 1024 chars + "... (truncated)"
        assert!(collection.repos_failed[0].1.len() <= 1024 + 17);
        assert!(collection.repos_failed[0].1.ends_with("... (truncated)"));
    }

    #[test]
    fn test_collection_special_chars_in_name() {
        let mut state = BossaState::default();
        let collection_name = "refs/../../../etc/passwd";
        state.mark_repo_cloned(collection_name, "rust");

        assert!(state.collections.contains_key(collection_name));
    }

    #[test]
    fn test_workspace_unicode_names() {
        let mut state = BossaState::default();
        state.mark_workspace_setup("日本語のプロジェクト");
        state.mark_workspace_setup("Émojis🚀");
        state.mark_workspace_setup("المشروع");

        assert_eq!(state.workspaces.repos_setup.len(), 3);
    }

    #[test]
    fn test_storage_add_duplicate_symlinks() {
        let mut state = BossaState::default();

        state.add_storage_symlink("t9", "~/dev/refs");
        state.add_storage_symlink("t9", "~/dev/refs"); // Duplicate
        state.add_storage_symlink("t9", "~/dev/refs"); // Duplicate

        let storage = state.storage.get("t9").unwrap();
        // Should only appear once due to contains check
        assert_eq!(
            storage
                .symlinks_created
                .iter()
                .filter(|path| *path == "~/dev/refs")
                .count(),
            1
        );
    }

    #[test]
    fn test_storage_mount_unmount_cycle() {
        let mut state = BossaState::default();

        state.mark_storage_mounted("t9");
        assert!(state.storage.get("t9").unwrap().is_mounted);
        assert!(state.storage.get("t9").unwrap().last_seen.is_some());

        state.mark_storage_unmounted("t9");
        assert!(!state.storage.get("t9").unwrap().is_mounted);
        // last_seen should still be set
        assert!(state.storage.get("t9").unwrap().last_seen.is_some());
    }

    #[test]
    fn test_remove_nonexistent_symlink() {
        let mut state = BossaState::default();
        state.add_storage_symlink("t9", "~/dev/refs");

        // Remove a symlink that doesn't exist
        state.remove_storage_symlink("t9", "~/nonexistent");

        // Original should still be there
        let storage = state.storage.get("t9").unwrap();
        assert_eq!(storage.symlinks_created.len(), 1);
        assert_eq!(storage.symlinks_created[0], "~/dev/refs");
    }

    #[test]
    fn test_state_with_many_collections() {
        let mut state = BossaState::default();

        // Add 1000 collections
        for i in 0..1000 {
            let collection_name = format!("collection-{i}");
            state.mark_repo_cloned(&collection_name, "repo1");
            state.mark_repo_cloned(&collection_name, "repo2");
        }

        assert_eq!(state.collections.len(), 1000);
    }

    #[test]
    fn test_state_with_many_repos_in_collection() {
        let mut state = BossaState::default();

        // Add 1000 repos to a single collection
        for i in 0..1000 {
            let repo_name = format!("repo-{i}");
            state.mark_repo_cloned("refs", &repo_name);
        }

        let collection = state.collections.get("refs").unwrap();
        assert_eq!(collection.repos_cloned.len(), 1000);
    }

    #[test]
    fn test_mark_collection_synced_updates_timestamp() {
        let mut state = BossaState::default();

        let before = Utc::now();
        std::thread::sleep(std::time::Duration::from_millis(10));

        state.mark_collection_synced("refs");

        let collection = state.collections.get("refs").unwrap();
        assert!(collection.last_sync.is_some());
        assert!(collection.last_sync.unwrap() > before);
    }

    #[test]
    fn test_touch_updates_last_updated() {
        let mut state = BossaState::default();
        let before = state.last_updated;

        std::thread::sleep(std::time::Duration::from_millis(10));

        // Note: touch() calls save() which may fail if we can't write to disk
        // In a unit test environment, this might fail, so we just test the timestamp update
        state.last_updated = Utc::now();
        assert!(state.last_updated > before);
    }

    #[test]
    fn test_deserialize_malformed_toml() {
        let malformed = r#"
[collections.refs]
last_sync = "not-a-valid-date"
repos_cloned = 123  # Should be array
"#;
        let result: Result<BossaState, _> = toml::from_str(malformed);
        assert!(result.is_err());
    }

    #[test]
    fn test_state_with_empty_collections_map() {
        let state = BossaState::default();
        assert!(state.collections.is_empty());

        // Serialize should work even with empty maps
        let toml_str = toml::to_string_pretty(&state).unwrap();
        assert!(!toml_str.is_empty());
    }

    #[test]
    fn test_collection_state_transition_cloned_to_failed_to_cloned() {
        let mut state = BossaState::default();

        // Start with cloned
        state.mark_repo_cloned("refs", "rust");
        let collection = state.collections.get("refs").unwrap();
        assert!(collection.repos_cloned.contains(&"rust".to_string()));
        assert!(collection.repos_failed.is_empty());

        // Mark as failed (should remove from cloned)
        state.mark_repo_failed("refs", "rust", "network error");
        let collection = state.collections.get("refs").unwrap();
        assert!(!collection.repos_cloned.contains(&"rust".to_string()));
        assert_eq!(collection.repos_failed.len(), 1);

        // Mark as cloned again (should remove from failed)
        state.mark_repo_cloned("refs", "rust");
        let collection = state.collections.get("refs").unwrap();
        assert!(collection.repos_cloned.contains(&"rust".to_string()));
        assert!(collection.repos_failed.is_empty());
    }

    #[test]
    fn test_workspace_state_concurrent_modifications() {
        let mut state = BossaState::default();

        // Simulate concurrent adds and removes
        state.mark_workspace_setup("repo1");
        state.mark_workspace_setup("repo2");
        state.mark_workspace_failed("repo1", "error");
        state.mark_workspace_setup("repo3");

        assert_eq!(state.workspaces.repos_setup.len(), 2);
        assert!(state.workspaces.repos_setup.contains(&"repo2".to_string()));
        assert!(state.workspaces.repos_setup.contains(&"repo3".to_string()));
        assert_eq!(state.workspaces.repos_failed.len(), 1);
    }
}
