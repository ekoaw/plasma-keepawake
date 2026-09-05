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

/// True iff either:
/// - a plain file named `name` exists in the signals directory (a single
///   producer asserts a condition by creating the file, clears it by
///   removing it), or
/// - a `<name>.d` directory exists and contains at least one file (*any*
///   producer among several currently asserts the condition).
///
/// The `.d` form exists for producers that can run as multiple concurrent,
/// independent instances - each writes its own uniquely-named file (e.g.
/// keyed by its own session/process id) instead of sharing one. Without
/// it, two concurrent producers racing on a single shared file means
/// whichever one finishes first clears the flag for the other that's still
/// active (this bit the Claude Code integration: two Claude Code sessions
/// share one `claude-thinking` signal, and the `Stop` hook of whichever
/// session ends first would clear it even if another session was still
/// working — see PLAN.md and README.md).
pub fn is_set(name: &str) -> bool {
    let dir = signals_dir();
    if dir.join(name).is_file() {
        return true;
    }
    match std::fs::read_dir(dir.join(format!("{name}.d"))) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => false,
    }
}
