mod stub;

use rhai::Engine;

/// Registers every condition-primitive function into the given Rhai engine.
/// Backed by [`stub`] until Milestone 2 replaces each function with a
/// real D-Bus/inotify-backed provider.
pub fn register_all(engine: &mut Engine) {
    stub::register(engine);
}
