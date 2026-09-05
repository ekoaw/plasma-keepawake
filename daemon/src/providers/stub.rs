//! Milestone-1 placeholder providers.
//!
//! Real providers (MPRIS, UPower, /proc, signal-file) land in
//! Milestone 2 as their own modules with D-Bus/inotify-backed caches. Until
//! then these read environment variables so `--check` can exercise the
//! config-loading and Rhai-evaluation pipeline end to end, e.g.:
//!
//! ```sh
//! STUB_MPRIS_PLAYING_CLIAMP=1 cargo run -- --check
//! ```
//!
//! No code outside this file should assume these are the real thing —
//! `register` here is what Milestone 2 replaces piece by piece.

use rhai::Engine;

fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1").unwrap_or(false)
}

fn shout(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect()
}

pub fn register(engine: &mut Engine) {
    engine.register_fn("mpris_playing", |name: &str| {
        env_flag(&format!("STUB_MPRIS_PLAYING_{}", shout(name)))
    });
    engine.register_fn("process_running", |pattern: &str| {
        env_flag(&format!("STUB_PROCESS_RUNNING_{}", shout(pattern)))
    });
    engine.register_fn("on_battery", || env_flag("STUB_ON_BATTERY"));
    engine.register_fn("on_ac", || !env_flag("STUB_ON_BATTERY"));
    engine.register_fn("signal", |name: &str| {
        env_flag(&format!("STUB_SIGNAL_{}", shout(name)))
    });
}
