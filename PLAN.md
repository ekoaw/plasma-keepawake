# Implementation plan

## Goals

- General rule engine: rules are boolean expressions over pluggable
  condition primitives, not hardcoded per-app logic.
- Ship two concrete rules on day one: cliamp playing (MPRIS), Claude Code
  active (hook-driven signal file).
- Daemon is the single source of truth and the single writer of its own
  config; the widget is a thin D-Bus client.
- Inhibition goes through `org.kde.Solid.PowerManagement.PolicyAgent` (the
  interface Plasma's own power settings page reads), not a bare
  `systemd-inhibit` subprocess, so active inhibitions are visible/consistent
  with the rest of the desktop.

## Non-goals (for now)

- A real loadable-plugin system (dlopen `cdylib` or WASM modules). Rejected
  for v1: no stable Rust ABI without pinning the exact toolchain, and WASM
  (`wasmtime`) is a heavy dependency for a problem that a 10-line Rust
  function solves. Revisit only if third parties want to ship condition
  types independently of this repo.
- A boolean-tree GUI rule builder. Rejected in favor of Rhai expressions,
  which are strictly more expressive and cheaper to build (a text field +
  live-eval badge, not a tree-of-widgets editor).
- Multi-user / system-wide daemon. This is a per-user session tool
  (`systemd --user`), matching the per-user nature of Plasma sessions and
  MPRIS/UPower/PolicyAgent session buses.

## Repo layout

```
plasma-keepawake/
  daemon/            # Rust crate: plasma-keepawaked
  widget/             # QML plasmoid (KPackage layout for KF6)
  README.md
  PLAN.md
```

## Confirmed D-Bus surfaces (introspected on this machine, Plasma 6.7)

`org.kde.Solid.PowerManagement.PolicyAgent`
(`/org/kde/Solid/PowerManagement/PolicyAgent`):

```
AddInhibition(u types, s app_name, s reason) -> u cookie
ReleaseInhibition(u cookie)
HasInhibition(u types) -> b
property ActiveInhibitions: a(ssssu)
```

Open item: the exact bitmask values for `types` (`RequiredPolicies`, e.g.
"interrupt session" vs "change screen settings") aren't in an installed
header on this machine (no `-dev` package present) — confirm against KDE
Frameworks Solid source before wiring `AddInhibition`, and verify
empirically by checking that the screen doesn't blank / system doesn't
suspend while an inhibition with the chosen value is active, and that it
shows up in `ActiveInhibitions`.

`org.mpris.MediaPlayer2.<player>` — standard MPRIS2: `PlaybackStatus`
property on `org.mpris.MediaPlayer2.Player`, changes announced via
`org.freedesktop.DBus.Properties.PropertiesChanged`. Player services come
and go (only exist while the app runs), so also watch
`org.freedesktop.DBus.NameOwnerChanged` to notice start/stop rather than
assuming the name is always ownable.

`org.freedesktop.UPower` — **system bus** (not session bus, unlike MPRIS/
PolicyAgent), object path `/org/freedesktop/UPower`, `OnBattery` property,
`PropertiesChanged` signal for updates. Confirmed present and running on
this machine (`upower.service`, active) even though it's a desktop, not
just laptops.

## Config schema (v1)

```json
{
  "version": 1,
  "rules": [
    { "name": "string", "enabled": true, "expr": "rhai boolean expression" }
  ]
}
```

- `version` lets us migrate the schema later without guessing.
- The daemon is the sole writer. Hand-editing is supported now (inotify
  watch + reload), but once the widget can mutate rules it does so via a
  D-Bus method (`AddRule` / `UpdateRule` / `RemoveRule`), never by writing
  the file itself — avoids two writers racing on one file / torn reads.
- On a bad `expr` (Rhai compile error) or invalid JSON: keep the last
  known-good config active, don't drop to "no rules," and surface the error
  (exposed via the daemon's status D-Bus property so the widget can show it).

## Daemon crate layout

- `config.rs` — schema types (serde), `Config::load`. Hot-reload via
  `notify` is still pending (Milestone 4 territory, once there's a
  persistent process for it to run inside).
- `providers/` — implemented as **on-demand queries**, not the
  event-driven caches originally sketched here: each function does its
  D-Bus round-trip / `/proc` scan / file-existence check fresh on every
  call, and any failure (service not running, no such property) degrades
  to a safe default (`false` for playing/running/on_battery) instead of
  propagating an error. This is correct and simple, and cheap enough for
  `--check`; it's the wrong shape for a long-running daemon polling every
  few seconds forever, so Milestone 4 replaces the *implementation* of
  these same functions with signal/inotify-fed caches without changing
  their names or the rule language.
  - `dbus.rs` — shared `get_property::<T>(bus, dest, path, iface, prop)`
    helper over `zbus::blocking`, one cached `Connection` per bus
    (session/system), `None` on any failure.
  - `mpris.rs` — `playing(name)`, session bus.
  - `power.rs` — `on_battery()`, **system** bus (see above).
  - `process.rs` — `running(pattern)`, `/proc/<pid>/cmdline` substring
    match with a `comm` fallback for processes with empty cmdlines. Known
    caveat: matches against *every* process's full command line, including
    whatever invoked the check itself if the pattern happens to appear
    there (bit us once while testing with an inline shell pattern — not a
    bug, just how substring matching over cmdlines works).
  - `signal.rs` — `is_set(name)`, plain file-existence check under the
    signals dir; no inotify yet since on-demand queries don't need it.
- `engine.rs` — Rhai `Engine`, registers one function per provider, compiles
  each rule's `expr` to an `AST` once at construction, evaluates on demand.
  A per-rule compile or eval error is isolated to that rule (reported, not
  fatal) rather than failing the whole evaluation.
- `inhibitor.rs` (pending, Milestone 3) — wraps `AddInhibition` /
  `ReleaseInhibition`, holds the current cookie (if any), computes the
  reason string from currently-true rule names.
- `service.rs` (pending, Milestone 4) — the daemon's own D-Bus interface,
  `org.plasmakeepawake.Daemon1`:
  - property: per-rule `{name, enabled, currently_true, last_error}`
  - property: whether an inhibition is currently held, and why
  - methods: `SetRuleEnabled(name, bool)`, `ReloadConfig()`, and later
    `AddRule` / `UpdateRule` / `RemoveRule`
- `main.rs` — currently a synchronous `--check` CLI; grows a persistent
  event loop in Milestone 4 wiring providers → engine → inhibitor → service.

Crates in use: `zbus` (blocking API — no async runtime needed for
on-demand queries), `rhai`, `serde`/`serde_json`. Not yet added: `notify`
(inotify, for hot-reload and live signal-file watching) and `tokio` (only
needed once the daemon has a persistent event loop to run) — both deferred
to Milestone 4 rather than added ahead of the code that needs them.
`process_running` ended up as a ~30-line manual `/proc` scan; `sysinfo`
wasn't needed.

## Claude Code integration (concrete)

A `signal()` provider watches
`$XDG_STATE_HOME/plasma-keepawake/signals/` (falls back to
`~/.local/state/...`) for file presence = true. Claude Code hooks in
`~/.claude/settings.json` become the producer, no daemon changes needed:

```json
{
  "hooks": {
    "PreToolUse": [{ "hooks": [{ "type": "command",
      "command": "touch ~/.local/state/plasma-keepawake/signals/claude-thinking" }] }],
    "Stop": [{ "hooks": [{ "type": "command",
      "command": "rm -f ~/.local/state/plasma-keepawake/signals/claude-thinking" }] }]
  }
}
```

(Exact hook event names/payload to double check against the current Claude
Code hooks reference when this is actually wired up — the mechanism is
`signal()` + any command touching a file, so it also works for other tools
by adding more hook lines, not more daemon code.)

## Widget (plasmoid) — planned scope for v1

- Panel/systray icon reflecting current state (awake-forced vs idle-allowed).
- Popup: list of rules with enabled toggle and live true/false indicator
  (read from the daemon's D-Bus properties — no local evaluation).
- "Edit rule" — plain text field for `expr`, saved via the daemon's D-Bus
  method (once that exists) rather than writing JSON directly. Until that
  method exists, editing is done by hand in the config file and the widget
  is read/toggle-only.
- Built against KF6 (Plasma 6.7 here), KPackage layout, `kpackagetool6` for
  local install during development.

## Milestones

1. ✅ **Daemon skeleton** — config load, Rhai engine with env-var-backed
   stub providers, a `--check` CLI flag that loads a config and prints each
   rule's current truth value. No D-Bus yet. Got the rule language right
   before touching desktop integration.
2. ✅ **Real providers** — mpris, UPower, process, signal-file, implemented
   as on-demand queries (see crate layout above) rather than caches — that
   part of the original plan is deferred to milestone 4. Verified live
   against this machine: real cliamp MPRIS playback state, real UPower
   `OnBattery`, a hand-created signal file, and `/proc` scanning.
3. **Inhibition** — confirm `PolicyAgent` bitmask, wire `AddInhibition`/
   `ReleaseInhibition` on rule-truth transitions, verify against System
   Settings' power page and `ActiveInhibitions`.
4. **Daemon D-Bus service + systemd unit** — `org.plasmakeepawake.Daemon1`,
   `plasma-keepawaked.service` (`systemctl --user enable --now`).
5. **Claude Code hook wiring** — add the hook config, confirm the
   `claude-code-active` rule actually tracks Claude Code activity in
   practice.
6. **Plasma widget v1** — status + toggle, read-only expr display.
7. **Widget rule editing** — `AddRule`/`UpdateRule`/`RemoveRule` on the
   daemon, text-field editor in the widget.
8. **Packaging** — `PKGBUILD` for the daemon binary + systemd unit,
   `kpackagetool6`-installable plasmoid, decide license (open decision
   below).

## Open decisions

- License — not chosen yet.
- Whether `process_running` polling interval should be configurable
  per-rule or global (default to global, e.g. 5s, until there's a reason
  not to).
- Whether to add a `command(...)` provider (run an arbitrary shell command,
  exit code 0 = true) as a lower-effort general escape hatch alongside
  `signal()`. Leaning no for v1 — `signal()` covers the same use case with
  no per-evaluation process-spawn cost and no shell-injection surface from
  config content, but worth revisiting if a real case needs a live command
  result rather than a hook-toggled flag.
