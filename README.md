# plasma-keepawake

Rule-driven "keep the system awake" tool for KDE Plasma. Instead of a single
manual caffeine toggle, you define rules — expressions over things like
"is media playing," "is a process running," "is a custom signal set" — and a
background daemon holds a real systemd-logind sleep inhibitor for as long
as any enabled rule is true — the same mechanism `systemd-inhibit` and
media players like cliamp itself use.

Motivating cases (the two it needs to handle on day one):

- Don't sleep while **cliamp** (a music player) is actively playing (via MPRIS).
- Don't sleep while **Claude Code** is actively working, signaled by a Claude
  Code hook touching a flag file.

The rule model is intentionally general — those two are just the first two
`expr` strings in the config, not special-cased in the daemon.

## Architecture

```
                 ┌─────────────────────────────┐
                 │   plasma-keepawaked (Rust)   │
                 │                              │
  MPRIS ────────▶│  providers: mpris, process,  │──▶ org.freedesktop.
  UPower ───────▶│  battery, signal-file        │    login1.Manager
  /proc ────────▶│                              │    .Inhibit()
  signal dir ───▶│  rule engine: rhai exprs     │    (holds the returned
  (Claude Code    │  over provider functions     │     fd; drop to release)
   hooks, etc.)   │                              │
                 │  own D-Bus service:          │
                 │  org.plasmakeepawake.Daemon1 │◀── queried/controlled by
                 └─────────────────────────────┘
                              ▲
                              │ D-Bus
                              ▼
                 ┌─────────────────────────────┐
                 │  Plasma widget (QML plasmoid)│
                 │  status icon + rule list +   │
                 │  enable/disable + raw expr   │
                 │  editor                      │
                 └─────────────────────────────┘
```

Two components, two toolchains, one repo:

- `daemon/` — Rust. Owns rule evaluation, the config file, and the actual
  inhibition. Runs as a `systemd --user` service. Source of truth.
- `widget/` — QML plasmoid for Plasma 6 (KF6). Thin client: shows status,
  toggles rules on/off, edits rule expressions. Talks to the daemon over
  D-Bus, holds no state of its own.

## Rule model

A rule is a name plus a boolean expression written in
[Rhai](https://rhai.rs/) (a small, sandboxed, Rust-native scripting
language — no file/network/process access unless the daemon explicitly
exposes it). The daemon registers one function per condition primitive; rule
expressions combine them with normal `&&` / `||` / `!`.

```json
{
  "version": 1,
  "rules": [
    {
      "name": "cliamp-playing",
      "enabled": true,
      "expr": "mpris_playing(\"cliamp\")"
    },
    {
      "name": "claude-code-active",
      "enabled": true,
      "expr": "signal(\"claude-thinking\")"
    },
    {
      "name": "media-but-not-on-battery",
      "enabled": false,
      "expr": "mpris_playing(\"cliamp\") && !on_battery()"
    }
  ]
}
```

While any **enabled** rule evaluates `true`, the daemon holds one
`org.freedesktop.login1.Manager.Inhibit()` lock (the same D-Bus call
`systemd-inhibit` and cliamp itself use — confirmed by checking
`systemd-inhibit --list`, see `PLAN.md`), with a reason string naming which
rule(s) are currently active. The moment none are true, it releases the
lock.

### Built-in provider functions (v1)

| Function | Backed by | Notes |
|---|---|---|
| `mpris_playing(name)` | MPRIS (`org.mpris.MediaPlayer2.<name>`) | true iff `PlaybackStatus == Playing` |
| `process_running(pattern)` | `/proc` scan | polled, no kernel event exists for arbitrary process start |
| `on_battery()` / `on_ac()` | UPower (system bus) | queried on demand; `false`/AC assumed if UPower isn't running |
| `signal(name)` | a flag file under `$XDG_STATE_HOME/plasma-keepawake/signals/<name>` | generic escape hatch — any script or hook can assert/clear a condition by touching/removing a file |

`signal()` is how Claude Code integration works: a Claude Code hook (e.g.
`PreToolUse` touches the file, `Stop` removes it) is just one producer of a
signal, with no Claude-specific code in the daemon. See `PLAN.md` for the
concrete hook config.

New primitives (e.g. "is a given window focused") are added as new Rust
functions registered with the Rhai engine — not a plugin/loadable-module
system. See `PLAN.md` for why that tradeoff was chosen over a real
plugin ABI.

## Status

The daemon (`daemon/`) is real and self-testable:

```sh
cd daemon
cargo run -- --check --config examples/config.json   # one-shot: print each rule's value
cargo run -- --run   --config examples/config.json   # persistent: holds/releases the real
                                                       # sleep inhibitor, serves D-Bus
```

While `--run` is active it holds a genuine `systemd-logind` sleep inhibitor
(shows up in `systemd-inhibit --list`) whenever an enabled rule is true, and
serves `org.plasmakeepawake.Daemon1` on the session bus — `busctl --user
introspect org.plasmakeepawake.Daemon1 /org/plasmakeepawake/Daemon1` to
poke at it directly. Editing the config file on disk hot-reloads it.

The Claude Code hooks (`PreToolUse`/`Stop`, touching the signal file) are
installed in this machine's own `~/.claude/settings.json` and verified
against a running daemon.

The widget (`widget/`) is a plain QML KPackage plasmoid — status icon,
rule list with enable/disable toggles, add/edit/remove rules, reload
button — talking to the daemon purely by shelling out to `busctl
--json=short` (there's no generic D-Bus-from-QML binding in Plasma 6; see
PLAN.md). Try it in an isolated preview window without touching your real
panel:

```sh
sudo pacman -S --needed plasma-sdk   # provides plasmoidviewer, one-time
plasmoidviewer -a widget -f planar -s 420x450
```

Not yet built: packaging/install (a systemd unit exists in `packaging/`
but isn't installed anywhere by default, and the widget isn't on your real
panel — both deliberately held back as separate steps). See
[`PLAN.md`](PLAN.md) for the full milestone list, open decisions, and a
known limitation around an unclean Claude Code exit leaving the signal
flag stuck.
