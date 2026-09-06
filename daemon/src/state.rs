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

    /// Renames a rule in place, keeping its `expr`/`enabled`/live value.
    /// A no-op (but still `Ok`) if `new_name == old_name`.
    pub fn rename_rule(&mut self, old_name: &str, new_name: &str) -> Result<(), String> {
        if old_name == new_name {
            return Ok(());
        }
        if !self.config.rules.iter().any(|r| r.name == old_name) {
            return Err(format!("no rule named {old_name:?}"));
        }
        if self.config.rules.iter().any(|r| r.name == new_name) {
            return Err(format!("a rule named {new_name:?} already exists"));
        }

        for r in &mut self.config.rules {
            if r.name == old_name {
                r.name = new_name.to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_state(rule_names: &[&str]) -> DaemonState {
        let path = std::env::temp_dir().join(format!(
            "plasma-keepawake-test-{}-{}.json",
            std::process::id(),
            rule_names.join("-")
        ));
        let config = Config {
            version: 1,
            rules: rule_names
                .iter()
                .map(|name| ConfigRule {
                    name: name.to_string(),
                    enabled: true,
                    expr: "true".to_string(),
                })
                .collect(),
        };
        DaemonState::new(path, config)
    }

    #[test]
    fn rename_rule_updates_name_and_persists() {
        let mut state = scratch_state(&["a", "b"]);
        state.rename_rule("a", "renamed").unwrap();
        assert!(state.config.rules.iter().any(|r| r.name == "renamed"));
        assert!(!state.config.rules.iter().any(|r| r.name == "a"));

        // Persisted, not just held in memory.
        let reloaded = Config::load(&state.config_path).unwrap();
        assert!(reloaded.rules.iter().any(|r| r.name == "renamed"));
        std::fs::remove_file(&state.config_path).ok();
    }

    #[test]
    fn rename_rule_rejects_unknown_source() {
        let mut state = scratch_state(&["a"]);
        assert!(state.rename_rule("nope", "renamed").is_err());
        std::fs::remove_file(&state.config_path).ok();
    }

    #[test]
    fn rename_rule_rejects_name_collision() {
        let mut state = scratch_state(&["a", "b"]);
        assert!(state.rename_rule("a", "b").is_err());
        // Unchanged on failure.
        assert!(state.config.rules.iter().any(|r| r.name == "a"));
        std::fs::remove_file(&state.config_path).ok();
    }

    #[test]
    fn rename_rule_same_name_is_a_noop_success() {
        let mut state = scratch_state(&["a"]);
        assert!(state.rename_rule("a", "a").is_ok());
        std::fs::remove_file(&state.config_path).ok();
    }
}
