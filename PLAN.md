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

## Claude Code integration (milestone 5, done; revised post-milestone-8 for concurrent sessions)

A `signal()` provider watches `$XDG_STATE_HOME/plasma-keepawake/signals/`
(falls back to `~/.local/state/...`). Claude Code hooks in
`~/.claude/settings.json` are the producer, no daemon changes needed for
the basic mechanism. Note `~` doesn't expand inside a hook `command`
string — `$HOME` does, since hook commands run through a shell.

**Original design (milestone 5) used one shared flag file**
(`signals/claude-thinking`, `PreToolUse` touches it, `Stop` removes it).
That has a real bug with more than one concurrent Claude Code session:
whichever session's `Stop` fires first removes the *shared* flag, even if
another session is still actively working — `claude-code-active` goes
false mid-turn for the session that's still busy, because the flag can't
tell sessions apart. Caught by inspection (asked "what if two Claude Code
instances run at once"), not by observing it happen.

**Fix: per-session flags, a `.d` directory instead of one file.** Each
`PreToolUse` writes its own file, named by the session's `session_id`
(hook commands get this via JSON on **stdin**, not an environment
variable — confirmed against the hooks reference; there's no
`$CLAUDE_SESSION_ID`-style var). `signal(name)`'s implementation
(`daemon/src/providers/signal.rs`) now checks *two* things: a plain file
at `signals/<name>` (the original single-flag form, still supported —
e.g. for manual testing with a bare `touch`) **or** a `signals/<name>.d`
directory containing at least one file (true iff *any* producer among
several is currently asserting the condition). One session finishing only
removes its own file; the flag stays true as long as any other session's
file remains.

```json
{
  "hooks": {
    "PreToolUse": [
      { "hooks": [{ "type": "command",
        "command": "mkdir -p \"$HOME/.local/state/plasma-keepawake/signals/claude-thinking.d\" && SID=$(jq -r '.session_id') && touch \"$HOME/.local/state/plasma-keepawake/signals/claude-thinking.d/$SID\"" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command",
        "command": "SID=$(jq -r '.session_id') && rm -f \"$HOME/.local/state/plasma-keepawake/signals/claude-thinking.d/$SID\"" }] }
    ]
  }
}
```

Needs `jq` (present on this machine; worth a dependency note if this ever
gets its own packaging beyond a manual hook config, since it's not pulled
in by `packaging/PKGBUILD`).

Verified two ways: (1) piping a synthetic `{"session_id":"AAA",...}` /
`{"session_id":"BBB",...}` payload into the exact hook command strings for
two fake concurrent sessions, confirming `Stop` for session A leaves
session B's file (and thus the flag) intact, and only goes false once both
are removed; (2) against the real installed daemon with the real hooks
active for *this actual session* — `signals/claude-thinking.d/` contains a
file named by this session's real `session_id`, and `Rules` correctly
reports `claude-code-active: true`.

`Stop` fires once per full turn (not per individual tool call), so a
session's flag stays set across its whole multi-tool-call turn rather than
flickering between calls — the right granularity for "keep the machine
awake while Claude is working on this turn."

**Known limitation, still accepted:** if a session dies uncleanly between
`PreToolUse` and `Stop` (crash, `kill -9`, ...), *its own* file in
`claude-thinking.d/` is never removed — no longer wipes out other
sessions' flags (that part's fixed), but it does mean `claude-code-active`
stays stuck true forever if that stale file is the last one left, even
once every real session has ended. Same mitigation idea as before, scaled
up: a `SessionStart` hook that clears out the whole `claude-thinking.d/`
directory (a new session starting is still a reasonable signal that any
previous turn is over) — still not implemented, still unverified against
the hooks reference, still hasn't been observed as a real problem. Safe to
clean up by hand: `rm -rf ~/.local/state/plasma-keepawake/signals/claude-thinking.d`.

## Widget (plasmoid) — milestone 6, done

