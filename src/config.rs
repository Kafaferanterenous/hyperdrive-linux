//! HyperDrive - config persistence (portable-first).
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    Normal,
    Dark,
    Pastel,
    Blue,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 4] = [
        ThemeChoice::Normal,
        ThemeChoice::Dark,
        ThemeChoice::Pastel,
        ThemeChoice::Blue,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Normal => "Normal",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Pastel => "Pastel",
            ThemeChoice::Blue => "Blue",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "Normal" => Some(ThemeChoice::Normal),
            "Dark" => Some(ThemeChoice::Dark),
            "Pastel" => Some(ThemeChoice::Pastel),
            "Blue" => Some(ThemeChoice::Blue),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub font_size: f32,
    /// Empty string = egui built-in default font.
    pub font_family: String,
    pub show_tree: bool,
    pub dual_pane: bool,
    pub active_pane: usize,
    /// Single-click opens/plays audio files (mp3/wav/ogg/flac/m4a).
    pub audio_one_click: bool,
    pub show_hidden: bool,
    pub show_preview: bool,
    pub bookmarks: Vec<String>,
    pub net_locations: Vec<(String, String)>, // (label, smb://url)
    pub pane_paths: [String; 2],
    pub side_tabs: [Vec<String>; 2],
    pub pane_sort: [(usize, bool); 2], // (key_idx, ascending)
    pub recents: Vec<String>,
    pub sync_browse: bool,
    pub openwith: Vec<(String, String)>, // (ext lowercase incl dot, desktop file path)
    pub grid_view: bool,
    pub tag_filter: Option<String>,
    pub col_date_w: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Dark,
            font_size: 16.0,
            font_family: String::new(),
            show_tree: true,
            dual_pane: false,
            active_pane: 0,
            audio_one_click: false,
            show_hidden: false,
            show_preview: true,
            bookmarks: Vec::new(),
            net_locations: Vec::new(),
            pane_paths: [String::new(), String::new()],
            side_tabs: [Vec::new(), Vec::new()],
            pane_sort: [(0usize, true), (0usize, true)],
            openwith: Vec::new(),
            grid_view: false,
            tag_filter: None,
            col_date_w: 138.0,
            recents: Vec::new(),
            sync_browse: false,
        }
    }
}

/// Portable-first: config beside the binary if writable, else ~/.config/hyperdrive/.
pub fn config_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("hyperdrive.conf");
            // Writable test: only trust it if it already exists OR dir is writable at save time.
            if candidate.exists() {
                return candidate;
            }
        }
    }
    fallback_path()
}

fn fallback_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("hyperdrive")
        .join("hyperdrive.conf")
}

