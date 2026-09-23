#!/usr/bin/env bash
# HyperDrive AppImage builder. Run after `cargo build --release`.
set -euo pipefail
cd "$(dirname "$0")/.."

APP=HyperDrive
APPDIR=build-appimage/AppDir
BIN=target/release/hyperdrive
LDEPLOY=packaging/linuxdeploy-x86_64.AppImage

[ -x "$BIN" ] || { echo "release binary missing - run cargo build --release"; exit 1; }

mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"
cp "$BIN" "$APPDIR/usr/bin/hyperdrive"
# Bundle the Pdfium engine for the inbuilt PDF viewer beside the binary so
# the AppImage stays one self-contained file.
PDFIUM_SRC="${PDFIUM_SRC:-/usr/bin/libpdfium.so}"
if [ -f "$PDFIUM_SRC" ]; then
  cp "$PDFIUM_SRC" "$APPDIR/usr/bin/libpdfium.so"
  echo "bundled libpdfium.so from $PDFIUM_SRC"
else
  echo "WARNING: $PDFIUM_SRC not found; PDF viewer will be unavailable" >&2
fi
cp packaging/hyperdrive.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/hyperdrive.png"
cat > "$APPDIR/usr/share/applications/hyperdrive.desktop" << DESKTOP
[Desktop Entry]
Type=Application
Name=HyperDrive
Comment=Fast, portable, self-contained file manager
Exec=hyperdrive
Icon=hyperdrive
Categories=System;FileManager;
Terminal=false
DESKTOP
cat > "$APPDIR/AppRun" << RUN
#!/bin/bash
HERE="\$(dirname "\$(readlink -f "\$0")")"
exec "\$HERE/usr/bin/hyperdrive" "\$@"
RUN
chmod +x "$APPDIR/AppRun"

if [ ! -x "$LDEPLOY" ]; then
  echo "downloading linuxdeploy..."
  curl -sfL -o "$LDEPLOY" \
    https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
  chmod +x "$LDEPLOY"
fi

VERSION=$(date +%Y%m%d)
# No-FUSE environments: extract linuxdeploy once, use its AppRun.
if [ ! -x squashfs-root/AppRun ]; then
  "$LDEPLOY" --appimage-extract >/dev/null
fi
OUTPUT="HyperDrive-$VERSION-x86_64.AppImage" \
  squashfs-root/AppRun --appdir "$APPDIR" --output appimage
echo "DONE: $(ls HyperDrive-*.AppImage | tail -1)"