`widget/` is a plain KPackage plasmoid (`metadata.json` +
`contents/ui/main.qml`), no C++/CMake build step. It talks to
`org.plasmakeepawake.Daemon1` entirely by shelling out to `busctl
--json=short` through `Plasma5Support.DataSource`'s `"executable"` engine
(`GetAll` for status, individual calls for `SetRuleEnabled`/`ReloadConfig`)
rather than any generic D-Bus-from-QML binding — **there isn't one** in
Plasma 6 for a pure-QML plasmoid. This was confirmed, not assumed: even
KDE Connect's own widget needs a compiled C++ QML plugin
(`org.kde.kdeconnect`, see `DBusProperty.qml`) for its D-Bus access.
`busctl --json=short` gives clean, directly-`JSON.parse`-able output and
keeps this plasmoid pure QML.

Compact representation is a `Kirigami.Icon` in the panel; full
representation shows inhibiting/reason, a `ReloadError` banner if the last
reload failed, and a checkbox + true/false indicator per rule, each toggle
calling `SetRuleEnabled` directly (no local evaluation — everything shown
is read from the daemon).

Getting this running exposed a string of stale/wrong assumptions about the
current Plasma 6 QML API, each caught immediately by `plasmoidviewer`
(from `plasma-sdk`) printing the exact file:line of the failure rather than
by guessing:
- `IconItem` doesn't exist anywhere in this Plasma 6.7 install under
  either `org.kde.plasma.core` or `org.kde.plasma.components` (a Plasma 5
  holdover) — `Kirigami.Icon` is the current replacement.
- `toolTipMainText`/`toolTipSubText` are properties of `PlasmoidItem`
  itself (set directly on `root`), not of the `Plasmoid.` attached object —
  confirmed by reading the actual `.qmltypes` file rather than guessing
  again after the first attempt (`Plasmoid.toolTipMainText`) didn't error
  but `Plasmoid.toolTipSubText` did, which would have been a confusing
  inconsistency to paper over without checking.

Verified live end to end via `plasmoidviewer`: real `Rules`/`Inhibiting`/
`Reason` data rendered correctly (cross-checked against `busctl` output
directly), a checkbox toggle correctly called `SetRuleEnabled` and changed
the daemon's live state (confirmed via `busctl` immediately after), and
`ReloadConfig` correctly reset that same override back to the config
file's value — which is the designed behavior (reload replaces the rule
engine wholesale, per the config-schema section above), not a bug, though
it did initially look like one until the test sequencing was untangled.

One process note from this session: automated screenshot capture
(`spectacle -a`/`-f` + crop) twice grabbed the wrong window under this
Wayland session (an unrelated browser tab, then a terminal) since window
position/focus isn't reliably scriptable here — switched to asking for
direct visual confirmation instead rather than continuing to guess at
capture coordinates.

## Rule editing (milestone 7, done)

`AddRule(name, expr, enabled) -> (success, error)`, `UpdateRule(name,
expr) -> (success, error)`, and `RemoveRule(name) -> (success, error)` on
`org.plasmakeepawake.Daemon1`. Each validates before doing anything else
(`AddRule` rejects a duplicate name, `UpdateRule`/`RemoveRule` reject an
unknown one, and both `AddRule`/`UpdateRule` reject an `expr` that doesn't
compile via `RuleEngine::validate_expr` — reusing the same Rhai engine and
registered provider functions rules are actually evaluated with, not a
separate check that could drift) then, on success, mutate `config.rules`,
rebuild `RuleEngine` from it, and persist to the config file atomically
(write to a `.json.tmp` sibling, then rename) — all inside `state.rs`'s
`commit()`/`persist()`, so the in-memory state and the on-disk file always
change together.

This is deliberately a **separate persistence path** from `SetRuleEnabled`
(milestone 4), which stays a transient in-memory override that a reload
resets — add/update/remove are meant to durably change the config, the
enabled toggle is meant to be a quick "not right now" that doesn't require
touching the file. Worth knowing when reading `state.rs`: `SetRuleEnabled`
goes through `rule_engine.set_enabled` directly, the other three go through
`DaemonState`'s own methods that also call `persist()`.

Exposing `expr` required adding it to the `Rules` D-Bus property (now
`a(sbbss)`, name/enabled/currently_true/last_error/expr) — the widget needs
the current expression to pre-fill an edit field, and `engine::Rule` didn't
retain the original source string before this (only the compiled `AST`).

