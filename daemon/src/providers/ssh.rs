use super::dbus::{self, Bus};

const LOGIND_DEST: &str = "org.freedesktop.login1";
const LOGIND_MANAGER_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER_IFACE: &str = "org.freedesktop.login1.Manager";
const LOGIND_SESSION_IFACE: &str = "org.freedesktop.login1.Session";

/// One row of `Manager.ListSessions`'s `a(susso)`: (session id, uid, user
/// name, seat id, session object path). Only the path is actually used —
/// the rest exist just to make the tuple's shape match the D-Bus signature
/// so it deserializes at all.
type SessionEntry = (
    String,
    u32,
    String,
    String,
    zbus::zvariant::OwnedObjectPath,
);

/// True iff `count() > 0`.
pub fn active() -> bool {
    count() > 0
}

/// `active_remote_hosts().len()`.
pub fn count() -> usize {
    active_remote_hosts().len()
}

/// `RemoteHost` (as logind reports it - typically the client's IP, exactly
/// what `who`/`w` would show) for every currently open SSH session, one
/// entry per session, in `ListSessions`' order - not deduplicated, so two
/// sessions from the same host show up twice and `.len()` still matches
/// `count()`. An entry is `""` if that session's `RemoteHost` is empty or
/// couldn't be read, again so the count stays right even then. Used for
/// the widget's SSH counter tooltip as well as `count()`/`active()`.
///
/// "SSH session" is identified by `Service == "sshd"` on the session's own
/// D-Bus object, the same field `loginctl session-status` shows. Confirmed
/// empirically on this machine (including with OpenSSH 10.5's newer split
/// `sshd`/`sshd-session` binaries): a real loopback SSH login still
/// registers `Service = "sshd"` via `pam_systemd.so` in `sshd`'s PAM stack
/// (`system-remote-login` → `system-login`), not the session's `Type`
/// (`tty`) or `Remote` (`true`) — those don't distinguish an SSH login
/// from other remote-but-not-SSH cases, so `Service` is the precise
/// signal. See PLAN.md for the verification.
///
/// Queried fresh via `Manager.ListSessions` each call — a D-Bus
/// round-trip per rule evaluation, same on-demand-not-cached policy as
/// every other provider (see `dbus.rs`'s module doc) — rather than
/// polling `who`/`ss`/`/proc` or holding a signal-fed cache.
pub fn active_remote_hosts() -> Vec<String> {
    let sessions: Vec<SessionEntry> = match dbus::call_method(
        Bus::System,
        LOGIND_DEST,
        LOGIND_MANAGER_PATH,
        LOGIND_MANAGER_IFACE,
        "ListSessions",
    ) {
        Some(s) => s,
        None => return Vec::new(), // logind unreachable - no sessions is the safe default, same as other providers
    };

    let entries: Vec<(Option<String>, Option<String>)> = sessions
        .iter()
        .map(|(_, _, _, _, path)| {
            let service = dbus::get_property::<String>(
                Bus::System,
                LOGIND_DEST,
                path.as_str(),
                LOGIND_SESSION_IFACE,
                "Service",
            );
            let remote_host = dbus::get_property::<String>(
                Bus::System,
                LOGIND_DEST,
                path.as_str(),
                LOGIND_SESSION_IFACE,
                "RemoteHost",
            );
            (service, remote_host)
        })
        .collect();

    ssh_hosts(&entries)
}

/// Pure filtering logic, split out from the D-Bus fetch above so it can be
/// exercised directly with synthetic session data instead of a real
/// logind - a live 0/1/2-concurrent-session transition isn't practical to
/// script in a unit test the way the fetch's *result* is.
fn ssh_hosts(entries: &[(Option<String>, Option<String>)]) -> Vec<String> {
    entries
        .iter()
        .filter(|(service, _)| service.as_deref() == Some("sshd"))
        .map(|(_, host)| host.clone().unwrap_or_default())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(service: &str, host: &str) -> (Option<String>, Option<String>) {
        (Some(service.to_string()), Some(host.to_string()))
    }

    #[test]
    fn counts_and_lists_only_sshd_sessions_across_transitions() {
        // No sessions at all.
        assert_eq!(ssh_hosts(&[]), Vec::<String>::new());

        // 0 -> 1: an unrelated session doesn't count; adding an sshd one does.
        assert_eq!(ssh_hosts(&[session("plasmalogin", "")]), Vec::<String>::new());
        assert_eq!(
            ssh_hosts(&[session("plasmalogin", ""), session("sshd", "10.0.0.5")]),
            vec!["10.0.0.5"]
        );

        // 1 -> 2: a second concurrent SSH session (from a different host).
        assert_eq!(
            ssh_hosts(&[session("sshd", "10.0.0.5"), session("sshd", "10.0.0.9")]),
            vec!["10.0.0.5", "10.0.0.9"]
        );

        // 2 -> 1: one of the two disconnects.
        assert_eq!(ssh_hosts(&[session("sshd", "10.0.0.9")]), vec!["10.0.0.9"]);

        // 1 -> 0: the last one disconnects.
        assert_eq!(ssh_hosts(&[]), Vec::<String>::new());

        // A session whose Service couldn't be read is never counted as
        // SSH; one whose RemoteHost couldn't be read still counts, as "".
        assert_eq!(ssh_hosts(&[(None, Some("10.0.0.5".into()))]), Vec::<String>::new());
        assert_eq!(ssh_hosts(&[(Some("sshd".into()), None)]), vec![""]);
    }
}
