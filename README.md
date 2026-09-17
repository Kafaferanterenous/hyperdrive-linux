# HyperDrive

**USE IT AT YOUR OWN RISK !** — file operations are real. Test on disposable
folders first. Powershell/MP3 tags etc. are provided as-is with no warranty
(see LICENSE).

A fast, portable, self-contained file manager for Linux (Windows/macOS planned).
Written in Rust with [egui](https://github.com/emilk/egui) — one binary, no runtime dependencies.

![license](https://img.shields.io/badge/license-GPL--3.0-blue)

## Features

- **Dual pane** with independent history, sort and filters; F5/F6 quick copy/move between panes
- **Directory tree sidebar** with auto-reveal, bookmarks and network section
- **Tabs** (Ctrl+T / Ctrl+W / Ctrl+PgUp/PgDn), middle-click folder = open in tab
- **Built-in media player** — mp3/wav/ogg/flac/m4a **and tracker modules** (XM · S3M · MOD · IT · MPTM) via libopenmpt
- **Image thumbnails + preview panel** with disk cache (`~/.cache/hyperdrive/`)
- **Text file previews**
- **Rubber-band selection**, Ctrl/Shift multi-select, inline rename (F2)
- **Dolphin-style menu bar**, right-click context menus
- **Background operations queue** with progress + cancel (copy/move/delete/extract)
- **Network**: SMB connect via GVFS (no root), mDNS server discovery, SFTP, share-folder wizard with security checklist
- **SQLite tags** (7 colors) stored portably beside the binary when possible
- **Recursive search** (Ctrl+F) with extension/size filters
- **4 themes** (Normal/Dark/Pastel/Blue) + font size/family settings from a curated legible list
- Session restore: panes, tabs, directories, sorts — all persisted to `hyperdrive.conf`
- Trash via `gio trash`; permanent delete with confirmation (Shift+Del)

## Build

```bash
sudo apt install -y libasound2-dev libopenmpt-dev pkg-config   # build deps (Debian/Mint)
cargo build --release
./target/release/hyperdrive
```

Packaging: `packaging/make-appimage.sh` (AppImage) · `packaging/make-deb.sh` (.deb)

## Portable mode

Drop `hyperdrive` (or the AppImage extracted) anywhere writable — config `hyperdrive.conf`,
tags DB `hyperdrive.db` are created next to the binary. Otherwise they live in
`~/.config/hyperdrive/` and `~/.local/share/hyperdrive/`.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| F2 / Shift+F2-style dialogs | Rename (inline) / Multi-Rename |
| Del · Shift+Del | Trash · Delete permanently |
| F4 / F5 | Terminal here · Reload |
| Alt+← → ↑ Backspace | Back Forward Up |
| Ctrl+C X V A L F H T W | clipboard, select-all, path bar, search, hidden, new/close tab |
| Ctrl+PgUp/PgDn | cycle tabs |
| Enter · Alt+Enter | Open selection · Properties |

## License

GPL-3.0-or-later. Copyright (C) 2026 dragon.
Free use WITH attribution; derivatives must also be GPL — no closed-source resale.

## Acknowledgements

Design inspiration: Explorer++, Spacedrive, Double Commander, Dolphin, OneCommander.
Built with egui, rodio, libopenmpt, image, rusqlite.
