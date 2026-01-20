//! macOS defaults resource - read/write system preferences

use anyhow::{Context, Result, bail};
use std::process::Command;

use super::{ApplyContext, ApplyResult, Resource, ResourceState, SudoRequirement};

/// A macOS default preference
#[derive(Debug, Clone)]
pub struct MacOSDefault {
    /// Domain (e.g., "com.apple.finder")
    pub domain: String,
    /// Key (e.g., "ShowPathbar")
    pub key: String,
    /// Desired value
    pub value: DefaultValue,
    /// Whether this default requires sudo
    pub requires_sudo: bool,
}

/// Value types for defaults
#[derive(Debug, Clone, PartialEq)]
pub enum DefaultValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

impl MacOSDefault {
    pub fn new(domain: &str, key: &str, value: DefaultValue) -> Self {
        Self {
            domain: domain.to_string(),
            key: key.to_string(),
            value,
            requires_sudo: false,
        }
    }

    pub fn with_sudo(mut self, requires: bool) -> Self {
        self.requires_sudo = requires;
        self
    }

    /// Parse "domain.key" format
    pub fn from_domain_key(domain_key: &str, value: DefaultValue) -> Result<Self> {
        // Handle NSGlobalDomain specially
        if domain_key.starts_with("NSGlobalDomain.") {
            let key = domain_key.strip_prefix("NSGlobalDomain.").unwrap();
            return Ok(Self::new("NSGlobalDomain", key, value));
        }

        // Find the last dot that separates domain from key
        let parts: Vec<&str> = domain_key.rsplitn(2, '.').collect();
        if parts.len() != 2 {
            bail!("Invalid domain.key format: {}", domain_key);
        }

        Ok(Self::new(parts[1], parts[0], value))
    }

    /// Read current value from defaults
    fn read_current(&self) -> Result<Option<DefaultValue>> {
        let output = Command::new("defaults")
            .args(["read", &self.domain, &self.key])
            .output()
            .context("Failed to execute defaults read")?;

        if !output.status.success() {
            // Key doesn't exist
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Try to parse as the same type as desired value
        let parsed = match &self.value {
            DefaultValue::Bool(_) => match stdout.as_str() {
                "1" | "true" => Some(DefaultValue::Bool(true)),
                "0" | "false" => Some(DefaultValue::Bool(false)),
                _ => None,
            },
            DefaultValue::Int(_) => stdout.parse::<i64>().ok().map(DefaultValue::Int),
            DefaultValue::Float(_) => stdout.parse::<f64>().ok().map(DefaultValue::Float),
            DefaultValue::String(_) => Some(DefaultValue::String(stdout)),
        };

        Ok(parsed)
    }

    /// Write value to defaults
    fn write_value(&self, ctx: &ApplyContext) -> Result<()> {
        let type_flag = match &self.value {
            DefaultValue::Bool(_) => "-bool",
            DefaultValue::Int(_) => "-int",
            DefaultValue::Float(_) => "-float",
            DefaultValue::String(_) => "-string",
        };

        let value_str = match &self.value {
            DefaultValue::Bool(b) => b.to_string(),
            DefaultValue::Int(i) => i.to_string(),
            DefaultValue::Float(f) => f.to_string(),
            DefaultValue::String(s) => s.clone(),
        };

        let args = ["write", &self.domain, &self.key, type_flag, &value_str];

        let output = if self.requires_sudo {
            ctx.sudo
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Sudo required but not available"))?
                .run("defaults", &args)?
        } else {
            Command::new("defaults")
                .args(args)
                .output()
                .context("Failed to execute defaults write")?
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("defaults write failed: {}", stderr.trim());
        }

        Ok(())
    }
}

impl Resource for MacOSDefault {
    fn id(&self) -> String {
        format!("{}.{}", self.domain, self.key)
    }

    fn description(&self) -> String {
        format!("Set {}.{} = {:?}", self.domain, self.key, self.value)
    }

    fn resource_type(&self) -> &'static str {
        "macos_default"
    }

    fn sudo_requirement(&self) -> SudoRequirement {
        if self.requires_sudo {
            SudoRequirement::Required {
                reason: format!("Setting {} requires sudo", self.id()),
            }
        } else {
            SudoRequirement::None
        }
    }

    fn current_state(&self) -> Result<ResourceState> {
        match self.read_current()? {
            None => Ok(ResourceState::Absent),
            Some(current) if current == self.value => Ok(ResourceState::Present {
                details: Some(format!("{:?}", current)),
            }),
            Some(current) => Ok(ResourceState::Modified {
                from: format!("{:?}", current),
                to: format!("{:?}", self.value),
            }),
        }
    }

    fn desired_state(&self) -> ResourceState {
        ResourceState::Present {
            details: Some(format!("{:?}", self.value)),
        }
    }

    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped {
                reason: "Dry run".to_string(),
            });
        }

        let current = self.current_state()?;

        match current {
            ResourceState::Present { .. } => Ok(ApplyResult::NoChange),
            _ => {
                self.write_value(ctx)?;
                Ok(ApplyResult::Modified)
            }
        }
    }
}
