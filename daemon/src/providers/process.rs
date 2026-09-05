use std::fs;

/// True iff any running process' command line contains `pattern`
/// (substring match, not a regex — keep it simple until a real case needs
/// more). Scans `/proc` fresh on every call; there's no kernel event for
/// "a process matching some pattern started," so this is polled by nature —
/// fine for `--check`, and Milestone 4's persistent loop will call this on
/// an interval rather than per rule evaluation.
///
/// Caveat when testing manually with `--check`: the substring match applies
/// to *every* process, including whatever shell/script invoked the check —
/// a pattern that happens to appear in that invocation's own command line
/// (e.g. embedded in a config path or inline script) will match on that,
/// not on the thing you meant to test for.
pub fn running(pattern: &str) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };

    for entry in entries.flatten() {
        let pid_name = entry.file_name();
        let Some(pid_str) = pid_name.to_str() else {
            continue;
        };
        if !pid_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        if let Ok(cmdline) = fs::read(entry.path().join("cmdline")) {
            let cmdline = cmdline
                .split(|&b| b == 0)
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            if !cmdline.is_empty() {
                if cmdline.contains(pattern) {
                    return true;
                }
                continue;
            }
        }

        // Kernel threads / very short-lived processes may have an empty
        // cmdline; fall back to comm (the bare executable name).
        if let Ok(comm) = fs::read_to_string(entry.path().join("comm"))
            && comm.trim().contains(pattern)
        {
            return true;
        }
    }

    false
}
