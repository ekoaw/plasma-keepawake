use std::path::PathBuf;

use crate::config::Config;
use crate::engine::RuleEngine;

/// Everything the poll loop and the D-Bus service both need to touch,
/// behind one lock. Reload replaces `rule_engine` wholesale rather than
/// mutating it in place, so a bad config never leaves rules half-updated.
pub struct DaemonState {
    pub config_path: PathBuf,
    pub rule_engine: RuleEngine,
    pub inhibiting: bool,
    pub reason: String,
    /// Set when the last reload attempt failed, so a hand-edit mistake is
    /// visible without dropping the last known-good rules.
    pub reload_error: Option<String>,
}

impl DaemonState {
    pub fn new(config_path: PathBuf, config: &Config) -> Self {
        DaemonState {
            config_path,
            rule_engine: RuleEngine::new(config),
            inhibiting: false,
            reason: String::new(),
            reload_error: None,
        }
    }

    /// Reloads the config from disk. On failure, keeps the current
    /// `rule_engine` untouched and records the error.
    pub fn reload(&mut self) {
        match Config::load(&self.config_path) {
            Ok(config) => {
                self.rule_engine = RuleEngine::new(&config);
                self.reload_error = None;
            }
            Err(e) => {
                self.reload_error = Some(e.to_string());
            }
        }
    }
}