Widget side: an "Add rule" form (name + expr text fields), a pencil icon
per rule that swaps that row for an inline expr `TextField` (Enter or a
checkmark button to save, an X to cancel), and a trash icon per rule
calling `RemoveRule` directly with no confirmation dialog — accepted for
v1 since the config file is easy to hand-recover from, revisit if an
accidental-delete complaint ever comes up. Each of these three calls
carries its own `(success, error)` result back to the widget (unlike
`SetRuleEnabled`/`ReloadConfig`, which only trigger a blind refresh) via a
`pendingCallbacks` map in `main.qml` keyed by the exact command string,
since the executable data engine identifies a completed source by the
command that produced it — a failure (e.g. a bad expression, or a name
that already exists) surfaces as a red banner in the popup instead of
silently no-oping.

Verified live end to end via `plasmoidviewer` + a disposable test daemon:
"Add rule" created a new rule with the entered name/expression and it
showed up correctly in `busctl`'s view of `Rules` *and* in the config file
on disk; editing an existing rule's expression via the pencil icon updated
both the same way. Both were cross-checked against `busctl`/`cat` after
each step rather than trusting the widget's own display alone.

## Packaging and the notify feedback-loop bug (milestone 8, done)

License: GPL-3.0-or-later (`LICENSE`, fetched verbatim from
`gnu.org/licenses/gpl-3.0.txt` rather than retyped from memory — it's a
legal document, not something to risk getting subtly wrong). Set in
`daemon/Cargo.toml`'s `license` field and `widget/metadata.json`'s
`License` field too.

`packaging/PKGBUILD` builds the daemon (`cargo build --release`) and
installs it plus the systemd unit plus the widget's KPackage into a real
package — verified with an actual `makepkg` build, not just written and
assumed correct. It builds from this local checkout (`$startdir/..`)
rather than a downloaded tarball, since there's no published upstream URL
yet; noted in the PKGBUILD itself as not AUR-submission-ready as-is.

