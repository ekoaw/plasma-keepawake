use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a `.d` entry is trusted without being re-touched. Chosen to
/// comfortably outlast the longest realistic gap between two `PreToolUse`
/// hook firings in a genuinely active Claude Code session (a single long
/// tool call or a big non-tool response), while still being short enough
/// that a crashed session's stuck flag self-heals in a bounded time rather
/// than staying stuck forever - see `clear_stale` and PLAN.md.
const MAX_AGE: Duration = Duration::from_secs(600);

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

/// True iff its mtime is less than `MAX_AGE` old. Unreadable metadata (e.g.
/// a race with another process removing the file) counts as stale.
fn is_fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < MAX_AGE)
}

/// True iff either:
/// - a plain file named `name` exists in the signals directory (a single
///   producer asserts a condition by creating the file, clears it by
///   removing it - no staleness check, since a one-shot producer has no
///   reason to keep re-touching it), or
/// - a `<name>.d` directory exists and contains at least one *fresh* file
///   (*any* producer among several currently asserts the condition).
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
///
/// The freshness requirement on `.d` entries exists for the flip side of
/// that same scenario: a producer that crashes (or is killed) instead of
/// exiting cleanly never runs its own cleanup, and would otherwise leave
/// that entry - and therefore the signal, and therefore the inhibitor -
/// stuck forever. `.d` producers are expected to keep re-touching their
/// own file while they're still active (Claude Code's `PreToolUse` hook
/// already does, once per tool call); an entry that's stopped being
/// touched is treated as abandoned once `MAX_AGE` passes, so the signal
/// self-heals within a bounded time instead of needing a manual cleanup.
pub fn is_set(name: &str) -> bool {
    let dir = signals_dir();
    if dir.join(name).is_file() {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir.join(format!("{name}.d"))) else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .any(|p| p.is_file() && is_fresh(&p))
}

/// Eagerly removes every stale (untouched for `MAX_AGE`) file across every
/// `<name>.d/` signal directory - the immediate, on-demand counterpart to
/// the staleness check `is_set` already applies lazily on each evaluation.
/// Never removes a fresh entry, so it can't clear a signal some producer
/// is still actively asserting. Returns how many files were removed.
pub fn clear_stale() -> usize {
    let Ok(top) = std::fs::read_dir(signals_dir()) else {
        return 0;
    };

    let mut removed = 0;
    for signal_dir in top
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.extension().is_some_and(|ext| ext == "d"))
    {
        let Ok(entries) = std::fs::read_dir(&signal_dir) else {
            continue;
        };
        for path in entries.filter_map(Result::ok).map(|e| e.path()) {
            if path.is_file() && !is_fresh(&path) && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn age(path: &Path, seconds_ago: u64) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(seconds_ago))
            .unwrap();
    }

    // Everything lives in one test, not one-per-assertion: `signals_dir`
    // reads the process-wide XDG_STATE_HOME env var, and cargo runs tests
    // in parallel by default, so two tests each pointing it somewhere
    // different would race.
    #[test]
    fn dot_d_staleness_and_clear_stale() {
        let scratch = std::env::temp_dir().join(format!(
            "plasma-keepawake-signal-test-{}",
            std::process::id()
        ));
        // SAFETY: no other test in this binary reads or writes
        // XDG_STATE_HOME, so there's no concurrent access to race with.
        unsafe {
            std::env::set_var("XDG_STATE_HOME", &scratch);
        }

        let dot_d = signals_dir().join("thinking.d");
        std::fs::create_dir_all(&dot_d).unwrap();
        let fresh = dot_d.join("live-session");
        let stale = dot_d.join("crashed-session");
        std::fs::write(&fresh, "").unwrap();
        std::fs::write(&stale, "").unwrap();
        age(&stale, MAX_AGE.as_secs() + 60);

        assert!(
            is_set("thinking"),
            "a fresh entry alongside a stale one should still count as set"
        );

        std::fs::remove_file(&fresh).unwrap();
        assert!(
            !is_set("thinking"),
            "once only a stale entry is left, the signal should self-clear with no manual cleanup"
        );

        std::fs::write(&fresh, "").unwrap();
        let removed = clear_stale();
        assert_eq!(removed, 1, "clear_stale should remove only the stale entry");
        assert!(fresh.exists(), "clear_stale must not touch a fresh entry");
        assert!(!stale.exists());

        std::fs::remove_dir_all(&scratch).ok();
    }
}
