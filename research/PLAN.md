# HyperDrive Plan

Project: 006 · Created 2026-08-25 · Linux Mint 22.3 dev machine (4 GB RAM, no sudo)

## Product definition
A fast, portable, self-contained file manager.
- **Later** multi-platform (Windows/macOS/Linux) — architecture chosen now for it
- Settings page: font size + curated legible-font family picker only
- Themes: Normal / Dark / Pastel / Blue
- Feature set = best-of synthesis (see research/RESEARCH.md)

## Stack decision: Rust + egui (eframe)

| Criterion | Rust+egui | Tauri2 | Electron | C++/Qt |
|---|---|---|---|---|
| Builds on THIS machine today | YES | NO¹ | YES | ?² |
| Binary size | 10–20 MB | 8–15 MB | 100+ MB | 15–40 MB |
| Startup | instant | fast | slow | fast |
| RAM idle | ~50–80 MB | ~120 MB | ~300+ MB | ~100 MB |
| Runtime deps on target OS | none³ | webkit | chromium bundle | qt libs |
| Font switch at runtime | built-in (atlas) | CSS | CSS | QFont |
| Themes | code colors | CSS | CSS | QSS |

¹ Tauri blocked: needs libwebkit2gtk-4.1-dev, installable only via sudo (unavailable).
² Qt dev libs not verified on machine.
³ X11/Wayland system libs already present on any desktop distro; Windows/macOS need nothing extra.

egui gives us runtime font switching and programmatic theming as first-class features —
exactly the settings requirements. Single `cargo build --release` produces the portable binary;
AppImage/bundle packaging later.

Fallback if egui proves unusable: C++ FLTK or GTK4 C (both proven working on this box).

## Architecture (Spacedrive lesson, without the weight)

```
crates/
  hd-core/      # NO GUI DEPS: fs ops, VFS trait, jobs queue, config, search
  hd-ui/        # egui frontend, talks to core via typed commands
```
Rules:
1. hd-core compiles headless; unit-tested without opening a window.
2. All file mutations go through a Job queue → durable, previewable, cancellable.
3. Platform-specific code isolated behind traits from day 1 (multi-platform later).
4. Config: portable-first — `hyperdrive.conf` beside binary if writable,
   else `~/.config/hyperdrive/hyperdrive.conf`.

## Milestones

- **M0 — Skeleton (est. 1–2 h):** cargo workspace, eframe window opens, theme constants,
  config load/save. VALIDATE egui builds clean on this machine before anything else.
- **M1 — Browser core (est. 3–4 h):** directory listing (async, non-blocking), navigation,
  breadcrumbs/back-forward, single pane, detail view w/ sortable columns, live filter box.
- **M2 — Settings + themes (est. 2–3 h):** settings modal: font size slider, font dropdown
  (curated list), theme picker (Normal/Dark/Pastel/Blue), all applied LIVE, persisted.
- **M3 — Tabs + bookmarks (est. 2–3 h):** tab strip, per-tab cwd/history, bookmark bar,
  keyboard shortcuts (Ctrl+T new tab, Ctrl+L path bar, F3 view toggle...).
- **M4 — Operations (est. 3–5 h):** copy/move/delete/rename through job queue with progress,
  pause/cancel, background queue panel; multi-rename tool with preview (DC lesson);
  dual-pane toggle (DC lesson); icon view mode.
- **M5 — Index & extras (later):** SQLite index for search/tags, dedup tool (BLAKE3),
  archive browsing, network VFS backends behind trait (muCommander lesson).
- **M6 — Packaging:** AppImage now; Windows NSIS/portable-zip + macOS app later.

## Immediate next action
M0: create workspace, add eframe dep, build hello-window release binary (~10 min incl. compile).
