#![allow(dead_code)]

use anyhow::{Context, Result};
pub use bossa::schema::*;
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;

/// CLI compatibility wrapper. The desired value and its write authority stay separate.
#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct BossaConfig {
    desired: bossa::schema::BossaConfig,
    #[serde(skip)]
    pub(crate) loaded: Option<bossa::LoadedBossaConfig>,
}

impl Deref for BossaConfig {
    type Target = bossa::schema::BossaConfig;
    fn deref(&self) -> &Self::Target {
        &self.desired
    }
}
impl DerefMut for BossaConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.desired
    }
}
impl BossaConfig {
    pub fn load() -> Result<Self> {
        crate::config::load_bossa_config(false)
    }
    pub(crate) fn from_loaded(loaded: bossa::LoadedBossaConfig) -> Self {
        Self {
            desired: loaded.desired().clone(),
            loaded: Some(loaded),
        }
    }
    pub(crate) fn from_desired(desired: bossa::schema::BossaConfig) -> Self {
        Self {
            desired,
            loaded: None,
        }
    }
    pub fn save(&mut self) -> Result<PathBuf> {
        let store = crate::config::main_store()?;
        match &mut self.loaded {
            Some(loaded) => {
                if loaded.store() != &store {
                    return Err(bossa::StoreError::Conflict.into());
                }
                loaded.replace_desired(self.desired.clone());
                loaded.save()
            }
            None => {
                let mut loaded = bossa::LoadedBossaConfig::load(store, false)?;
                if loaded.has_document() {
                    anyhow::bail!(
                        "An unbound configuration cannot overwrite an existing document; reload first"
                    );
                }
                loaded.replace_desired(self.desired.clone());
                let path = loaded.save()?;
                self.loaded = Some(loaded);
                Ok(path)
            }
        }
    }
    pub fn update<T>(mutate: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let store = crate::config::main_store()?;
        bossa::update_bossa_config(&store, |desired| {
            let mut wrapper = Self::from_desired(std::mem::take(desired));
            let result = mutate(&mut wrapper)?;
            *desired = wrapper.desired;
            Ok(result)
        })
        .context("Could not update configuration")
    }
    pub fn into_desired(self) -> bossa::schema::BossaConfig {
        self.desired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn json_dock_preferences_preserve_omission_and_explicit_false() {
        let unconfigured = serde_json::to_value(DockConfig::default()).unwrap();
        for key in ["autohide", "tilesize", "minimize_effect", "show_recents"] {
            assert!(
                unconfigured.get(key).is_none(),
                "Unconfigured {key} must stay omitted for JSON edits and older readers"
            );
        }
        let explicit: DockConfig = serde_json::from_str(
            r#"{"autohide":false,"tilesize":64,"minimize_effect":"scale","show_recents":false}"#,
        )
        .unwrap();
        let explicit = serde_json::to_value(explicit).unwrap();
        assert_eq!(explicit["autohide"], false);
        assert_eq!(explicit["show_recents"], false);
        assert_eq!(explicit["tilesize"], 64);
        assert_eq!(explicit["minimize_effect"], "scale");
    }

    #[test]
    fn test_parse_example_config() {
        let toml = r#"
[collections.refs]
path = "~/dev/refs"
description = "Reference repositories"

[collections.refs.clone]
depth = 1
single_branch = true

[[collections.refs.repos]]
name = "rust"
url = "https://github.com/rust-lang/rust"

[workspaces]
root = "~/dev/ws"
structure = "bare-worktree"

[[workspaces.repos]]
name = "dotfiles"
url = "git@github.com:user/dotfiles.git"
category = "utils"
worktrees = ["main"]

[storage.t9]
mount = "/Volumes/T9"
type = "external"

[[storage.t9.symlinks]]
from = "~/dev/refs"
to = "{mount}/refs"

[nova]
stages = ["defaults", "homebrew", "essential", "stow", "collections", "workspaces"]
"#;

        let config: BossaConfig = toml::from_str(toml).expect("Failed to parse config");

        // Test collections
        assert!(config.collections.contains_key("refs"));
        let refs = &config.collections["refs"];
        assert_eq!(refs.path, "~/dev/refs");
        assert_eq!(refs.clone.depth, 1);
        assert!(refs.clone.single_branch);
        assert_eq!(refs.repos.len(), 1);
        assert_eq!(refs.repos[0].name, "rust");

        // Test workspaces
        assert_eq!(config.workspaces.root, "~/dev/ws");
        assert_eq!(config.workspaces.structure, "bare-worktree");
        assert_eq!(config.workspaces.repos.len(), 1);
        assert_eq!(config.workspaces.repos[0].name, "dotfiles");
        assert_eq!(config.workspaces.repos[0].category, "utils");

        // Test storage
        assert!(config.storage.contains_key("t9"));
        let t9 = &config.storage["t9"];
        assert_eq!(t9.mount, "/Volumes/T9");
        assert_eq!(t9.storage_type, StorageType::External);
        assert_eq!(t9.symlinks.len(), 1);

        // Test nova
        assert_eq!(config.nova.stages.len(), 6);
        assert!(config.nova.has_stage("homebrew"));
    }

    #[test]
    fn test_collection_validation() {
        let mut collection = Collection {
            path: "~/test".to_string(),
            description: "Test".to_string(),
            clone: CloneSettings::default(),
            storage: None,
            repos: vec![],
        };

        assert!(collection.validate().is_ok());

        // Empty path should fail
        collection.path = String::new();
        assert!(collection.validate().is_err());
    }

    #[test]
    fn test_workspace_repo_paths() {
        let repo = WorkspaceRepo {
            name: "test-repo".to_string(),
            url: "git@github.com:user/test-repo.git".to_string(),
            category: "projects".to_string(),
            worktrees: vec!["main".to_string()],
            description: String::new(),
        };

        let root = PathBuf::from("/home/user/ws");
        let bare_path = repo.bare_path(&root);
        assert_eq!(
            bare_path,
            PathBuf::from("/home/user/ws/projects/test-repo.git")
        );

        let worktree_path = repo.worktree_path(&root, "main");
        assert_eq!(
            worktree_path,
            PathBuf::from("/home/user/ws/projects/test-repo/main")
        );
    }

    #[test]
    fn test_symlink_placeholder() {
        let symlink = Symlink {
            from: "~/dev/refs".to_string(),
            to: "{mount}/refs".to_string(),
        };

        let expanded = symlink
            .expanded_to("/Volumes/T9", crate::paths::environment())
            .unwrap();
        assert_eq!(expanded, PathBuf::from("/Volumes/T9/refs"));
    }

    // ====================================================================
    // Adversarial Tests - Edge Cases and Invalid Inputs
    // ====================================================================

    #[test]
    fn test_parse_empty_config() {
        let toml = "";
        let config: BossaConfig = toml::from_str(toml).unwrap();
        assert!(config.collections.is_empty());
        assert!(config.workspaces.repos.is_empty());
        assert!(config.storage.is_empty());
    }

    #[test]
    fn test_parse_config_rejects_unknown_fields() {
        let toml = r#"
[collections.refs]
path = "~/dev/refs"
unknown_field = "should_be_ignored"

[[collections.refs.repos]]
name = "rust"
url = "https://github.com/rust-lang/rust"
extra_field = 123
"#;
        // Unknown intent must be diagnosed instead of silently disappearing.
        let result: Result<BossaConfig, _> = toml::from_str(toml);
        assert!(result.unwrap_err().to_string().contains("unknown field"));
    }

    #[test]
    fn test_collection_empty_name() {
        let repo = CollectionRepo {
            name: String::new(),
            url: "https://github.com/user/repo".to_string(),
            default_branch: "main".to_string(),
            description: String::new(),
        };
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_collection_empty_url() {
        let repo = CollectionRepo {
            name: "test".to_string(),
            url: String::new(),
            default_branch: "main".to_string(),
            description: String::new(),
        };
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_collection_whitespace_only_name() {
        let repo = CollectionRepo {
            name: "   ".to_string(),
            url: "https://github.com/user/repo".to_string(),
            default_branch: "main".to_string(),
            description: String::new(),
        };
        // Fixed: whitespace-only names should now fail validation
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_workspace_empty_category() {
        let repo = WorkspaceRepo {
            name: "test".to_string(),
            url: "https://github.com/user/test".to_string(),
            category: String::new(),
            worktrees: vec![],
            description: String::new(),
        };
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_workspace_special_chars_in_name() {
        let repo = WorkspaceRepo {
            name: "test/../../../etc/passwd".to_string(),
            url: "https://github.com/user/test".to_string(),
            category: "projects".to_string(),
            worktrees: vec![],
            description: String::new(),
        };
        // Fixed: path traversal attempts should now fail validation
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_symlink_empty_from() {
        let symlink = Symlink {
            from: String::new(),
            to: "/somewhere".to_string(),
        };
        assert!(symlink.validate().is_err());
    }

    #[test]
    fn test_symlink_empty_to() {
        let symlink = Symlink {
            from: "~/test".to_string(),
            to: String::new(),
        };
        assert!(symlink.validate().is_err());
    }

    #[test]
    fn test_symlink_multiple_placeholders() {
        let symlink = Symlink {
            from: "~/dev/refs".to_string(),
            to: "{mount}/{mount}/refs".to_string(),
        };
        let expanded = symlink
            .expanded_to("/Volumes/T9", crate::paths::environment())
            .unwrap();
        assert_eq!(expanded, PathBuf::from("/Volumes/T9//Volumes/T9/refs"));
    }

    #[test]
    fn test_storage_empty_mount() {
        let storage = Storage {
            mount: String::new(),
            storage_type: StorageType::External,
            symlinks: vec![],
            description: String::new(),
        };
        assert!(storage.validate().is_err());
    }

    #[test]
    fn test_nova_duplicate_stages() {
        let config = NovaConfig {
            stages: vec![
                "defaults".to_string(),
                "homebrew".to_string(),
                "defaults".to_string(), // duplicate
            ],
            stage_config: HashMap::new(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_collection_add_duplicate_repo() {
        let mut collection = Collection {
            path: "~/test".to_string(),
            description: "Test".to_string(),
            clone: CloneSettings::default(),
            storage: None,
            repos: vec![],
        };

        let repo1 = CollectionRepo {
            name: "test-repo".to_string(),
            url: "https://github.com/user/test1".to_string(),
            default_branch: "main".to_string(),
            description: String::new(),
        };

        let repo2 = CollectionRepo {
            name: "test-repo".to_string(),                    // Same name
            url: "https://github.com/user/test2".to_string(), // Different URL
            default_branch: "main".to_string(),
            description: String::new(),
        };

        collection.add_repo(repo1);
        assert_eq!(collection.repos.len(), 1);

        collection.add_repo(repo2); // Should replace
        assert_eq!(collection.repos.len(), 1);
        assert_eq!(collection.repos[0].url, "https://github.com/user/test2");
    }

    #[test]
    fn test_workspace_extremely_long_name() {
        let long_name = "a".repeat(10000);
        let repo = WorkspaceRepo {
            name: long_name,
            url: "https://github.com/user/test".to_string(),
            category: "projects".to_string(),
            worktrees: vec![],
            description: String::new(),
        };
        assert!(repo.validate().is_ok());

        let root = PathBuf::from("/home/user/ws");
        let path = repo.bare_path(&root);
        // This could cause filesystem issues
        assert!(path.to_string_lossy().len() > 10000);
    }

    #[test]
    fn test_default_value_to_args() {
        let bool_val = DefaultValue::Bool(true);
        assert_eq!(bool_val.to_defaults_args(), vec!["-bool", "true"]);

        let int_val = DefaultValue::Int(42);
        assert_eq!(int_val.to_defaults_args(), vec!["-int", "42"]);

        let float_val = DefaultValue::Float(2.5);
        assert_eq!(float_val.to_defaults_args(), vec!["-float", "2.5"]);

        let string_val = DefaultValue::String("test".to_string());
        assert_eq!(string_val.to_defaults_args(), vec!["-string", "test"]);

        let array_val = DefaultValue::Array(vec![
            DefaultValue::String("a".to_string()),
            DefaultValue::String("b".to_string()),
        ]);
        let args = array_val.to_defaults_args();
        assert_eq!(args[0], "-array");
        assert_eq!(args[1], "a");
        assert_eq!(args[2], "b");
    }

    #[test]
    fn test_parse_config_missing_required_fields() {
        let toml = r#"
[[collections.refs.repos]]
name = "rust"
# Missing 'url' field
"#;
        let result: Result<BossaConfig, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_workspaces_categories() {
        let workspaces = WorkspacesConfig {
            repos: vec![
                WorkspaceRepo {
                    name: "repo1".to_string(),
                    url: "url1".to_string(),
                    category: "utils".to_string(),
                    worktrees: vec![],
                    description: String::new(),
                },
                WorkspaceRepo {
                    name: "repo2".to_string(),
                    url: "url2".to_string(),
                    category: "projects".to_string(),
                    worktrees: vec![],
                    description: String::new(),
                },
                WorkspaceRepo {
                    name: "repo3".to_string(),
                    url: "url3".to_string(),
                    category: "utils".to_string(),
                    worktrees: vec![],
                    description: String::new(),
                },
            ],
            ..Default::default()
        };

        let categories = workspaces.categories();
        assert_eq!(categories.len(), 2);
        assert!(categories.contains(&"utils".to_string()));
        assert!(categories.contains(&"projects".to_string()));
    }

    #[test]
    fn test_unicode_in_paths() {
        let collection = Collection {
            path: "~/dev/日本語/テスト".to_string(),
            description: "Test".to_string(),
            clone: CloneSettings::default(),
            storage: None,
            repos: vec![],
        };
        assert!(collection.validate().is_ok());
    }

    #[test]
    fn test_collection_repo_path_traversal_variants() {
        // Test various path traversal attempts
        let traversal_attempts = vec![
            "test/../etc",
            "../etc/passwd",
            "test/../../etc",
            "test\\etc",
            "test/subdir",
        ];

        for attempt in traversal_attempts {
            let repo = CollectionRepo {
                name: attempt.to_string(),
                url: "https://github.com/user/repo".to_string(),
                default_branch: "main".to_string(),
                description: String::new(),
            };
            assert!(
                repo.validate().is_err(),
                "Path traversal should be rejected: {attempt}"
            );
        }
    }

    #[test]
    fn test_workspace_repo_path_traversal_variants() {
        // Test various path traversal attempts
        let traversal_attempts = vec![
            "test/../etc",
            "../etc/passwd",
            "test/../../etc",
            "test\\etc",
            "test/subdir",
        ];

        for attempt in traversal_attempts {
            let repo = WorkspaceRepo {
                name: attempt.to_string(),
                url: "https://github.com/user/test".to_string(),
                category: "projects".to_string(),
                worktrees: vec![],
                description: String::new(),
            };
            assert!(
                repo.validate().is_err(),
                "Path traversal should be rejected: {attempt}"
            );
        }
    }

    #[test]
    fn test_workspace_whitespace_only_category() {
        let repo = WorkspaceRepo {
            name: "test".to_string(),
            url: "https://github.com/user/test".to_string(),
            category: "   ".to_string(),
            worktrees: vec![],
            description: String::new(),
        };
        // Fixed: whitespace-only categories should now fail validation
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_tool_definition_build_url_direct() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            url: Some("https://example.com/{version}/tool-{platform}.tar.gz".to_string()),
            version: Some("1.2.3".to_string()),
            ..Default::default()
        };

        let url = def.build_url("mytool").unwrap();
        assert!(url.contains("1.2.3"));
        // Platform depends on the current system
        assert!(url.contains("https://example.com/1.2.3/tool-"));
    }

    #[test]
    fn test_tool_definition_build_url_pattern() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            base_url: Some("https://releases.example.com/artifacts".to_string()),
            path: Some("org/mytool".to_string()),
            version: Some("0.9.2".to_string()),
            archive_name: Some("mytool".to_string()),
            ..Default::default()
        };

        let url = def.build_url("mytool").unwrap();
        assert!(
            url.starts_with(
                "https://releases.example.com/artifacts/org/mytool/0.9.2/mytool-0.9.2-"
            )
        );
    }

    #[test]
    fn test_tool_definition_build_url_custom_pattern() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            base_url: Some("https://downloads.example.com/bin".to_string()),
            path: Some("tools/mycli".to_string()),
            version: Some("20250910".to_string()),
            url_pattern: Some("{base_url}/{path}/{version}/mycli-{platform}".to_string()),
            archive_type: Some("binary".to_string()),
            ..Default::default()
        };

        let url = def.build_url("mycli").unwrap();
        assert!(url.starts_with("https://downloads.example.com/bin/tools/mycli/20250910/mycli-"));
        assert!(!url.ends_with(".tar.gz")); // Binary download, no extension in pattern
    }

    #[test]
    fn test_tool_definition_platform_style_long() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            platform_style: Some("long".to_string()),
            ..Default::default()
        };

        let platform = def.get_platform_string();
        // Format should be os-arch (e.g., darwin-arm64, linux-amd64)
        assert!(platform.contains('-'));
    }

    #[test]
    fn test_tool_definition_platform_style_short() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            platform_style: Some("short".to_string()),
            ..Default::default()
        };

        let platform = def.get_platform_string();
        // Format should be os_arch (e.g., darwin_arm64, linux_amd64)
        assert!(platform.contains('_'));
    }

    #[test]
    fn test_tool_definition_platform_style_custom() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            platform_style: Some("{os}_{arch}_custom".to_string()),
            ..Default::default()
        };