This *was* actually installed on this machine: `pacman -U` the built
package, real config written to `~/.config/plasma-keepawake/config.json`
(the example config, per request), `systemctl --user enable --now
plasma-keepawaked.service`, and the widget added to the real panel via
"Add Widgets" (which initially didn't show it at all — a running
`plasmashell` doesn't discover newly-installed system plasmoids on its
own, regardless of install method (`pacman` or `kpackagetool6 --install`
both failed to appear until `systemctl --user restart
plasma-plasmashell.service`, done with explicit confirmation since it
visibly flickers the whole desktop).

**A real, serious bug surfaced during this real deployment**, not caught
by any earlier testing because earlier tests never ran the daemon long
enough against its real config path with real edits: `AddRule`/
`UpdateRule`/`RemoveRule` (and, it turns out, the user genuinely using the
widget's editor on the real panel) triggered runaway CPU — 0% baseline
climbing past 40%+ within a couple minutes of an edit, sustained
indefinitely, discovered via `ps` showing the service's average CPU over
its lifetime and confirmed by deliberately reproducing it against a
disposable test daemon rather than continuing to experiment on the live
one (which was stopped immediately once the anomaly was noticed).

Root cause, found via temporary `eprintln!` instrumentation on the watch
callback (first attempt was contaminated by redirecting the debug
daemon's own stdout into the *same directory being watched*, which is a
different, self-inflicted feedback loop worth knowing about but not the
real bug — redone with the log elsewhere): `watch_config`'s callback
reloaded on *any* event whose path matched the config file, with no
filter on event *kind*. `DaemonState::reload()` opens and reads that same
file, which itself generates an `Access(Open)` inotify event on it — so
reload triggers a read, the read generates an access event, the access
event triggers another reload, forever. Each iteration is cheap (compile
a few Rhai expressions) but nonzero, so it manifests as climbing CPU
rather than an instant crash — exactly the kind of bug that's invisible
in a quick manual test (milestone 4's original hot-reload verification
only tested a *hand-edited* file once, and milestones didn't include a
CPU-usage check after a `persist()`-triggered write specifically).

Fix: only reload on `EventKind::is_create()` or `EventKind::is_modify()`,
explicitly excluding `is_access()` (`main.rs`'s `watch_config`). A
legitimate `persist()` write still triggers exactly one harmless
self-reload (a `Modify`/`Create` event on the real content change), which
is correct and intended — the bug was reacting to reads, not to writes.

Verified the fix three ways: (1) reproduced the exact runaway-CPU
sequence against a disposable daemon pre-fix, confirmed the same fix
resolves it post-fix (CPU settles back toward 0% instead of climbing,
sampled repeatedly over time rather than trusting one snapshot); (2)
redeployed the fixed build to the real installed service and watched its
CPU stay flat through a real `busctl`-driven edit; (3) had the user add a
rule through the actual widget on their actual panel and confirmed CPU
stayed flat through that too, not just through my own scripted D-Bus
calls. Test rules added during this verification were removed from the
real config afterward.

Lesson for future testing: any change touching the config-reload path
needs a "leave it running for a while after an edit, then check CPU"
step, not just "confirm the edit took effect" — the earlier milestones'
verification confirmed *correctness* of hot-reload but never checked its
*steady-state cost*, which is exactly where this bug lived.

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
5. ✅ **Claude Code hook wiring** — added to this machine's real
   `~/.claude/settings.json` (`PreToolUse`/`Stop`), verified end to end
   against a running daemon. See "Claude Code integration" above, including
   the accepted crash/stuck-flag limitation.
6. ✅ **Plasma widget v1** — status + toggle, read-only expr display. See
   "Widget (plasmoid)" above for how it talks to the daemon and the API
   corrections `plasmoidviewer` caught along the way.
7. ✅ **Widget rule editing** — `AddRule`/`UpdateRule`/`RemoveRule` on the
   daemon plus an editor in the widget (Add rule form, inline expr editing,
   remove button per rule). See "Rule editing" below.
8. ✅ **Packaging** — done and actually installed/enabled on this machine
   (not just written), including a real production bug found and fixed
   along the way. See "Packaging and the notify feedback-loop bug" below.

## Post-milestone: install script

`packaging/install.sh` wraps the manual install sequence documented above
into one command (`makepkg` → `pacman -U` → default config if none exists
→ enable + restart the service → ask, not assume, about restarting
`plasmashell`). Added after the fact once the manual sequence had already
been run by hand enough times during milestone 8 to be worth automating.
Idempotent by design: re-running it after a code change rebuilds,
reinstalls, and restarts the daemon (`restart`, not `enable --now`, since
the latter wouldn't pick up a new binary on an already-running service —
exactly the mistake that would've mattered most right after the CPU-loop
fix), while never touching an existing config. Tested for real against
this machine's already-installed, already-customized setup: config (which
by this point had rules the user added through the widget, different from
the seeded example) came through untouched, and the daemon restarted
clean.

## Open decisions

- ~~License — not chosen yet.~~ GPL-3.0-or-later (milestone 8) — `LICENSE`
  (verbatim from gnu.org, not retyped from memory), `daemon/Cargo.toml`'s
  `license` field, and `widget/metadata.json`'s `License` field.
- Whether `process_running` polling interval should be configurable
  per-rule or global (default to global, e.g. 5s, until there's a reason
  not to).
- Whether to add a `command(...)` provider (run an arbitrary shell command,
  exit code 0 = true) as a lower-effort general escape hatch alongside
  `signal()`. Leaning no for v1 — `signal()` covers the same use case with
  no per-evaluation process-spawn cost and no shell-injection surface from
  config content, but worth revisiting if a real case needs a live command
  result rather than a hook-toggled flag.

## Post-milestone: status icon + rule renaming (v0.2.0)

Two small feature requests after the widget had been in daily use for a
while: make the panel icon visually distinguish "sleep allowed" from
"sleep inhibited" (not just the tooltip text), and let a rule's name be
edited, not just its expression.

**Renaming.** Straightforward on the daemon side, following the exact
shape of `add_rule`/`update_rule`/`remove_rule` in `state.rs`:
`rename_rule(old_name, new_name)` rejects an unknown source name or a
collision with an existing name, is a no-op success if the names are
equal, and otherwise mutates `config.rules` in place and persists — same
`commit()` path as everything else, so the live `RuleEngine` is rebuilt
from the renamed config rather than patched. This is the first genuinely
untested code path in the daemon (the write-up throughout milestones 1-8
was all live/manual verification, no `#[cfg(test)]` anywhere yet), and it
was worth a small `mod tests` in `state.rs` rather than another live D-Bus
round-trip: renaming would otherwise have needed either a disposable
second daemon (can't - `org.plasmakeepawake.Daemon1` is a single
well-known bus name, no two instances can hold it) or exercising it
against the real, already-running, already-inhibiting service, which
wasn't worth the risk of a real inhibitor gap just to check a rename.
Exposed as `RenameRule(old, new) -> (bool, string)` on the D-Bus interface,
mirroring the other mutating methods. Widget side: the rule editor's
pencil icon now also pre-fills a name field, and `saveRule()` calls
`RenameRule` first (only if the name actually changed) and `UpdateRule`
second, so a rename collision surfaces as its own error instead of being
silently absorbed into (or masking) the expr update.

**Status icon.** This took several wrong turns worth recording, since
each one looked reasonable and failed for a different, non-obvious
reason:

1. First attempt: a single icon (`preferences-system-power-management`,
   the one already used) with `isMask: true` and `color:` switched between
   `Kirigami.Theme.positiveTextColor` (green, inhibiting) and
   `disabledTextColor` (gray, allowed). Wrong immediately - that icon has
   no `-symbolic` variant in Breeze at all (only in an unrelated, obscure
   `char-white` theme), so `isMask` was masking an icon that didn't
   resolve, rendering nothing.
2. Second attempt: Breeze actually ships icons named for exactly this
   state - `system-suspend-inhibited` / `system-suspend-uninhibited`,
   found under `status/22` and `status/24` in both `breeze` and
   `breeze-dark`. These looked perfect (literally named for the concept)
   and resolve fine via `kiconfinder6`, but rendered blank in the widget's
   `Kirigami.Icon` regardless. The reason is in Breeze's own
   `index.theme`: that bucket is commented "Icon(s) for Plasma theme/
   System Tray. Not particularly used on Plasma. - DO_NOT_USE_ANYWHERE_ELSE
   - Monochrome" - reserved for the System Tray plasmoid's own internal
   rendering path, not general reuse via a plain icon-name lookup.
3. Landed on `system-suspend-symbolic` (an `actions/` context icon, not
   `status/`) with the same `isMask` + `Kirigami.Theme.positiveTextColor`/
   `disabledTextColor`/`negativeTextColor` (daemon unreachable) switch as
   attempt 1. Confirmed via `kiconfinder6` that it resolves through the
   same lookup path as icons already known to render correctly elsewhere
   in this same widget (e.g. `document-edit`), and confirmed visually in
   the popup's full representation.

**The real time sink wasn't the icon - it was verifying it.**
`plasmoidviewer -f planar` doesn't wrap a real panel, so it was never a
valid test of the *compact* (panel/taskbar) representation - only ever a
test of the full (popup) representation, which cost a full round of
confusion before that was recognized. Then, once testing moved to the
real panel: rebuilding and reinstalling via `install.sh` updates the files
on disk (verified: `md5sum` on the installed vs. repo `main.qml` matched,
`pacman -Q` showed the new version, `busctl introspect` showed the new
`RenameRule` method) and restarting `plasma-plasmashell.service` is enough
to make a *newly added* widget instance show up (see milestone 8's
"widget not showing in Add Widgets" note) - but it is **not** enough to
make an *already-placed* widget instance reload its QML. That instance
kept running visibly stale code (an old icon, no rename field) through two
full rebuild-reinstall-restart cycles, with every on-disk artifact
confirmed correct the whole time. The fix was removing the widget from
the panel and re-adding it fresh from the widget picker - a new applet
instance with no chance of inherited state, not just a new process.
**Lesson for next time a widget UI change doesn't seem to take effect
despite a verified-correct install: don't trust "restart plasmashell" -
remove and re-add the applet instance itself.**

To make "is this actually the new build" independently checkable without
re-deriving all of the above each time, bumped `daemon/Cargo.toml`,
`packaging/PKGBUILD` (`pkgver`), and `widget/metadata.json` (`KPlugin.
Version`) to `0.2.0`, and added a small `"widget vX.Y.Z"` label to the
bottom of the widget's popup - a visible, no-guesswork way to confirm
which build a given panel instance is actually running.
