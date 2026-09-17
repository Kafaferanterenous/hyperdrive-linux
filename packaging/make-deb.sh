#!/usr/bin/env bash
# Build a minimal .deb for Debian/Ubuntu/Mint.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=${1:-0.4}
PKG=hyperdrive_${VERSION}_amd64
rm -rf build-deb/$PKG
mkdir -p build-deb/$PKG/DEBIAN build-deb/$PKG/usr/bin build-deb/$PKG/usr/share/applications \
         build-deb/$PKG/usr/share/icons/hicolor/256x256/apps
cp target/release/hyperdrive build-deb/$PKG/usr/bin/
cp packaging/hyperdrive.png build-deb/$PKG/usr/share/icons/hicolor/256x256/apps/hyperdrive.png
cat > build-deb/$PKG/usr/share/applications/hyperdrive.desktop << D
[Desktop Entry]
Type=Application
Name=HyperDrive
Comment=Fast, portable, self-contained file manager
Exec=hyperdrive
Icon=hyperdrive
Categories=System;FileManager;
Terminal=false
D
cat > build-deb/$PKG/DEBIAN/control << C
Package: hyperdrive
Version: $VERSION
Section: utils
Priority: optional
Architecture: amd64
Depends: libc6, libasound2t64 | libasound2
Maintainer: dragon <dragon@localhost>
Description: Fast, portable, self-contained file manager
 HyperDrive is a GPL-3.0 dual-pane file manager written in Rust.
C
dpkg-deb --build build-deb/$PKG
echo "DONE: build-deb/${PKG}.deb"
