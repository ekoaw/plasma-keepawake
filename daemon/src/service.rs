use std::sync::{Arc, Mutex};

use crate::state::DaemonState;

pub const BUS_NAME: &str = "org.plasmakeepawake.Daemon1";
pub const OBJECT_PATH: &str = "/org/plasmakeepawake/Daemon1";

pub struct DaemonIface {
    pub state: Arc<Mutex<DaemonState>>,
}

#[zbus::interface(name = "org.plasmakeepawake.Daemon1")]
impl DaemonIface {
    /// `(name, enabled, currently_true, last_error, expr)` per rule; error
    /// is "" when the rule last evaluated cleanly. `expr` is included so a
    /// UI can pre-fill an edit field with the current expression.
    #[zbus(property)]
    fn rules(&self) -> Vec<(String, bool, bool, String, String)> {
        let state = self.state.lock().unwrap();
        state
            .rule_engine
            .rules()
            .iter()
            .map(|r| {
                let (value, err) = match &r.value {
                    Ok(v) => (*v, String::new()),
                    Err(e) => (false, e.clone()),
                };
                (r.name.clone(), r.enabled, value, err, r.expr.clone())
            })
            .collect()
    }

    #[zbus(property)]
    fn inhibiting(&self) -> bool {
        self.state.lock().unwrap().inhibiting
    }

    #[zbus(property)]
    fn reason(&self) -> String {
        self.state.lock().unwrap().reason.clone()
    }

    #[zbus(property)]
    fn reload_error(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .reload_error
            .clone()
            .unwrap_or_default()
    }

    /// Returns `false` if no rule with that name exists.
    fn set_rule_enabled(&self, name: &str, enabled: bool) -> bool {
        self.state
            .lock()
            .unwrap()
            .rule_engine
            .set_enabled(name, enabled)
    }

    /// Reloads the config file from disk. Check `ReloadError` afterwards —
    /// this always succeeds as a D-Bus call even if the reload itself
    /// failed, so a bad config surfaces as a property rather than a
    /// dropped connection.
    fn reload_config(&self) {
        self.state.lock().unwrap().reload();
    }

    /// Adds a new rule and persists it to the config file. `(success,
    /// error)` - error is "" on success, e.g. "a rule named ... already
    /// exists" or a Rhai compile error otherwise.
    fn add_rule(&self, name: &str, expr: &str, enabled: bool) -> (bool, String) {
        match self
            .state
            .lock()
            .unwrap()
            .add_rule(name.to_string(), expr.to_string(), enabled)
        {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e),
        }
    }

    /// Changes an existing rule's `expr` and persists it. Does not touch
    /// `enabled` - use `SetRuleEnabled` for that.
    fn update_rule(&self, name: &str, expr: &str) -> (bool, String) {
        match self
            .state
            .lock()
            .unwrap()
            .update_rule(name, expr.to_string())
        {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e),
        }
    }

    /// Renames a rule (keeping its expr/enabled/live value) and persists
    /// the rename. `(success, error)`.
    fn rename_rule(&self, old_name: &str, new_name: &str) -> (bool, String) {
        match self
            .state
            .lock()
            .unwrap()
            .rename_rule(old_name, new_name)
        {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e),
        }
    }

    /// Removes a rule and persists the removal. `(success, error)`.
    fn remove_rule(&self, name: &str) -> (bool, String) {
        match self.state.lock().unwrap().remove_rule(name) {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e),
        }
    }
}
