# PROMPT — Build HyperDrive on macOS

Copy-paste everything below into the AI assistant on your Mac.

---

You are building **HyperDrive**, an existing GPL-3.0 file manager written in Rust (eframe/egui 0.27.2 + rodio + libopenmpt FFI + image + rusqlite-bundled). The full source is in this folder (Cargo.toml, build.rs, src/, packaging/, LICENSE).

Goal: working Apple Silicon (aarch64-apple-darwin) build, tested, packaged as a minimal `.app` + zip. Intel: add `--target x86_64-apple-darwin` at the end if desired.

## 1. Prerequisites (once)
```bash
xcode-select --install
curl https://sh.rustup.rs | sh      # rustup, stable toolchain
brew install pkg-config libopenmpt create-dmg
```
Notes:
- No ALSA on macOS — rodio/cpal uses CoreAudio automatically.
- rusqlite "bundled" compiles its own SQLite (clang from Xcode CLT handles it).
- libopenmpt via Homebrew provides headers + dylib that pkg-config finds;
  our `build.rs` already shells out to `pkg-config --libs libopenmpt`, so no edits needed.

## 2. Build & test
```bash
cargo test                 # 3 unit tests must pass
cargo build --release
```
Binary: `target/release/hyperdrive`

## 3. Smoke test (GUI)
```bash
./target/release/hyperdrive
```
Check: window opens, navigation/sort/filter work, dual-pane toggle, themes switch
(font list will show whatever curated fonts exist under /System/Library/Fonts —
macOS ships none of our Linux paths; the app falls back to its built-in default font, which is fine),
play an mp3 and an .xm module (CoreAudio output).

## 4. Package as minimal .app + zip
```bash
APP=build-macos/HyperDrive.app/Contents/MacOS
mkdir -p "$APP" build-macos/HyperDrive.app/Contents/Resources
cp target/release/hyperdrive "$APP/"
cp packaging/hyperdrive.png build-macos/HyperDrive.app/Contents/Resources/hyperdrive.icns 2>/dev/null || \
  cp packaging/hyperdrive.png build-macos/HyperDrive.app/Contents/Resources/hyperdrive.png
cat > build-macos/HyperDrive.app/Contents/Info.plist << 'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>HyperDrive</string>
  <key>CFBundleIdentifier</key><string>local.hyperdrive</string>
  <key>CFBundleExecutable</key><string>hyperdrive</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>hyperdrive</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
# link openmpt dylib inside the bundle so it runs on machines without brew
LIBOMPT=$(otool -L "$APP/hyperdrive" | grep -o '/[^ ]*libopenmpt[^ ]*dylib' | head -1)
if [ -n "$LIBOMPT" ]; then
  mkdir -p build-macos/HyperDrive.app/Contents/Frameworks
  cp "$LIBOMPT" build-macos/HyperDrive.app/Contents/Frameworks/
  install_name_tool -change "$LIBOMPT" \
    "@executable_path/../Frameworks/$(basename "$LIBOMPT")" "$APP/hyperdrive"
fi
codesign --force --deep -s - build-macos/HyperDrive.app   # ad-hoc sign
create-dmg --volname HyperDrive HyperDrive-macos.dmg build-macos/HyperDrive.app || \
  zip -r HyperDrive-macos.zip build-macos/HyperDrive.app
```

## 5. Known platform notes
- Trash/permanent-delete use Linux `gio trash`; on macOS Delete Permanently works,
  Move-to-Trash will report failure — acceptable v1 (documented).
- Network SMB connect uses GVFS (Linux-only); menu items fail gracefully on macOS.
- First launch of ad-hoc-signed app may need right-click → Open (Gatekeeper).

## 6. Report back
Build/smoke results, dmg/zip size, any source changes (expect zero except possibly build.rs tweaks).
