//! macOS defaults resource - read/write system preferences

use anyhow::{Context, Result, bail};
use std::fmt;

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

impl fmt::Display for DefaultValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefaultValue::Bool(b) => write!(f, "{b}"),
            DefaultValue::Int(i) => write!(f, "{i}"),
            DefaultValue::Float(v) => write!(f, "{v}"),
            DefaultValue::String(s) => write!(f, "{s}"),
        }
    }
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
        if let Some(key) = domain_key.strip_prefix("NSGlobalDomain.") {
            return Ok(Self::new("NSGlobalDomain", key, value));
        }

        // Find the last dot that separates domain from key
        let parts: Vec<&str> = domain_key.rsplitn(2, '.').collect();
        if parts.len() != 2 {
            bail!("Invalid domain.key format: {domain_key}");
        }

        Ok(Self::new(parts[1], parts[0], value))
    }

    /// Read current value from defaults
    fn read_current(&self) -> Result<Option<DefaultValue>> {
        if self.requires_sudo {
            bail!(
                "Privileged user-domain defaults are unsupported: elevation changes the observed preference namespace; no write was scheduled"
            );
        }
        if matches!(self.value, DefaultValue::Float(value) if !value.is_finite()) {
            bail!("Non-finite preference values are not supported");
        }
        super::native::identifier(&self.key, "preference key")?;
        let values = super::native::preferences(&self.domain)?;
        let Some(value) = values.get(&self.key) else {
            return Ok(None);
        };
        let value = match value {
            plist::Value::Boolean(value) => DefaultValue::Bool(*value),
            plist::Value::Integer(value) => DefaultValue::Int(
                value
                    .as_signed()
                    .context("Native integer is outside the supported signed range")?,
            ),
            plist::Value::Real(value) if value.is_finite() => DefaultValue::Float(*value),
            plist::Value::String(value) => DefaultValue::String(value.clone()),
            _ => bail!("Native preference has an unsupported value type; it was not replaced"),
        };
        Ok(Some(value))
    }

    /// Write value to defaults
    fn write_value(&self, _ctx: &ApplyContext) -> Result<()> {
        if self.requires_sudo {
            bail!("Privileged user-domain defaults are unsupported; no command was executed");
        }
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

        let output =
            crate::runner::capture("defaults", args).context("Failed to execute defaults write")?;
        if !output.status.success() {
            bail!(
                "defaults write failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
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
                details: Some(format!("{current:?}")),
            }),
            Some(current) => Ok(ResourceState::Modified {
                from: format!("{current:?}"),
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

#[cfg(test)]
mod tests {
    use super::{DefaultValue, MacOSDefault};

    #[test]
    fn from_domain_key_supports_ns_global_domain() {
        let pref = MacOSDefault::from_domain_key("NSGlobalDomain.KeyRepeat", DefaultValue::Int(2))
            .expect("NSGlobalDomain key should parse");

        assert_eq!(pref.domain, "NSGlobalDomain");
        assert_eq!(pref.key, "KeyRepeat");
    }

    #[test]
    fn from_domain_key_rejects_missing_separator() {
        let result = MacOSDefault::from_domain_key("invalid", DefaultValue::Bool(true));
        assert!(result.is_err());
    }
}
