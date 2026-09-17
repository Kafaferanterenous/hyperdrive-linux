# PROMPT — Build HyperDrive on Windows

Copy-paste everything below into the AI assistant on your Windows machine.

---

You are building **HyperDrive**, an existing GPL-3.0 file manager written in Rust (eframe/egui 0.27.2 + rodio + libopenmpt FFI + image + rusqlite-bundled). The full source is in this folder (Cargo.toml, build.rs, src/, packaging/, LICENSE).

Goal: produce a working Windows x64 build and a portable zip.

## 1. Prerequisites (run once)
```powershell
# Rust (MSVC toolchain) — install via rustup-init.exe from https://rustup.rs
# Visual Studio Build Tools with "Desktop development with C++" workload
git --version          # any recent git
```
No ALSA/libasound needed on Windows — rodio/cpal uses WASAPI automatically.
rusqlite "bundled" feature compiles its own SQLite (needs VS C++ tools present).

## 2. libopenmpt for Windows (required by src/audio.rs FFI)
1. Download the Windows dev package: https://lib.openmpt.org/libopenmpt/download/
   (file like `libopenmpt-X.Y.Z-dev-win-x64.tar.gz` or .zip)
2. Extract, then place:
   - `libopenmpt.dll` + import lib `libopenmpt.lib` (or `openmpt.dll/.lib`) → `C:\hyperdrive-libs\`
3. Patch `build.rs` fallback section so MSVC finds it. Replace the `_ =>` fallback with:
```rust
_ => {
    println!("cargo:rustc-link-lib=openmpt");
    println!("cargo:rustc-link-search=native=C:\\hyperdrive-libs");
}
```
4. At runtime the DLL must be findable: copy `libopenmpt.dll` next to the final exe
   (the portable zip step below does this).

If linking still fails, alternative: set env `OPENMPT_DIR` and add
`println!("cargo:rustc-link-search=native={}\\lib", env)` — adjust to wherever you extracted.

## 3. Build
```powershell
cargo test            # 3 unit tests must pass
cargo build --release
```
Expected output binary: `target\release\hyperdrive.exe`.

## 4. Smoke test
Double-click `target\release\hyperdrive.exe`. Check:
- window opens, folder listing works, dual pane toggle works
- Settings → themes switch; font family dropdown should list **Segoe UI** (path already coded)
- play an mp3 AND an .xm module (audio out via WASAPI)

## 5. Package portable zip
```powershell
mkdir dist\HyperDrive-win64
copy target\release\hyperdrive.exe dist\HyperDrive-win64\
copy C:\hyperdrive-libs\libopenmpt.dll dist\HyperDrive-win64\
copy README.md dist\HyperDrive-win64\
Compress-Archive -Path dist\HyperDrive-win64 -DestinationPath HyperDrive-win64.zip
```
The app is already portable-first: it creates `hyperdrive.conf` / `hyperdrive.db`
beside the exe when that directory is writable.

## 6. Known platform notes
- Fonts: curated list includes Segoe UI path (`C:\Windows\Fonts\segoeui.ttf`) — done.
- `gio trash`/`xdg-open` calls are Linux-only; Windows open uses `cmd /C start` (already coded).
- Context-menu "Extract here" prefers 7z if `7z.exe` is on PATH (install 7-Zip), else fails gracefully.
- SMB/SFTP network mounting relies on GVFS (Linux-only). On Windows, network browsing is
  expected to be unavailable — do not attempt to port gio; leave menu items, they fail gracefully.

## 7. Report back
Commit nothing; just report: build success/failures, smoke-test results,
final zip size, and any source changes you had to make (especially build.rs).
