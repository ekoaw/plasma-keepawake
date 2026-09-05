#!/usr/bin/env bash
# Builds and installs plasma-keepawake on an Arch-based system: package
# build + install, a default config (if none exists yet), and enabling the
# daemon service. Safe to re-run - it won't touch an existing config, and
# always rebuilds + restarts the daemon so a re-run after a code change
# actually picks up the fix (systemctl enable --now would not restart an
# already-running service).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/plasma-keepawake"
CONFIG_FILE="$CONFIG_DIR/config.json"
EXAMPLE_CONFIG="$REPO_ROOT/daemon/examples/config.json"

require() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: missing required command '$1'. $2" >&2
        exit 1
    fi
}

echo "==> Checking dependencies"
require pacman "This installer targets Arch-based distros (pacman)."
require makepkg "Install with: sudo pacman -S --needed base-devel"
require cargo "Install a Rust toolchain, e.g.: sudo pacman -S --needed rust (or via rustup)"
require systemctl "This installer expects a systemd user session."

echo "==> Building package"
(cd "$SCRIPT_DIR" && makepkg -f --nosign)
PKG_FILE=$(cd "$SCRIPT_DIR" && ls -t plasma-keepawake-*.pkg.tar.zst | head -1)

echo "==> Installing $PKG_FILE (will prompt for your sudo password)"
sudo pacman -U --noconfirm "$SCRIPT_DIR/$PKG_FILE"

echo "==> Config"
if [ -f "$CONFIG_FILE" ]; then
    echo "    $CONFIG_FILE already exists, leaving it as-is."
else
    mkdir -p "$CONFIG_DIR"
    cp "$EXAMPLE_CONFIG" "$CONFIG_FILE"
    echo "    wrote the example config to $CONFIG_FILE"
    echo "    edit it by hand, or use the widget's Add/Edit/Remove rule buttons once it's running."
fi

echo "==> Starting the daemon"
systemctl --user daemon-reload
systemctl --user enable plasma-keepawaked.service
systemctl --user restart plasma-keepawaked.service
sleep 1
systemctl --user --no-pager --lines=0 status plasma-keepawaked.service || true

echo
echo "==> Widget"
echo "    Right-click your panel -> Add Widgets... -> search 'Plasma Keepawake'."
echo "    If it isn't there: plasmashell only discovers a newly-installed widget"
echo "    after a restart (panel layout is preserved, but it's a visible flicker)."
read -r -p "    Restart plasmashell now so the widget shows up? [y/N] " reply
case "$reply" in
    [yY]*)
        systemctl --user restart plasma-plasmashell.service
        echo "    plasmashell restarted."
        ;;
    *)
        echo "    Skipped - run this later if needed: systemctl --user restart plasma-plasmashell.service"
        ;;
esac

echo
echo "Done."
echo "  Config: $CONFIG_FILE"
echo "  Logs:   journalctl --user -u plasma-keepawaked -f"
echo "  Status: systemctl --user status plasma-keepawaked"
