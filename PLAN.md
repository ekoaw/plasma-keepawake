# Implementation plan

## Goals

- General rule engine: rules are boolean expressions over pluggable
  condition primitives, not hardcoded per-app logic.
- Ship two concrete rules on day one: cliamp playing (MPRIS), Claude Code
  active (hook-driven signal file).
- Daemon is the single source of truth and the single writer of its own
  config; the widget is a thin D-Bus client.
- Inhibition goes through `org.freedesktop.login1.Manager.Inhibit()`
  (systemd-logind) — the same mechanism the `systemd-inhibit` CLI wraps, and
  empirically confirmed to be what real apps on this machine actually use
  (see "Inhibition mechanism" below for how that was confirmed and what it
  replaced).

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
  packaging/          # systemd unit, eventually a PKGBUILD
  README.md
  PLAN.md
```

## Inhibition mechanism (how milestone 3 landed here)

The original plan (above, and in the first cut of this doc) was to call
`org.kde.Solid.PowerManagement.PolicyAgent.AddInhibition(types, app_name,
reason) -> cookie`, on the theory that it's "the interface Plasma's own
power settings page reads from." That theory didn't survive contact with
the real system:

- No `-dev` package on this machine ships the header defining the `types`
  bitmask (`RequiredPolicies`), so the value to pass was already a guess.
- Probing `AddInhibition` with candidate values (1, 2, 4, 8) via `busctl`
  produced real cookies and `HasInhibition` returned `true`, but **none of
  it showed up** in `RequestedInhibitions`/`ActiveInhibitions`, nor in
  Plasma's battery/power applet popup — the one place a user could visually
  confirm "yes, something is holding sleep off."
- Ground truth came from `systemd-inhibit --list`, which showed cliamp's
  *actual* inhibitor as a `systemd-inhibit`-wrapped subprocess — i.e. cliamp
  itself uses systemd-logind's inhibitor mechanism, not PolicyAgent. That
  also explains why `RequestedInhibitions`/the battery popup kept showing
  cliamp's entry unchanged no matter what was probed on PolicyAgent: those
  UI surfaces most likely just mirror logind's inhibitor list rather than
  reflecting `AddInhibition` calls at all.

Conclusion: `org.freedesktop.login1.Manager.Inhibit()` is the real,
verified mechanism, confirmed two ways — it's what cliamp already uses, and
after wiring it up (`daemon/src/inhibitor.rs`) `plasma-keepawaked` itself
showed up correctly in `systemd-inhibit --list` (`WHO=plasma-keepawaked
... WHAT=sleep ... MODE=block`) while a test rule was true, and disappeared
the moment it went false. No PolicyAgent bitmask to guess, and no cookie
bookkeeping either — `Inhibit()` returns a file descriptor; holding it open
*is* the inhibition, dropping it releases the lock, and an unclean daemon
exit can't leak an inhibitor since the kernel closes the fd when the
process dies.

```
org.freedesktop.login1.Manager (system bus, /org/freedesktop/login1)
  Inhibit(what: "sleep", who: s, why: s, mode: "block") -> h (fd)
```

## Other confirmed D-Bus surfaces (introspected on this machine, Plasma 6.7)

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
- `inhibitor.rs` — holds an `Option<OwnedFd>` from `login1.Manager.Inhibit`;
  `reconcile(should_inhibit, reason)` acquires/releases on state transitions
  and reports whether a transition happened, so the caller only logs on
  change. Reason string is the comma-joined names of currently-true enabled
  rules.
- `state.rs` — `DaemonState { config_path, rule_engine, inhibiting, reason,
  reload_error }`, the one thing behind the shared lock. `reload()` rebuilds
  `rule_engine` from disk wholesale and only touches `reload_error` on
  failure — the last known-good `rule_engine` is left in place, per the
  config-schema section above.
- `service.rs` — the daemon's own D-Bus interface,
  `org.plasmakeepawake.Daemon1` at `/org/plasmakeepawake/Daemon1`, wrapping
  `Arc<Mutex<DaemonState>>`:
  - property `Rules: a(sbbs)` — `(name, enabled, currently_true,
    last_error)` per rule, last_error `""` when clean.
  - properties `Inhibiting: b`, `Reason: s`, `ReloadError: s`.
  - methods `SetRuleEnabled(name: s, enabled: b) -> b` (false if no such
    rule) and `ReloadConfig()`. `AddRule`/`UpdateRule`/`RemoveRule` are
    still just planned, for when the widget needs to write rules rather
    than only toggle/reload them.
- `main.rs` — `--check` (one-shot) and `--run`: builds `DaemonState`,
  starts the D-Bus service, watches the config file's parent directory
  with `notify` and reloads on any event touching the exact path (watching
  the directory rather than the file survives an editor's save-by-rename,
  which would silently orphan a direct file watch), then polls every 2s to
  re-evaluate, update `inhibiting`/`reason`, and reconcile the logind
  inhibitor. The D-Bus dispatch thread and the poll loop only ever touch
  shared state through the one `Mutex<DaemonState>` lock, held briefly
  (never across a D-Bus/proc/fs provider call or the blocking `Inhibit()`
  call itself).

Crates in use: `zbus` (blocking API — no async runtime needed for
on-demand queries or for serving the interface), `rhai` (with the `sync`
feature enabled so `Engine`/`AST` are `Send + Sync` and can live behind
`Arc<Mutex<DaemonState>>`), `serde`/`serde_json`, `notify` (inotify, config
hot-reload). No `tokio` — the blocking zbus APIs cover both the client
queries and serving `Daemon1`, so there was never a point where an async
runtime paid for itself. `process_running` ended up as a ~30-line manual
`/proc` scan; `sysinfo` wasn't needed.

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
3. ✅ **Inhibition** — landed on `login1.Manager.Inhibit()` instead of
   `PolicyAgent.AddInhibition` (see "Inhibition mechanism" above for why),
   wired into a `--run` poll loop, verified against `systemd-inhibit --list`
   showing `plasma-keepawaked` appear/disappear in step with a test rule.
4. ✅ **Daemon D-Bus service + systemd unit** — `org.plasmakeepawake.Daemon1`
   live and verified via `busctl` (introspection, `Rules`/`Inhibiting`
   properties, `SetRuleEnabled` overriding a true rule back to
   not-inhibiting, hot-reload on file edit, last-good-config-kept +
   `ReloadError` surfaced on invalid JSON). `packaging/plasma-keepawaked.
   service` written; **not yet installed/enabled** on this machine — that's
   a separate, deliberate step (see note below), not implied by writing the
   unit file.
5. **Claude Code hook wiring** — add the hook config, confirm the
   `claude-code-active` rule actually tracks Claude Code activity in
   practice.
6. **Plasma widget v1** — status + toggle, read-only expr display.
7. **Widget rule editing** — `AddRule`/`UpdateRule`/`RemoveRule` on the
   daemon, text-field editor in the widget.
8. **Packaging** — `PKGBUILD` for the daemon binary + systemd unit,
   `kpackagetool6`-installable plasmoid, decide license (open decision
   below).

Note on the unit file: writing `packaging/plasma-keepawaked.service` is
just putting the file in the repo. `systemctl --user enable --now` makes
the daemon start on every future login and starts it immediately on this
machine — a persistent, visible change to the session — so that's held
back as a separate step until it's actually wanted, not bundled into
"wrote the packaging file."

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
