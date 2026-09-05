use std::path::PathBuf;

/// `$XDG_STATE_HOME/plasma-keepawake/signals/`, falling back to
/// `~/.local/state/plasma-keepawake/signals/`.
fn signals_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("plasma-keepawake").join("signals")
}

/// True iff a file named `name` exists in the signals directory. Any script
/// or hook (e.g. a Claude Code hook) asserts a condition by creating the
/// file and clears it by removing it — see README.md.
pub fn is_set(name: &str) -> bool {
    signals_dir().join(name).is_file()
}
