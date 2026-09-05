use std::path::PathBuf;

use crate::config::{Config, Rule as ConfigRule};
use crate::engine::RuleEngine;

/// Everything the poll loop and the D-Bus service both need to touch,
/// behind one lock. Reload replaces `rule_engine` wholesale rather than
/// mutating it in place, so a bad config never leaves rules half-updated.
pub struct DaemonState {
    pub config_path: PathBuf,
    /// The config as last loaded from (or persisted to) disk. Source of
    /// truth for `AddRule`/`UpdateRule`/`RemoveRule`, which mutate this and
    /// write it back out. Deliberately separate from `rule_engine`'s live
    /// `enabled` flags: `SetRuleEnabled` is a transient in-memory override
    /// that a reload resets, while add/update/remove are meant to persist.
    config: Config,
    pub rule_engine: RuleEngine,
    pub inhibiting: bool,
    pub reason: String,
    /// Set when the last reload attempt failed, so a hand-edit mistake is
    /// visible without dropping the last known-good rules.
    pub reload_error: Option<String>,
}

impl DaemonState {
    pub fn new(config_path: PathBuf, config: Config) -> Self {
        DaemonState {
            config_path,
            rule_engine: RuleEngine::new(&config),
            config,
            inhibiting: false,
            reason: String::new(),
            reload_error: None,
        }
    }

    /// Reloads the config from disk. On failure, keeps the current
    /// `config`/`rule_engine` untouched and records the error.
    pub fn reload(&mut self) {
        match Config::load(&self.config_path) {
            Ok(config) => {
                self.rule_engine = RuleEngine::new(&config);
                self.config = config;
                self.reload_error = None;
            }
            Err(e) => {
                self.reload_error = Some(e.to_string());
            }
        }
    }

    pub fn add_rule(&mut self, name: String, expr: String, enabled: bool) -> Result<(), String> {
        if self.config.rules.iter().any(|r| r.name == name) {
            return Err(format!("a rule named {name:?} already exists"));
        }
        self.rule_engine.validate_expr(&expr)?;

        self.config.rules.push(ConfigRule {
            name,
            enabled,
            expr,
        });
        self.commit()
    }

    pub fn update_rule(&mut self, name: &str, expr: String) -> Result<(), String> {
        if !self.config.rules.iter().any(|r| r.name == name) {
            return Err(format!("no rule named {name:?}"));
        }
        self.rule_engine.validate_expr(&expr)?;

        for r in &mut self.config.rules {
            if r.name == name {
                r.expr = expr.clone();
            }
        }
        self.commit()
    }

    /// Returns an error if no rule named `name` exists.
    pub fn remove_rule(&mut self, name: &str) -> Result<(), String> {
        let before = self.config.rules.len();
        self.config.rules.retain(|r| r.name != name);
        if self.config.rules.len() == before {
            return Err(format!("no rule named {name:?}"));
        }
        self.commit()
    }

    /// Rebuilds `rule_engine` from the now-mutated `config` and persists it
    /// to disk. The in-memory state and the file are always changed
    /// together, so they can't drift out of sync.
    fn commit(&mut self) -> Result<(), String> {
        self.rule_engine = RuleEngine::new(&self.config);
        self.persist()
    }

    /// Writes `config` to `config_path`, via a temp file + rename so a
    /// crash mid-write can't leave a truncated config for the next load
    /// (ours or a future reload) to trip over.
    fn persist(&self) -> Result<(), String> {
        let mut json = serde_json::to_string_pretty(&self.config).map_err(|e| e.to_string())?;
        json.push('\n');
        let tmp_path = self.config_path.with_extension("json.tmp");
        std::fs::write(&tmp_path, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp_path, &self.config_path).map_err(|e| e.to_string())?;
        Ok(())
    }
}
