#!/bin/sh
# Builds lightview_<version>_amd64.deb in the repository root.
# Build on the oldest distro to support: the package needs that distro's glibc or newer.
set -eu
cd "$(dirname "$0")"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
cargo build --release

stage=target/deb
rm -rf "$stage"
install -Dsm755 target/release/lightview "$stage/usr/bin/lightview"
install -Dm644 lightview.desktop "$stage/usr/share/applications/lightview.desktop"
install -Dm644 icon/aperture.svg "$stage/usr/share/icons/hicolor/scalable/apps/lightview.svg"
mkdir -p "$stage/usr/share/doc/lightview" "$stage/DEBIAN"
{
    printf 'lightview\nCopyright (C) 2026 Pablo Yubero\nLicensed under the GNU General Public License, version 3 or later.\n\n'
    cat LICENSE
    printf '\n\n'
    cat fonts/NOTICE.txt fonts/LICENSE-APACHE-2.0.txt fonts/emoji-icon-font-mit-license.txt
    printf '\n\n'
    ./third-party-notices.py
} > "$stage/usr/share/doc/lightview/copyright"
cat > "$stage/DEBIAN/control" <<EOF
Package: lightview
Version: $version
Section: graphics
Priority: optional
Architecture: amd64
Installed-Size: $(du -sk "$stage/usr" | cut -f1)
Depends: libc6 (>= 2.35), libgcc-s1, xdg-utils
Recommends: libturbojpeg | libturbojpeg0, curl, unzip
Maintainer: Pablo Yubero
Description: A simple, fast image viewer
 Opens images instantly, with light editing and optional local AI tools.
EOF

dpkg-deb --build --root-owner-group "$stage" "lightview_${version}_amd64.deb"
