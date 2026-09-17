# HyperDrive Research — File Manager Landscape (2026-08-25)

Goal: build HyperDrive — multi-platform, portable, self-contained, fast.
Required UX: concise settings (font size + font family, legible fonts only),
themes: Normal, Dark, Pastel, Blue.

---

## 1. Explorer++ (v1.4.0, Jan 2024) — Windows only

- **Stack:** C++ / Win32 API. Download ~3 MB. GPL-3.0. 3,271 stars.
- **Why it matters to us:** proof that a native, dependency-free file manager can be tiny AND feature-rich.

**Best parts worth stealing:**

| Feature | Detail | Adopt? |
|---|---|---|
| True portability | Config saved to file OR registry; runs from USB stick | YES — config file next to exe by default |
| Tabs | Multiple folders in one window | YES |
| Bookmarks | Bookmark tabs and folders, persistent | YES |
| Selection preview pane | Shows preview of selected file instantly | LATER (M3) |
| Keyboard shortcuts | Memorable, consistent nav keys | YES — first-class from day 1 |
| View modes | Icon / list / detail / thumbnail / tile | YES (detail + icon first) |
| Filtering | Live filter of current listing | YES |
| Search | By name AND attributes | YES (name first) |
| Advanced ops | Merge/split files, change dates/attributes, save directory listing | LATER (M4+) |
| Plugin system | Embedded V8 JS plugins | NO — out of scope, complexity trap |

**Weakness:** Windows-only (deep Win32 coupling). Lesson: keep platform layer thin.

---

## 2. Spacedrive (v2.0.0-alpha.1, Dec 26 2025) — cross-platform

- **Stack:** Single Rust crate (CQRS/DDD), Tokio, SQLite (SeaORM/sqlx), Tauri 2 desktop,
  React 19 frontend, React Native mobile. AGPL/FSL. ~38.8k stars.
- **Status:** full ground-up rewrite after v1 stalled Jan 2025. Alpha = macOS/Linux; Windows in alpha.2.
- **Lesson learned from their v1 failure:** original monorepo (PRRTT stack, Prisma, pnpm workspace) got too complex → rewrite as ONE Rust crate with auto-generated TS types.

**Best parts worth stealing:**

| Feature | Detail | Adopt? |
|---|---|---|
| Core/UI separation | Rust core exposes typed commands; UI is replaceable | YES — architecture principle #1 |
| Content identity | BLAKE3 hashing w/ sampling for dedup | LATER — optional dedup tool (M5) |
| Transactional actions | Preview ops before execute (conflicts, space, time) | YES — for destructive batch ops |
| Durable jobs | Ops survive restart/interruption | YES — queue persisted to disk |
| Tags/metadata beyond path | Tag once, find anywhere | LATER — SQLite index (M5) |
| Instant search over index | DB-backed, not per-keystroke rescan | YES when indexing lands |
| Settings: light/dark toggle, language, default view | Clean minimal settings page | YES — matches our settings goal |

**Weakness:** heavy deps (FFmpeg, LanceDB, Whisper), daemon-centric design = NOT lightweight/portable.
Lesson: we take their architecture ideas, not their dependency weight.

---

## 3. Double Commander (active, 4.4k stars) — cross-platform

- **Stack:** Free Pascal / Lazarus. GPL-2.0. Dual-pane Total Commander clone.

**Best parts worth stealing:**

| Feature | Detail | Adopt? |
|---|---|---|
| Dual-pane mode | Side-by-side copy/move workflow | YES — toggleable (single ↔ dual pane) |
| Background file operations | Non-blocking copies/moves with progress queue | YES |
| Archive-as-folder browsing | ZIP/TAR/7z browsed transparently | LATER (via backend lib) |
| Multi-rename tool | Pattern-based batch rename with preview | YES (M4) — pairs with transactional preview |
| Custom columns | User-configurable detail columns | MAYBE (M5) |
| Full-text search | Content search inside files | LATER |
| Built-in viewer/editor (F3/F4) | Hex/text viewer | MAYBE (M4, text-only viewer) |

**Weakness:** dated UI, Pascal talent pool small. Lesson: features matter more than framework age.

---

## 4. muCommander (v1.6.2, May 2026) — cross-platform

- **Stack:** Java/Kotlin, bundles JRE. GPL-3.0. Dual-pane, tabs.
- **Best parts:** virtual filesystem abstraction (local + FTP/SFTP/SMB/NFS/S3 behind ONE API),
  checksum tools, credentials manager, themes, batch rename, pause/resume transfers.
- **Weakness:** ~109 MB download, JVM lag. Anti-pattern for "self-contained & fast".
- **Adopt:** the VFS-abstraction idea (one interface, many backends) — but with native backends,
  not a runtime. Pause/resume on long transfers: YES.

---

## 5. Cross-cutting synthesis — what HyperDrive takes from each

1. **From Explorer++:** portability-first config (file beside binary), tabs, bookmarks,
   keyboard shortcuts, view modes, live filter, lean footprint (~<15 MB target).
2. **From Spacedrive:** core/UI split with typed command API, durable op jobs,
   preview-before-execute, optional SQLite index for search/tags. Skip: daemon, P2P, AI, heavy media deps.
3. **From Double Commander:** dual-pane toggle, background ops queue, multi-rename with preview.
4. **From muCommander:** unified VFS interface for future network/cloud backends, pause/resume.

### Settings page spec (user requirement)
- Font size slider (12–28 px), applies live.
- Font family dropdown — curated LEGIBLE set only: Inter, Roboto, Noto Sans,
  Open Sans, Source Sans 3, DejaVu Sans, Ubuntu Sans, Atkinson Hyperlegible, IBM Plex Sans.
  No decorative/monospace-as-default/cursive faces.
- Theme picker: Normal (light), Dark, Pastel, Blue.
- Persisted to `hyperdrive.conf` (JSON/TOML) beside binary (portable) with fallback to
  `~/.config/hyperdrive/`.

### Theme spec
| Theme | BG | Text | Accent |
|---|---|---|---|
| Normal | #FFFFFF | #1A1A1A | #2563EB |
| Dark | #1E1E1E | #E6E6E6 | #4C9AFF |
| Pastel | #FDF6EC | #5B5147 | #F2A6B3 (+soft greens/blues) |
| Blue | #EFF6FF | #0F2A52 | #1D4ED8 |

## Tech stack decision (see PLAN.md)
Winner: **Rust + egui/eframe** — pure-Rust GUI, zero system dev-lib requirements at build time
(unlike Tauri/webkit2gtk which is BLOCKED on this machine without sudo), single static binary,
~10–20 MB, starts instantly, runs on Windows/macOS/Linux, font atlas + theming built-in
(font switch at runtime is a first-class feature).