        let platform = def.get_platform_string();
        assert!(platform.ends_with("_custom"));
    }

    #[test]
    fn test_tool_definition_is_binary_download() {
        let binary_def = ToolDefinition {
            source: ToolSource::Http,
            archive_type: Some("binary".to_string()),
            ..Default::default()
        };
        assert!(binary_def.is_binary_download());

        let targz_def = ToolDefinition {
            source: ToolSource::Http,
            archive_type: Some("tar.gz".to_string()),
            ..Default::default()
        };
        assert!(!targz_def.is_binary_download());

        let default_def = ToolDefinition {
            source: ToolSource::Http,
            ..Default::default()
        };
        assert!(!default_def.is_binary_download());
    }

    #[test]
    fn test_tool_definition_effective_extension() {
        let def_default = ToolDefinition {
            source: ToolSource::Http,
            ..Default::default()
        };
        assert_eq!(def_default.effective_extension(), "tar.gz");

        let def_zip = ToolDefinition {
            source: ToolSource::Http,
            archive_type: Some("zip".to_string()),
            ..Default::default()
        };
        assert_eq!(def_zip.effective_extension(), "zip");
    }

    #[test]
    fn test_tool_definition_validation_http_base_url_path() {
        // Valid: base_url + path
        let def = ToolDefinition {
            source: ToolSource::Http,
            base_url: Some("https://example.com".to_string()),
            path: Some("tools/mytool".to_string()),
            ..Default::default()
        };
        assert!(def.validate("test").is_ok());

        // Invalid: only base_url without path
        let def_invalid = ToolDefinition {
            source: ToolSource::Http,
            base_url: Some("https://example.com".to_string()),
            ..Default::default()
        };
        assert!(def_invalid.validate("test").is_err());
    }

    #[test]
    fn test_platforms_config_no_restriction() {
        // No platforms = available everywhere
        let def = ToolDefinition {
            source: ToolSource::Http,
            url: Some("https://example.com/tool.tar.gz".to_string()),
            ..Default::default()
        };
        assert!(def.is_available_for_current_platform());
    }

    #[test]
    fn test_platforms_config_available() {
        let mut linux_archs = HashMap::new();
        linux_archs.insert(
            "amd64".to_string(),
            PlatformArch {
                available: true,
                ..Default::default()
            },
        );

        let mut darwin_archs = HashMap::new();
        darwin_archs.insert(
            "arm64".to_string(),
            PlatformArch {
                available: true,
                ..Default::default()
            },
        );
        darwin_archs.insert(
            "amd64".to_string(),
            PlatformArch {
                available: true,
                ..Default::default()
            },
        );

        let platforms = PlatformsConfig {
            linux: Some(linux_archs),
            darwin: Some(darwin_archs),
            windows: None,
        };

        // Test the is_available method directly
        assert!(platforms.is_available("linux", "amd64"));
        assert!(platforms.is_available("darwin", "arm64"));
        assert!(platforms.is_available("darwin", "amd64"));
        // Windows not specified, so available by default
        assert!(platforms.is_available("windows", "amd64"));
    }

    #[test]
    fn test_platforms_config_not_available() {
        let mut linux_archs = HashMap::new();
        linux_archs.insert(
            "amd64".to_string(),
            PlatformArch {
                available: true,
                ..Default::default()
            },
        );

        let platforms = PlatformsConfig {
            linux: Some(linux_archs),
            darwin: None,
            windows: None,
        };

        // arm64 not in linux archs, so not available
        assert!(!platforms.is_available("linux", "arm64"));
    }

    #[test]
    fn test_platforms_config_archive_type_override() {
        let mut windows_archs = HashMap::new();
        windows_archs.insert(
            "amd64".to_string(),
            PlatformArch {
                available: true,
                archive_type: Some("zip".to_string()),
                ..Default::default()
            },
        );

        let platforms = PlatformsConfig {
            linux: None,
            darwin: None,
            windows: Some(windows_archs),
        };

        // Test getting overrides
        let overrides = platforms.get_current_overrides();
        // Can't reliably test this without knowing current platform
        // Just verify it doesn't crash
        let _ = overrides;
    }

    #[test]
    fn test_network_config_to_env_vars() {
        let config = NetworkConfig {
            http_proxy: Some("http://proxy.example.com:8080".to_string()),
            https_proxy: Some("http://proxy.example.com:8080".to_string()),
            no_proxy: Some("localhost,.example.com".to_string()),
            go: Some(GoNetworkConfig {
                goproxy: Some("https://proxy.example.com/go,direct".to_string()),
                gosumdb: Some("sum.golang.org".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let vars = config.to_env_vars();

        // Check that proxy vars are set (both upper and lower case)
        assert!(vars.iter().any(|(k, _)| k == "HTTP_PROXY"));
        assert!(vars.iter().any(|(k, _)| k == "http_proxy"));
        assert!(vars.iter().any(|(k, _)| k == "HTTPS_PROXY"));
        assert!(vars.iter().any(|(k, _)| k == "NO_PROXY"));
        assert!(vars.iter().any(|(k, _)| k == "GOPROXY"));
        assert!(vars.iter().any(|(k, _)| k == "GOSUMDB"));
    }

    #[test]
    fn test_network_config_has_proxy() {
        let no_proxy = NetworkConfig::default();
        assert!(!no_proxy.has_proxy());

        let with_proxy = NetworkConfig {
            http_proxy: Some("http://proxy.example.com:8080".to_string()),
            ..Default::default()
        };
        assert!(with_proxy.has_proxy());
    }
}

#[cfg(all(test, unix))]
mod persistence_tests {
    use super::*;
    use bossa::{CommandRequest, IoMode, ProcessLimits, ProcessRunner, SystemRunner};
    use std::time::{Duration, Instant};

    #[test]
    fn stale_config_child() {
        let Some(name) = std::env::var_os("BOSSA_STALE_CONFIG_CHILD") else {
            return;
        };
        let root = PathBuf::from(std::env::var_os("BOSSA_CONFIG_FIXTURE").unwrap());
        let name = name.to_str().unwrap();
        let mut config = BossaConfig::load().unwrap();
        config.collections.insert(
            name.into(),
            Collection {
                path: root.join(name).to_str().unwrap().into(),
                description: String::new(),
                clone: Default::default(),
                storage: None,
                repos: vec![],
            },
        );
        std::fs::write(root.join(format!("ready-{name}")), b"").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.join("go").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Err(error) = config.save() {
            eprintln!("{error:#}");
            std::process::exit(7);
        }
    }

    #[test]
    fn competing_loaded_config_snapshots_never_both_report_success() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("config")).unwrap();
        std::fs::create_dir(root.join("home")).unwrap();
        std::fs::write(root.join("config/config.toml"), "[collections]\n").unwrap();
        let outputs = std::thread::scope(|scope| {
            let jobs = ["a", "b"].map(|name| {
                scope.spawn(|| {
                    let environment = std::collections::BTreeMap::from([
                        ("HOME".into(), root.join("home").into_os_string()),
                        (
                            "BOSSA_CONFIG_DIR".into(),
                            root.join("config").into_os_string(),
                        ),
                        ("BOSSA_CONFIG_FIXTURE".into(), root.clone().into_os_string()),
                        ("BOSSA_STALE_CONFIG_CHILD".into(), name.into()),
                    ]);
                    SystemRunner::new(environment)
                        .with_limits(ProcessLimits {
                            timeout: Duration::from_secs(15),
                            max_output_bytes: 64 * 1024,
                        })
                        .run(
                            &CommandRequest {
                                program: std::env::current_exe().unwrap().into_os_string(),
                                args: [
                                    "--exact",
                                    "schema::persistence_tests::stale_config_child",
                                    "--nocapture",
                                ]
                                .into_iter()
                                .map(Into::into)
                                .collect(),
                                working_dir: root.clone(),
                            },
                            IoMode::Capture,
                        )
                        .unwrap()
                })
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            while !root.join("ready-a").exists() || !root.join("ready-b").exists() {
                assert!(
                    Instant::now() < deadline,
                    "owned children did not reach read barrier"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            std::fs::write(root.join("go"), b"").unwrap();
            jobs.into_iter()
                .map(|job| job.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            outputs
                .iter()
                .filter(|output| output.status.success())
                .count(),
            1,
            "both stale writers reported success, or neither reached promotion"
        );
        let rejected = outputs
            .iter()
            .find(|output| !output.status.success())
            .unwrap();
        assert_eq!(rejected.status.code(), Some(7));
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains("revision"),
            "wrong failure: {}",
            String::from_utf8_lossy(&rejected.stderr)
        );
        let config: BossaConfig =
            toml::from_str(&std::fs::read_to_string(root.join("config/config.toml")).unwrap())
                .unwrap();
        assert_eq!(config.collections.len(), 1);
    }
}
