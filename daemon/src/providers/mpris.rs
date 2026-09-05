use super::dbus::{self, Bus};

/// True iff `org.mpris.MediaPlayer2.<name>` exists and reports
/// `PlaybackStatus == "Playing"`. A player that isn't running (no such
/// D-Bus name) or doesn't answer is treated as "not playing", not an error —
/// most media players only own their MPRIS name while open.
pub fn playing(name: &str) -> bool {
    let status: Option<String> = dbus::get_property(
        Bus::Session,
        &format!("org.mpris.MediaPlayer2.{name}"),
        "/org/mpris/MediaPlayer2",
        "org.mpris.MediaPlayer2.Player",
        "PlaybackStatus",
    );
    status.as_deref() == Some("Playing")
}
