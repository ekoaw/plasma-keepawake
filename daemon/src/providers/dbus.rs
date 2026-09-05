//! Small shared helpers for the D-Bus-backed providers.
//!
//! Milestone 2 queries D-Bus on demand (one round-trip per rule evaluation)
//! rather than keeping a live cache updated by signals — correct and simple,
//! but not what you'd want polling every few seconds forever. The event-driven
//! cache described in PLAN.md is Milestone 4's job, once there's a persistent
//! daemon loop for it to feed. Connections themselves *are* cached (opening a
//! session/system bus connection per call would be needlessly wasteful even
//! for on-demand queries).

use std::sync::OnceLock;
use zbus::blocking::Connection;

fn session() -> &'static Option<Connection> {
    static CONN: OnceLock<Option<Connection>> = OnceLock::new();
    CONN.get_or_init(|| Connection::session().ok())
}

fn system() -> &'static Option<Connection> {
    static CONN: OnceLock<Option<Connection>> = OnceLock::new();
    CONN.get_or_init(|| Connection::system().ok())
}

/// Fetches a single property from an arbitrary D-Bus object, returning
/// `None` on any failure (service not running, no such property, bus
/// unavailable, ...). Providers turn that into a safe default rather than
/// propagating an error into rule evaluation.
pub fn get_property<T>(
    bus: Bus,
    destination: &str,
    path: &str,
    interface: &str,
    property: &str,
) -> Option<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    let conn = match bus {
        Bus::Session => session(),
        Bus::System => system(),
    }
    .as_ref()?;

    let proxy = zbus::blocking::Proxy::new(conn, destination.to_string(), path.to_string(), interface.to_string())
        .ok()?;
    proxy.get_property::<T>(property).ok()
}

#[derive(Clone, Copy)]
pub enum Bus {
    Session,
    System,
}