impl Settings {
    pub fn load_or_default() -> Self {
        let mut s = Self::default();
        let path = config_path();
        if let Ok(text) = fs::read_to_string(path) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    match k.trim() {
                        "theme" => {
                            if let Some(t) = ThemeChoice::parse(v) {
                                s.theme = t;
                            }
                        }
                        "font_size" => {
                            if let Ok(f) = v.trim().parse::<f32>() {
                                if (12.0..=28.0).contains(&f) {
                                    s.font_size = f;
                                }
                            }
                        }
                        "font_family" => s.font_family = v.trim().to_string(),
                        "show_tree" => s.show_tree = v.trim() == "1",
                        "dual_pane" => s.dual_pane = v.trim() == "1",
                        "active_pane" => {
                            if let Ok(n) = v.trim().parse::<usize>() {
                                s.active_pane = n.min(1);
                            }
                        }
                        "audio_one_click" => s.audio_one_click = v.trim() == "1",
                        "show_hidden" => s.show_hidden = v.trim() == "1",
                        "show_preview" => s.show_preview = v.trim() == "1",
                        "grid_view" => s.grid_view = v.trim() == "1",
                        "sync_browse" => s.sync_browse = v.trim() == "1",
                        "recents" => {
                            s.recents = v.split(';').filter(|x| !x.is_empty())
                                .map(|x| x.to_string()).collect();
                        }
                        "bookmarks" => {
                            s.bookmarks = v.split(';').filter(|x| !x.is_empty())
                                .map(|x| x.to_string()).collect();
                        }
                        "pane0_path" => s.pane_paths[0] = v.trim().to_string(),
                        "pane1_path" => s.pane_paths[1] = v.trim().to_string(),
                        "side0_tabs" => {
                            s.side_tabs[0] = v.split(';').filter(|x| !x.is_empty())
                                .map(|x| x.to_string()).collect();
                        }
                        "side1_tabs" => {
                            s.side_tabs[1] = v.split(';').filter(|x| !x.is_empty())
                                .map(|x| x.to_string()).collect();
                        }
                        "pane0_sort" => parse_sort(&mut s.pane_sort[0], v),
                        "pane1_sort" => parse_sort(&mut s.pane_sort[1], v),
                        "openwith" => {
                            s.openwith = v.split(';').filter(|x| x.contains('|'))
                                .filter_map(|x| {
                                    let (a, b) = x.split_once('|')?;
                                    Some((a.to_string(), b.to_string()))
                                }).collect();
                        }
                        "net_locations" => {
                            s.net_locations = v.split(';').filter(|x| x.contains('|'))
                                .map(|x| {
                                    let (a, b) = x.split_once('|').unwrap_or((x, ""));
                                    (a.to_string(), b.to_string())
                                }).collect();
                        }
                        "col_date_w" => {
                            if let Ok(f) = v.trim().parse::<f32>() {
                                if (40.0..=300.0).contains(&f) {
                                    s.col_date_w = f;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let body = format!(
            "theme={}\nfont_size={:.1}\nfont_family={}\nshow_tree={}\ndual_pane={}\nactive_pane={}\naudio_one_click={}\nshow_hidden={}\nshow_preview={}\ngrid_view={}\nsync_browse={}\nrecents={}\nbookmarks={}\nnet_locations={}\npane0_path={}\npane1_path={}\nside0_tabs={}\nside1_tabs={}\npane0_sort={}\npane1_sort={}\nopenwith={}\ncol_date_w={:.1}\n",
            self.theme.label(),
            self.font_size,
            self.font_family,
            if self.show_tree { 1 } else { 0 },
            if self.dual_pane { 1 } else { 0 },
            self.active_pane,
            if self.audio_one_click { 1 } else { 0 },
            if self.show_hidden { 1 } else { 0 },
            if self.show_preview { 1 } else { 0 },
            if self.grid_view { 1 } else { 0 },
            if self.sync_browse { 1 } else { 0 },
            self.recents.iter().take(12).cloned().collect::<Vec<_>>().join(";"),
            self.bookmarks.join(";"),
            self.net_locations.iter().map(|(a,b)| format!("{a}|{b}")).collect::<Vec<_>>().join(";"),
            self.pane_paths[0],
            self.pane_paths[1],
            self.side_tabs[0].join(";"),
            self.side_tabs[1].join(";"),
            format_args!("{} {}", self.pane_sort[0].0, if self.pane_sort[0].1 {1} else {0}),
            format_args!("{} {}", self.pane_sort[1].0, if self.pane_sort[1].1 {1} else {0}),
            self.openwith.iter().map(|(a,b)| format!("{a}|{b}")).collect::<Vec<_>>().join(";"),
            self.col_date_w,
        );
        // Try portable location first, fall back to user config dir.
        if fs::write(&path, &body).is_ok() {
            return Ok(());
        }
        fs::write(fallback_path(), body)
    }
}

fn parse_sort(slot: &mut (usize, bool), v: &str) {
    let mut it = v.split_whitespace();
    if let Some(k) = it.next().and_then(|x| x.parse::<usize>().ok()) {
        if k <= 3 { slot.0 = k; }
    }
    if let Some(a) = it.next() { slot.1 = a == "1"; }
}
