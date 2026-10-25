#!/usr/bin/env bash
# Adds MoxRay to the application launcher (menu, dock, taskbar icon) for the current user.
# GNOME and KDE find a window's icon through a .desktop file named after its app id, which
# for this app is "moxray". Usage: scripts/install-desktop.sh [path/to/moxray-binary]
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
binary="${1:-}"
if [[ -z "$binary" ]]; then
  for candidate in "$root/target/release/moxray" "$root/target/debug/moxray"; do
    [[ -x "$candidate" ]] && { binary="$candidate"; break; }
  done
fi
[[ -x "$binary" ]] || { echo "No built binary found. Run 'cargo build --release' first." >&2; exit 1; }
binary="$(realpath "$binary")"

data="${XDG_DATA_HOME:-$HOME/.local/share}"
mkdir -p "$data/applications"
for size in 32 128 256 512; do
  src="$root/icons/${size}x${size}.png"
  [[ $size == 256 ]] && src="$root/icons/128x128@2x.png"
  [[ $size == 512 ]] && src="$root/icons/icon.png"
  mkdir -p "$data/icons/hicolor/${size}x${size}/apps"
  cp "$src" "$data/icons/hicolor/${size}x${size}/apps/moxray.png"
done

cat > "$data/applications/moxray.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=MoxRay
Comment=Proxy client for Xray-core
Exec="$binary"
Icon=moxray
Terminal=false
Categories=Network;
StartupWMClass=moxray
DESKTOP

command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
echo "Installed launcher for $binary"
