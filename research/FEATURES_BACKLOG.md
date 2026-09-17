# HyperDrive Feature Backlog — Spacedrive / OneCommander / Linux FM synthesis
Compiled 2026-08-25 after installing Spacedrive (v1 0.4.3 + v2 alpha.2 server) on testmachine.

## Spacedrive — what to adopt
| Idea | Notes for HyperDrive |
|---|---|
| Sidebar sections: Locations / Tags / Recents | We have Bookmarks/Devices/Network — add **Recents** (MRU dirs) + later Tags |
| Overview dashboard (storage bars, recent files) | M5: "Overview" start page per pane |
| Inspector right panel (file details/thumbnail) | M5: toggleable right info panel |
| Jobs center UI (progress of copies/indexing) | Feeds our M4 ops queue UI |
| Grid + list view toggle | M5 icon/grid view mode |
| Tag colors on files | SQLite index era (M6) |
| Settings: appearance/language/default-view | We match already; add language later |

Observed on this machine: v1 window renders with distortion (Tauri/WebKit GPU issue in VM);
v2 alpha runs as local server (127.0.0.1:6969, needs DATA_DIR env; FFmpeg7 bundled in .deb works via LD_LIBRARY_PATH).
Lesson: our egui stack avoids their entire class of WebKit rendering bugs.

## OneCommander — what to adopt
| Idea | Notes |
|---|---|
| macOS-style COLUMN view | Distinctive; M7 candidate (path-per-column browsing) |
| Preview pane with instant content preview | text/image preview inline (M6) |
| Color labels/flags on files | pairs with tags DB |
| Modern fluent spacing/typography | keep our large-font legibility direction |
| Batch rename wizard UI | merge with our planned multi-rename (M4) |

## Classic Linux FMs (Dolphin/Nemo/Thunar/PCManFM/Krusader) — adopt list
Already done by us now:
✓ menu bar File/Edit/View/Go/Bookmarks/Help · tree sidebar sections · dual pane · hidden files · clipboard cut/copy/paste · trash via gio · new folder/rename · SMB connect (gio mount, no root) · share wizard w/ security tips

To add (prioritized):
1. **F2/F5/F6/Del/Ctrl+H keyboard shortcuts** (rename/copy-to-other-pane/move-to-other-pane/trash/toggle hidden) — Dolphin muscle memory
2. Embedded terminal panel (Dolphin's killer feature; `xterm -e "cd $cwd"` fallback: embed via spawn)
3. Open With… dialog + default-app association memory
4. Properties dialog (permissions/chk numeric, sizes, times)
5. Symlink creation (relative/absolute), duplicate (Ctrl+D)
6. Inline rename (edit name cell in place)
7. Rubber-band selection rectangle
8. Thumbnails for images/video in list rows (tiny, cached)
9. Archive context actions via file-roller/xarchiver CLI if present
10. Disk usage column mode (du% bar like baobab-lite)
11. Split view sync-navigation lock (Krusader)
12. Search-as-you-type across subdirs (recursive find w/ thread pool)
13. Bookmarks editing manager dialog (reorder/delete/rename)
14. Session restore (reopen last tabs/panes/dirs) — config already persists panes
15. Icon themes pack + theme editor (we have 4; add accent-color picker)

## Deliberately NOT adopting
- Cloud/AI/P2P indexing (Spacedrive) — against portable+light goal
- Plugin JS engine (Explorer++ V8) — complexity trap
- Java-style universal VFS runtime (muCommander) — we do trait-based backends instead
