#!/bin/sh
# Installs lightview for the current user (building it first in a source checkout) and makes it
# the default image viewer.
# Undo: rm ~/.local/bin/lightview ~/.local/share/applications/lightview.desktop
# ~/.local/share/icons/hicolor/scalable/apps/lightview.svg,
# then pick another default app in System Settings → Default Applications / File Associations.
set -eu
cd "$(dirname "$0")"

# A release download ships the binary next to this script; a source checkout builds it.
if [ -x ./lightview ] && [ ! -f Cargo.toml ]; then
    bin=./lightview
else
    echo "Building (can take about a minute after code changes)…"
    cargo build --release
    bin=target/release/lightview
fi
install -Dm755 "$bin" "$HOME/.local/bin/lightview"
# Absolute Exec path: ~/.local/bin isn't always on the PATH the desktop session uses.
install -Dm644 icon/aperture.svg "$HOME/.local/share/icons/hicolor/scalable/apps/lightview.svg"
mkdir -p "$HOME/.local/share/applications"
sed "s|^Exec=lightview|Exec=$HOME/.local/bin/lightview|" lightview.desktop > "$HOME/.local/share/applications/lightview.desktop"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
xdg-mime default lightview.desktop image/jpeg image/png image/gif image/webp image/bmp image/tiff

echo "Installed. Default for JPEG is now: $(xdg-mime query default image/jpeg)"
