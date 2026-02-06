//! Shell config scanner - finds hardcoded paths in shell rc files

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// A reference to a path found in a config file
#[derive(Debug, Clone)]
pub struct PathReference {
    /// The file where this reference was found
    pub file: PathBuf,
    /// Line number (1-indexed)
    pub line: usize,
    /// The full line content
    pub content: String,
    /// The path that was found
    #[allow(dead_code)] // Will be used by future features
    pub path: String,
    /// Type of reference (alias, export, etc.)
    pub ref_type: RefType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefType {
    /// cd alias: alias foo="cd /path"
    CdAlias,
    /// Export: export VAR=/path
    Export,
    /// PATH addition: PATH=/path:$PATH
    PathAddition,
    /// Source command: source /path/file
    Source,
    /// Generic path reference
    Other,
}

/// Scanner for shell configuration files
pub struct ShellScanner {
    /// Path prefix to search for
    search_path: PathBuf,
}

impl ShellScanner {
    pub fn new(search_path: impl AsRef<Path>) -> Self {
        Self {
            search_path: search_path.as_ref().to_path_buf(),
        }
    }

    /// Scan common shell rc files for references to the search path
    pub fn scan_all(&self) -> Result<Vec<PathReference>> {
        let mut results = Vec::new();

        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;

        // Common shell rc files
        let rc_files = [
            home.join(".zshrc"),
            home.join(".bashrc"),
            home.join(".bash_profile"),
            home.join(".profile"),
            home.join(".zprofile"),
            home.join(".config/fish/config.fish"),
        ];

        for file in &rc_files {
            if file.exists() {
                results.extend(self.scan_file(file)?);
            }
        }

        // Also scan fish conf.d
        let fish_conf_d = home.join(".config/fish/conf.d");
        if fish_conf_d.exists() {
            for entry in fs::read_dir(&fish_conf_d)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "fish") {
                    results.extend(self.scan_file(&path)?);
                }
            }
        }

        Ok(results)
    }

    /// Scan a single file for path references
    pub fn scan_file(&self, file: &Path) -> Result<Vec<PathReference>> {
        let content = fs::read_to_string(file)?;
        let search_str = self.search_path.to_string_lossy();

        let mut results = Vec::new();

        for (idx, line) in content.lines().enumerate() {
            // Skip comments
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }

            // Check if line contains our search path
            if line.contains(search_str.as_ref()) {
                let ref_type = self.detect_ref_type(line);
                results.push(PathReference {
                    file: file.to_path_buf(),
                    line: idx + 1,
                    content: line.to_string(),
                    path: self.extract_path(line),
                    ref_type,
                });
            }
        }

        Ok(results)
    }

    fn detect_ref_type(&self, line: &str) -> RefType {
        let trimmed = line.trim();

        if trimmed.starts_with("alias ") && trimmed.contains("cd ") {
            RefType::CdAlias
        } else if trimmed.starts_with("export ") {
            RefType::Export
        } else if trimmed.contains("PATH=") || trimmed.contains("PATH:") {
            RefType::PathAddition
        } else if trimmed.starts_with("source ") || trimmed.starts_with(". ") {
            RefType::Source
        } else {
            RefType::Other
        }
    }

    fn extract_path(&self, line: &str) -> String {
        // Try to extract the actual path from the line
        let search_str = self.search_path.to_string_lossy();

        // Find where our search path starts
        if let Some(start) = line.find(search_str.as_ref()) {
            // Find the end of the path (space, quote, or end of line)
            let rest = &line[start..];
            let end = rest.find(['"', '\'', ' ', ':', ';']).unwrap_or(rest.len());
            return rest[..end].to_string();
        }

        self.search_path.to_string_lossy().to_string()
    }

    /// Generate replacement line with new path
    pub fn replace_path(line: &str, old_path: &str, new_path: &str) -> String {
        line.replace(old_path, new_path)
    }
}
