//! Sleep inhibition via systemd-logind, the mechanism confirmed to actually
//! work on this machine (`systemd-inhibit --list` showing cliamp's own
//! inhibitor was the ground truth that ruled out
//! `org.kde.Solid.PowerManagement.PolicyAgent.AddInhibition` — see PLAN.md).
//!
//! `Inhibit()` hands back a file descriptor; holding it open *is* the
//! inhibition, and dropping it releases the lock. No cookie bookkeeping, and
//! no cleanup needed on an unclean exit — the kernel closes the fd (and so
//! releases the inhibitor) when the process dies, the same guarantee
//! `systemd-inhibit <command>` relies on.

use std::os::fd::OwnedFd;

use zbus::blocking::{Connection, Proxy};

pub struct Inhibitor {
    held: Option<OwnedFd>,
}

impl Inhibitor {
    pub fn new() -> Self {
        Inhibitor { held: None }
    }

    /// Acquires the inhibitor if `should_inhibit` and we don't hold one yet,
    /// or releases it if we hold one and `should_inhibit` is now false.
    /// Returns whether the held/released state actually changed, so the
    /// caller can log a transition instead of on every poll.
    pub fn reconcile(&mut self, should_inhibit: bool, reason: &str) -> bool {
        match (self.held.is_some(), should_inhibit) {
            (false, true) => {
                match acquire(reason) {
                    Some(fd) => {
                        self.held = Some(fd);
                        true
                    }
                    None => false, // logind unreachable; stay uninhibited rather than panic
                }
            }
            (true, false) => {
                self.held = None; // Drop closes the fd -> releases the inhibitor
                true
            }
            _ => false,
        }
    }
}

fn acquire(reason: &str) -> Option<OwnedFd> {
    let conn = Connection::system().ok()?;
    let proxy = Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .ok()?;
    let fd: zbus::zvariant::OwnedFd = proxy
        .call("Inhibit", &("sleep", "plasma-keepawaked", reason, "block"))
        .ok()?;
    Some(fd.into())
}
