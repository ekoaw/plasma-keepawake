use super::dbus::{self, Bus};

/// True iff UPower reports `OnBattery`. If UPower isn't running (common on
/// desktops with no battery) this defaults to `false` (i.e. "assume AC") —
/// picked so `on_battery()`-gated rules don't silently fire everywhere on a
/// machine with no battery, and `on_ac()` stays the safer default to build
/// other rules on.
pub fn on_battery() -> bool {
    dbus::get_property::<bool>(
        Bus::System,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
        "OnBattery",
    )
    .unwrap_or(false)
}
