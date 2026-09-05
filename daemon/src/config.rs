use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub expr: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub rules: Vec<Rule>,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    UnsupportedVersion(u32),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "could not read config file: {e}"),
            ConfigError::Parse(e) => write!(f, "could not parse config: {e}"),
            ConfigError::UnsupportedVersion(v) => {
                write!(f, "unsupported config version {v} (expected 1)")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
        let config: Config = serde_json::from_str(&raw).map_err(ConfigError::Parse)?;
        if config.version != 1 {
            return Err(ConfigError::UnsupportedVersion(config.version));
        }
        Ok(config)
    }

    /// `$XDG_CONFIG_HOME/plasma-keepawake/config.json`, falling back to
    /// `~/.config/plasma-keepawake/config.json`.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("plasma-keepawake").join("config.json")
    }
}
