# plasma-keepawake

Rule-driven "keep the system awake" tool for KDE Plasma, developed with the
help of AI assistance. Instead of a single manual caffeine toggle, you define
rules — expressions over things like "is media playing," "is a process
running," "is a custom signal set" — and a background daemon holds a real
systemd-logind sleep inhibitor for as long as any enabled rule is true — the
same mechanism `systemd-inhibit` and media players like cliamp itself use.

Motivating cases (the two it needs to handle on day one):

- Don't sleep while **cliamp** (a music player) is actively playing (via MPRIS).
- Don't sleep while **Claude Code** is actively working, signaled by a Claude
  Code hook touching a flag file.

The rule model is intentionally general — those two are just the first two
`expr` strings in the config, not special-cased in the daemon.

## Architecture

```
                  ┌──────────────────────────────┐
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
                  └──────────────────────────────┘
                                 ▲
                                 │ D-Bus
                                 ▼
                  ┌──────────────────────────────┐
                  │  Plasma widget (QML plasmoid)│
                  │  status icon + rule list +   │
                  │  enable/disable + raw expr   │
                  │  editor                      │
                  └──────────────────────────────┘
```

Two components, two toolchains, one repo:

- `daemon/` — Rust. Owns rule evaluation, the config file, and the actual
  inhibition. Runs as a `systemd --user` service. Source of truth.
- `widget/` — QML plasmoid for Plasma 6 (KF6). Thin client: shows status,
  toggles rules on/off, edits rule expressions. Talks to the daemon over
  D-Bus, holds no state of its own.

## Installing

**Dependencies:**
- An Arch-based Linux distro (uses `pacman`/`makepkg`) running KDE Plasma 6.
- Build-time only, not needed afterward: a Rust toolchain (`cargo`) and
  `base-devel` (for `makepkg`).
- Runtime: `systemd` and `plasma-workspace` — both pulled in automatically
  as package dependencies.
- Optional, for widget development only: `plasma-sdk` (provides
  `plasmoidviewer`, an isolated preview window — see "Status" below).

```sh
git clone https://github.com/ekoaw/plasma-keepawake.git
cd plasma-keepawake
./packaging/install.sh
```

This builds the package, installs it (`sudo pacman -U`, will prompt for
your password), writes a default config to
`~/.config/plasma-keepawake/config.json` **only if one doesn't already
exist** (never overwrites your rules), and enables + (re)starts the
`plasma-keepawaked` service. It then asks — doesn't assume — whether to
restart `plasmashell` so the widget shows up in the widget picker, since
that's a visible flicker across your whole desktop. Safe to re-run later
(e.g. after pulling an update): it rebuilds, reinstalls, and restarts the
daemon so a fix actually takes effect, still without touching your config.

Then: right-click your panel → **Add Widgets…** → search "Plasma
Keepawake" → drag it onto the panel.

**Claude Code integration** is a separate, optional manual step (it edits
`~/.claude/settings.json`, outside anything the package touches) — see
"Claude Code integration" in [`PLAN.md`](PLAN.md) for the exact hook
config.

<details>
<summary>Manual install (if you'd rather not run a script)</summary>

```sh
cd packaging
makepkg --nosign
sudo pacman -U plasma-keepawake-*.pkg.tar.zst

mkdir -p ~/.config/plasma-keepawake
cp ../daemon/examples/config.json ~/.config/plasma-keepawake/config.json   # or write your own

systemctl --user daemon-reload
systemctl --user enable --now plasma-keepawaked.service
```

Then add the widget as above; if it doesn't show up, `systemctl --user
restart plasma-plasmashell.service`.
</details>

**Uninstalling:**
```sh
systemctl --user disable --now plasma-keepawaked.service
sudo pacman -R plasma-keepawake
```
`pacman -R` doesn't touch `~/.config/plasma-keepawake/` or
`~/.local/state/plasma-keepawake/` — remove those by hand if you want your
config/signal files gone too.

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
| `signal(name)` | `$XDG_STATE_HOME/plasma-keepawake/signals/<name>` (a flag file) **or** `signals/<name>.d/` (a directory) | true if the flag file exists, or the `.d` directory has ≥1 file in it — see below |

`signal()` is how Claude Code integration works, and it's why there are
two forms. A single flag file is enough for one producer (any script or
hook can assert/clear a condition by touching/removing a file). But two
*concurrent* Claude Code sessions sharing one file is a real bug: whichever
session finishes first removes the shared file, even if another session is
still working. The `.d` directory form fixes that — each producer instance
writes its own uniquely-named file (Claude Code's hooks use the session's
`session_id`, delivered on the hook's stdin as JSON, not an env var) and
removes only that one; the signal stays true as long as *any* file remains
in the directory. See `PLAN.md`'s "Claude Code integration" section for
the concrete hook config (needs `jq`) and how this was verified.

New primitives (e.g. "is a given window focused") are added as new Rust
functions registered with the Rhai engine — not a plugin/loadable-module
system. See `PLAN.md` for why that tradeoff was chosen over a real
plugin ABI.

## Status

All 8 planned milestones are done, and it's actually installed and
running on this machine per the steps above, not just built — daemon
active as a service holding a real inhibitor, widget on the real panel,
Claude Code hooks wired in, all verified against live state rather than
assumed. The widget itself is a status icon + rule list with toggles,
add/edit/remove rules, and a reload button, all reading and writing the
daemon's live state (see "Architecture" above for why it shells out to
`busctl --json=short` rather than using a D-Bus-from-QML binding — there
isn't one in Plasma 6 for a pure-QML plasmoid).

For development, both pieces are testable in isolation without touching
your real panel or a real config:

```sh
cd daemon
cargo run -- --check --config examples/config.json   # one-shot: print each rule's value
cargo run -- --run   --config examples/config.json   # persistent, own D-Bus service + inhibitor

sudo pacman -S --needed plasma-sdk   # provides plasmoidviewer, one-time
plasmoidviewer -a ../widget -f planar -s 420x450   # isolated preview window
```

The real find of this build-out: a production bug in the config
hot-reload path caused unbounded CPU growth after any rule edit (reload
reading the file generated its own filesystem event, which triggered
another reload, forever) — caught only once the daemon was actually
deployed and edited for real, not during any earlier scripted test. Fixed
and verified against the live service. See [`PLAN.md`](PLAN.md) for that
story in full, the rest of the milestone history, open decisions, and a
known limitation around an unclean Claude Code exit leaving the signal
flag stuck.

## License

GPL-3.0-or-later — see [`LICENSE`](LICENSE).
