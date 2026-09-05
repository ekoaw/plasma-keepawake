mod dbus;
mod mpris;
mod power;
mod process;
mod signal;

use rhai::Engine;

/// Registers every condition-primitive function into the given Rhai engine.
///
/// These query D-Bus/`/proc`/the filesystem fresh on every call. Milestone 4
/// (the persistent daemon loop) is expected to swap these for cache-backed
/// versions kept live by D-Bus signals / inotify rather than re-querying on
/// every evaluation — the function names and behavior stay the same either
/// way.
pub fn register_all(engine: &mut Engine) {
    engine.register_fn("mpris_playing", mpris::playing);
    engine.register_fn("on_battery", power::on_battery);
    engine.register_fn("on_ac", || !power::on_battery());
    engine.register_fn("process_running", process::running);
    engine.register_fn("signal", signal::is_set);
}
