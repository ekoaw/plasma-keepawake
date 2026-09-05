use std::sync::{Arc, Mutex};

use crate::state::DaemonState;

pub const BUS_NAME: &str = "org.plasmakeepawake.Daemon1";
pub const OBJECT_PATH: &str = "/org/plasmakeepawake/Daemon1";

pub struct DaemonIface {
    pub state: Arc<Mutex<DaemonState>>,
}

#[zbus::interface(name = "org.plasmakeepawake.Daemon1")]
impl DaemonIface {
    /// `(name, enabled, currently_true, last_error)` per rule; error is ""
    /// when the rule last evaluated cleanly.
    #[zbus(property)]
    fn rules(&self) -> Vec<(String, bool, bool, String)> {
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
                (r.name.clone(), r.enabled, value, err)
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
}
