//! Pure typed configuration decoding.
use crate::{ConfigFormat, schema::BossaConfig};
use anyhow::{Context, Result};

#[derive(Debug)]
pub struct DecodedBossaConfig {
    pub desired: BossaConfig,
    pub legacy: bool,
}

/// Decode explicit bytes without filesystem or environment access.
pub fn decode_bossa_config(format: ConfigFormat, bytes: &[u8]) -> Result<DecodedBossaConfig> {
    let (desired, legacy) = match format {
        ConfigFormat::Json => crate::legacy_config::decode(crate::decode_unique_json(bytes)?)?,
        ConfigFormat::Toml => {
            let contents = std::str::from_utf8(bytes).context("Configuration is not UTF-8")?;
            crate::legacy_config::decode_toml(toml::from_str(contents)?)?
        }
    };
    Ok(DecodedBossaConfig { desired, legacy })
}
