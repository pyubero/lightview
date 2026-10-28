#!/bin/sh
# Builds lightview, installs it for the current user and makes it the default image viewer.
# Undo: rm ~/.local/bin/lightview ~/.local/share/applications/lightview.desktop,
# then pick another default app in System Settings → Default Applications / File Associations.
set -eu
cd "$(dirname "$0")"

cargo build --release
install -Dm755 target/release/lightview "$HOME/.local/bin/lightview"
# Absolute Exec path: ~/.local/bin isn't always on the PATH the desktop session uses.
mkdir -p "$HOME/.local/share/applications"
sed "s|^Exec=lightview|Exec=$HOME/.local/bin/lightview|" lightview.desktop > "$HOME/.local/share/applications/lightview.desktop"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
xdg-mime default lightview.desktop image/jpeg image/png image/gif image/webp image/bmp

echo "Installed. Default for JPEG is now: $(xdg-mime query default image/jpeg)"
