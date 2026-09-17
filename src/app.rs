//! HyperDrive - main application UI.
//! Layout (Explorer++ x Spacedrive hybrid):
//!   [ toolbar: nav | path-edit | filter | toggles ]
//!   [ optional directory tree sidebar ]
//!   [ single or dual file panes SIDE-BY-SIDE, each w/ path bar ]
//!   [ narrow status bar ]
//! All table cells and headers are custom-painted, strictly left-aligned.
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

use crate::audio::AudioPlayer;
use crate::browser::{self, drive_roots, DirSizes, Entry, History, SortKey};
use crate::config::{Settings, ThemeChoice};
use crate::theme;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};



fn is_audio_file(name: &str) -> bool {
    let n = name.to_lowercase();
    [".mp3", ".wav", ".ogg", ".flac", ".m4a", ".xm", ".s3m", ".mod", ".it", ".mptm"]
        .iter()
        .any(|ext| n.ends_with(ext))
}

/// Paint left-aligned text clipped to width w; adds ellipsis when truncated.
fn paint_cell(
    painter: &egui::Painter,
    x: f32,
    cy: f32,
    w: f32,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
) {
    let clip = egui::Rect::from_min_max(egui::pos2(x, cy - 2000.0), egui::pos2(x + w, cy + 2000.0));
    let cp = painter.with_clip_rect(clip);
    let max_w = w - 2.0;
    let chars: Vec<char> = text.chars().collect();
    let mut out = text.to_string();
    let probe = |t: &str| cp.layout_no_wrap(t.to_string(), font.clone(), color).size().x;

    if probe(&out) > max_w {
        let mut n = chars.len();
        let mut found = false;
        while n > 0 {
            let mut cand: String = chars[..n].iter().collect();
            cand.push('\u{2026}');
            if probe(&cand) <= max_w {
                out = cand;
                found = true;
                break;
            }
            n -= 1;
        }
        if !found {
            out = "\u{2026}".to_string();
        }
    }

    let galley = cp.layout_no_wrap(out, font, color);
    let gy = cy - galley.size().y / 2.0;
    cp.galley(egui::pos2(x, gy), galley, color);
}

fn load_subdirs(dir: &Path) -> Vec<Entry> {
    browser::list_dir(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.is_dir)
        .collect()
}

// ---------------------------------------------------------------- Pane

pub struct Pane {
    pub history: History,
    pub entries: Vec<Entry>,
    pub filtered: Vec<usize>,
    pub filter: String,
    pub sort_key: SortKey,
    pub sort_asc: bool,
    pub selected: HashSet<String>,
    pub path_edit: String,
    pub last_error: Option<String>,
    pub pending_audio: Option<PathBuf>,
    pub show_hidden: bool,
    pub renaming: Option<(String, String)>, // (old_name, buffer)
    pub err_ttl: u32,
    pub rename_done: Option<(String, String, bool)>, // (old, new, commit?)
    pub anchor_idx: Option<usize>,
    pub action_req: Option<RowAction>,
    pub open_in_new_tab: Option<PathBuf>,
    pub open_req: Option<PathBuf>,
    pub listing_rx: Option<std::sync::mpsc::Receiver<Result<Vec<Entry>, String>>>,
    pub rubber: Option<(egui::Pos2, egui::Pos2)>,
    pub rubber_sel: std::collections::HashSet<String>,
    pub row_rects: Vec<egui::Rect>,
    pub pane_list_rect: Option<egui::Rect>,
    pub press_origin: Option<(egui::Pos2,)>,
    pub pending_press_paths: Option<Vec<PathBuf>>,
    pub tag_filter: Option<String>,
}

impl Pane {
    pub fn new(start: PathBuf) -> Self {
        Self {
            history: History::with_start(start),
            entries: Vec::new(),
            filtered: Vec::new(),
            filter: String::new(),
            sort_key: SortKey::Name,
            sort_asc: true,
            selected: HashSet::new(),
            path_edit: String::new(),
            last_error: None,
            pending_audio: None,
            show_hidden: false,
            renaming: None,
            err_ttl: 0,
            rename_done: None,
            anchor_idx: None,
            action_req: None,
            open_in_new_tab: None,
            open_req: None,
            listing_rx: None,
            rubber: None,
            rubber_sel: Default::default(),
            row_rects: Vec::new(),
            pane_list_rect: None,
            press_origin: None,
            pending_press_paths: None,
            tag_filter: None,
        }
    }

    fn sync_path_edit(&mut self) {
        self.path_edit = self.history.current.display().to_string();
    }

    pub fn reload(&mut self) {
        // Async listing: keeps UI responsive on folders with 100k+ files.
        let dir = self.history.current.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let res = browser::list_dir(&dir)
                .map_err(|e| format!("{}: {}", dir.display(), e));
            let _ = tx.send(res);
        });
        self.listing_rx = Some(rx);
    }

    /// Poll finished background listings; returns true when new data arrived.
    pub fn poll_listing(&mut self) -> bool {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.listing_rx else { return false };
        match rx.try_recv() {
            Ok(Ok(mut list)) => {
                browser::sort_entries(&mut list, self.sort_key, self.sort_asc);
                self.entries = list;
                self.apply_sort();
                self.last_error = None;
                self.listing_rx = None;
                true
            }
            Ok(Err(e)) => {
                self.last_error = Some(e);
                self.err_ttl = 480;
                self.listing_rx = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.listing_rx = None;
                true
            }
        }
    }

    fn apply_sort(&mut self) {
        browser::sort_entries(&mut self.entries, self.sort_key, self.sort_asc);
        let f = self.filter.to_lowercase();
        let show_hidden = self.show_hidden;
        self.filtered.clear();
        let hidden_ok = |n: &str| show_hidden || !n.starts_with('.');
        let _ = hidden_ok;
        if f.is_empty() {
            self.filtered.extend(
                self.entries.iter().enumerate()
                    .filter(|(_, e)| hidden_ok(&e.name))
                    .map(|(i, _)| i),
            );
        } else {
            self.filtered.extend(
                self.entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.name.to_lowercase().contains(&f) && (show_hidden || !e.name.starts_with('.')))
                    .map(|(i, _)| i),
            );
        }
        // Tag filter
        if let Some(ref tf) = self.tag_filter.clone() {
            self.filtered.retain(|&i| {
                let _key = self.entries[i].path.display().to_string();
                self.entries[i].tag_color.as_deref() == Some(tf.as_str())
            });
        }
    }

    pub fn navigate(&mut self, path: PathBuf) {
        let target = std::fs::canonicalize(&path).unwrap_or(path);
        self.history.go(target);
        self.after_dir_change();
    }

    /// Back/Forward traversal - must NOT re-push history (bug in v0.3:
    /// toolbar called back() then navigate(), undoing the step).
    pub fn go_back(&mut self) {
        if let Some(prev) = self.history.back() {
            self.after_dir_change_to(prev);
        }
    }


    pub fn go_forward(&mut self) {
        if let Some(next) = self.history.forward() {
            self.after_dir_change_to(next);
        }
    }

    fn after_dir_change(&mut self) {
        self.filter.clear();
        self.selected.clear();
        self.reload();
        self.sync_path_edit();
    }

    fn after_dir_change_to(&mut self, dir: PathBuf) {
        // current already updated by History; just refresh UI state.
        let _ = dir;
        self.filter.clear();
        self.selected.clear();
        self.reload();
        self.sync_path_edit();
    }

    pub fn apply_sort_pub(&mut self) { self.apply_sort(); }

    pub fn toggle_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_key = key;
            self.sort_asc = true;
        }
        self.apply_sort();
    }

    /// Queue background size computation for all subdirectories of cwd.
    fn ensure_dir_sizes(&self, sizes: &DirSizes, ctx: &egui::Context) {
        let c = browser::egui_ctx::Ctx(ctx.clone());
        for e in &self.entries {
            if e.is_dir {
                sizes.request(&e.path, c.clone());
            }
        }
    }

    /// Draw one full file pane. Sets `activate` on interaction.
    /// Returns a navigation request (path to enter).
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        ui: &mut egui::Ui,
        row_h: f32,
        is_active: bool,
        activate: &mut bool,
        sizes: &DirSizes,
        audio_one_click: bool,
        want_focus_path: bool,
        grid_view: bool,
        _side_idx: usize,
        thumbs: &crate::thumbs::ThumbStore,
        tag_cache: &std::collections::HashMap<PathBuf, String>,
        thumb_tex: &mut std::collections::HashMap<PathBuf, egui::TextureHandle>,
        col_date_w: f32,
    ) -> Option<PathBuf> {
        if self.poll_listing() {
            ui.ctx().request_repaint();
        }
        self.row_rects.clear();
        let mut nav_request: Option<PathBuf> = None;
        let avail = ui.available_width();

        // Adaptive columns (all left-aligned, painted manually).
        let name_pad = 8.0_f32;
        let size_w = 90.0_f32;
        let date_w = col_date_w;
        let show_modified = avail > 330.0;
        let show_created = avail > 480.0 && show_modified;
        let used = size_w
            + if show_modified { date_w } else { 0.0 }
            + if show_created { date_w } else { 0.0 };
        let name_w = (avail - used - name_pad * 2.0).max(110.0);

        // --- pane header: active tint + editable path + filter ---
        let header_fill = if is_active {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().faint_bg_color
        };
        egui::Frame::none()
            .fill(header_fill)
            .inner_margin(egui::Margin::symmetric(5.0, 4.0))
            .show(ui, |ui| {
                ui.set_width(avail - 10.0);
                ui.horizontal(|ui| {
                    let path_id = egui::Id::new(format!("pane_path_{}", self as *const _ as usize));
                    let path_te = egui::TextEdit::singleline(&mut self.path_edit)
                        .font(egui::TextStyle::Small)
                        .id(path_id);
                    if want_focus_path {
                        ui.memory_mut(|m| m.request_focus(path_id));
                    }
                    let resp = ui.add(path_te);
                    if resp.gained_focus() || resp.changed() {
                        *activate = true;
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        nav_request = Some(PathBuf::from(self.path_edit.trim().to_string()));
                    }
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    let filt = ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("filter")
                            .font(egui::TextStyle::Small)
                            .desired_width(120.0),
                    );
                    if filt.changed() {
                        self.apply_sort();
                    }
                    if filt.gained_focus() {
                        *activate = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.small(format!(
                            "{} items \u{00B7} {} sel",
                            self.entries.len(),
                            self.selected.len()
                        ));
                    });
                });
            });
        ui.separator();

        // --- column headers (painted, left-aligned, clickable) ---
        let h = row_h * 0.8;
        let text_color = ui.visuals().text_color();
        let hover_tint = ui.visuals().faint_bg_color;
        let font = egui::FontId::proportional(h * 0.62);
        let top = ui.cursor().top();
        let x0 = ui.cursor().left() + name_pad / 2.0;

        let a_n = if self.sort_key == SortKey::Name { if self.sort_asc { " \u{2191}" } else { " \u{2193}" } } else { "" };
        let a_s = if self.sort_key == SortKey::Size { if self.sort_asc { " \u{2191}" } else { " \u{2193}" } } else { "" };
        let a_m = if self.sort_key == SortKey::Modified { if self.sort_asc { " \u{2191}" } else { " \u{2193}" } } else { "" };
        let a_c = if self.sort_key == SortKey::Created { if self.sort_asc { " \u{2191}" } else { " \u{2193}" } } else { "" };

        let cell = |ui: &mut egui::Ui, rx: f32, w: f32, label: String| -> bool {
            let rect = egui::Rect::from_min_size(egui::pos2(rx, top), egui::vec2(w, h));
            let resp = ui.allocate_rect(rect, egui::Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 2.0, hover_tint);
            }
            paint_cell(
                ui.painter(),
                rect.left() + 2.0,
                rect.center().y,
                w,
                &label,
                font.clone(),
                text_color,
            );
            resp.clicked()
        };

        let mut x = x0;
        if cell(ui, x, name_w, format!("{}{}", SortKey::Name.label(), a_n)) {
            self.toggle_sort(SortKey::Name);
            *activate = true;
        }
        x += name_w;
        if cell(ui, x, size_w, format!("{}{}", SortKey::Size.label(), a_s)) {
            self.toggle_sort(SortKey::Size);
            *activate = true;
        }
        x += size_w;
        if show_modified
            && cell(ui, x, date_w, format!("{}{}", SortKey::Modified.label(), a_m))
        {
            self.toggle_sort(SortKey::Modified);
            *activate = true;
        }
        x += if show_modified { date_w } else { 0.0 };
        if show_created
            && cell(ui, x, date_w, format!("{}{}", SortKey::Created.label(), a_c))
        {
            self.toggle_sort(SortKey::Created);
            *activate = true;
        }
        ui.advance_cursor_after_rect(egui::Rect::from_min_size(
            egui::pos2(ui.cursor().left(), top),
            egui::vec2(avail, h),
        ));
        ui.separator();

        // ================= GRID VIEW =================
        let mut grid_nav: Option<PathBuf> = None;
        if grid_view {
            let indices_g = self.filtered.clone();
            let thumbs_ref = thumbs;
            let texmap = &mut *thumb_tex;
            egui::ScrollArea::vertical().auto_shrink([false,false]).show(ui, |ui| {
                let cell = 112.0_f32;
                let cols = ((ui.available_width() - 8.0) / cell).floor().max(1.0) as usize;
                let mut k = 0usize;
                while k < indices_g.len() {
                    ui.horizontal(|ui| {
                        for _c in 0..cols {
                            if k >= indices_g.len() { break; }
                            let e = &self.entries[indices_g[k]];
                            // copy-out
                            let (nm, pth, isdir, sel) = (
                                e.name.clone(), e.path.clone(), e.is_dir,
                                self.selected.contains(&e.name));
                            let cellr = egui::Frame::none()
                                .fill(if sel { ui.visuals().selection.bg_fill }
                                      else { egui::Color32::TRANSPARENT })
                                .inner_margin(4.0)
                                .show(ui, |ui| {
                                    ui.set_min_size(egui::vec2(cell - 8.0, 108.0));
                                    ui.vertical(|ui| {
                                        // image area
                                        let mut drawn=false;
                                        if !isdir && crate::thumbs::ThumbStore::is_image(&pth) {
                                            let c3=ui.ctx().clone();
                                            thumbs_ref.request(pth.clone(), move || c3.request_repaint());
                                            if let Some(d)=thumbs_ref.get(&pth) {
                                                if !texmap.contains_key(&pth) {
                                                    let img=egui::ColorImage::from_rgba_unmultiplied(
                                                        [d.w as usize,d.h as usize],&d.rgba);
                                                    let t=ui.ctx().load_texture(
                                                        format!("gt_{}",pth.display()),img,
                                                        egui::TextureOptions::LINEAR);
                                                    texmap.insert(pth.clone(),t);
                                                }
                                                if let Some(t)=texmap.get(&pth){
                                                    let w=88.0_f32; let h=64.0_f32;
                                                    let sc=(w/d.w as f32).min(h/d.h as f32).min(1.0);
                                                    let dw=d.w as f32*sc; let dh=d.h as f32*sc;
                                                    let tid=t.id();
                                                    ui.image(egui::load::SizedTexture::new(tid,egui::vec2(dw,dh)));
                                                    drawn=true;
                                                }
                                            }
                                        }
                                        if !drawn {
                                            let ic=if isdir{"\u{1F4C1}"}else{"\u{1F4C4}"};
                                            ui.label(egui::RichText::new(ic).size(28.0));
                                        }
                                        let nm2 = if nm.len()>18 { format!("{}\u{2026}", &nm[..nm.char_indices().take(16).last().unwrap().0]) } else { nm.clone() };
                                        ui.small(&nm2);
                                    });
                                });
                            let resp = cellr.response
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            // interactions
                            if resp.clicked() || resp.secondary_clicked() { *activate = true; }
                            if resp.clicked() && !ui.input(|i| i.modifiers.ctrl) {
                                self.selected.clear();
                                self.selected.insert(nm.clone());
                                self.anchor_idx = Some(k);
                                if !isdir && audio_one_click && is_audio_file(&nm) {
                                    self.pending_audio = Some(pth.clone());
                                }
                            } else if resp.clicked() && ui.input(|i| i.modifiers.ctrl) {
                                if sel { self.selected.remove(&nm); } else { self.selected.insert(nm.clone()); }
                            }
                            if resp.double_clicked() {
                                if isdir { grid_nav = Some(pth.clone()); }
                                else if is_audio_file(&nm) { self.pending_audio = Some(pth.clone()); }
                                else { self.open_req = Some(pth.clone()); }
                            }
                            resp.context_menu(|ui| {
                                if ui.button("Open").clicked() {
                                    if isdir { grid_nav = Some(pth.clone()); }
                                    else if is_audio_file(&nm) { self.pending_audio = Some(pth.clone()); }
                                    else { self.open_req = Some(pth.clone()); }
                                    ui.close_menu();
                                }
                                if ui.button("Rename...\tF2").clicked() {
                                    self.renaming = Some((nm.clone(), nm.clone()));
                                    ui.close_menu();
                                }
                                if ui.button("Move to Trash\tDel").clicked() {
                                    self.action_req = Some(RowAction::Trash); ui.close_menu();
                                }
                            });
                            k += 1;
                        }
                    });
                }
            });
            return grid_nav;
        }

        // --- rows (virtualized, fully painted => exact left alignment) ---
        let list_top = ui.cursor().top();
        let indices = self.filtered.clone();
        let has_parent = self
            .history
            .current
            .parent()
            .map(|p| p != self.history.current)
            .unwrap_or(false);
        let parent_path = self.history.current.parent().map(|p| p.to_path_buf());
        let total_rows = indices.len() + if has_parent { 1 } else { 0 };
        let small_font = egui::FontId::proportional(row_h * 0.52);
        let main_font = egui::FontId::proportional(row_h * 0.58);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, total_rows, |ui, range| {
                let row_left = ui.cursor().left() + 4.0;
                let row_w = avail - 8.0;
                for idx in range {
                    // ---- virtual ".." parent row ----
                    if idx == 0 && has_parent {
                        let pp = match &parent_path {
                            Some(p) => p.clone(),
                            None => continue,
                        };
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(row_w, row_h),
                            egui::Sense::click(),
                        );
                        let p = ui.painter_at(rect);
                        if resp.hovered() {
                            p.rect_filled(rect.expand(1.0), 2.0, ui.visuals().faint_bg_color);
                        }
                        let txt_c = ui.visuals().text_color();
                        paint_cell(
                            &p,
                            row_left,
                            rect.center().y,
                            name_w,
                            "\u{1F4C1} ..",
                            main_font.clone(),
                            txt_c,
                        );
                        if resp.clicked() || resp.double_clicked() {
                            *activate = true;
                        }
                        if resp.clicked() || resp.double_clicked() {
                            nav_request = Some(pp);
                        }
                        continue;
                    }
                    let idx = if has_parent { idx - 1 } else { idx };
                    // Copy data out for the borrow checker.
                    let (name_txt, path, is_dir, size_s, mod_s, created_s, is_sel, name) = {
                        let e = &self.entries[indices[idx]];
                        (
                            e.name.clone(),
                            e.path.clone(),
                            e.is_dir,
                            if e.is_dir {
                                match sizes.get(&e.path) {
                                    Some(n) => browser::format_size(n),
                                    None if sizes.is_computing(&e.path) => "\u{2026}".to_string(),
                                    None => "<dir>".to_string(),
                                }
                            } else {
                                browser::format_size(e.size)
                            },
                            browser::format_time(e.modified),
                            browser::format_time(e.created),
                            self.selected.contains(&e.name),
                            e.name.clone(),
                        )
                    };

                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(row_w, row_h),
                        egui::Sense::click(),
                    );
                    self.row_rects.push(rect);
                    let p = ui.painter_at(rect);

                    if is_sel {
                        p.rect_filled(rect.expand(1.0), 2.0, ui.visuals().selection.bg_fill);
                    } else if resp.hovered() {
                        p.rect_filled(rect.expand(1.0), 2.0, ui.visuals().faint_bg_color);
                    }

                    let icon = if is_dir { "\u{1F4C1}" } else { "\u{1F4C4}" };
                    // Thumbnails: queue decode + draw when ready
                    let mut thumb_drawn = false;
                    if !is_dir && crate::thumbs::ThumbStore::is_image(&path) {
                        let ctx2 = ui.ctx().clone();
                        thumbs.request(path.clone(), move || ctx2.request_repaint());
                        if let Some(d) = thumbs.get(&path) {
                            if !thumb_tex.contains_key(&path) {
                                let img = egui::ColorImage::from_rgba_unmultiplied(
                                    [d.w as usize, d.h as usize],
                                    &d.rgba);
                                let tex = ui.ctx().load_texture(
                                    format!("thumb_{}", path.display()),
                                    img,
                                    egui::TextureOptions::LINEAR);
                                thumb_tex.insert(path.clone(), tex);
                            }
                            if let Some(tex) = thumb_tex.get(&path) {
                                let tsz = 18.0_f32;
                                let r = egui::Rect::from_min_size(
                                    egui::pos2(row_left, rect.center().y - tsz / 2.0),
                                    egui::vec2(tsz, tsz));
                                p.image(tex.id(), r, egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE);
                                thumb_drawn = true;
                            }
                        }
                    }
                    let mut name_off = if thumb_drawn { 22.0 } else { 0.0 };
                    if let Some(hex) = tag_cache.get(&path) {
                        if let Ok(c) = egui::Color32::from_hex(hex) {
                            p.circle_filled(
                                egui::pos2(row_left + name_off + 6.0, rect.center().y),
                                4.0, c);
                            name_off += 12.0;
                        }
                    }
                    let txt_c = ui.visuals().text_color();

                    let cy = rect.center().y;
                    let mut cx = row_left;
                    let renaming_here = self
                        .renaming
                        .as_ref()
                        .map(|(o, _)| o == &name)
                        .unwrap_or(false);
                    if renaming_here {
                        let mut buf = self.renaming.as_ref().unwrap().1.clone();
                        let rid = egui::Id::new(format!("rename_{}_{}", idx, name));
                        let te = egui::TextEdit::singleline(&mut buf)
                            .font(main_font.clone())
                            .id(rid)
                            .desired_width(name_w - 6.0);
                        ui.memory_mut(|m| m.request_focus(rid));
                        let r2 = ui.allocate_ui_at_rect(
                            egui::Rect::from_min_size(
                                egui::pos2(cx, rect.top()),
                                egui::vec2(name_w, row_h),
                            ),
                            |ui| ui.add(te),
                        );
                        let rf = r2.inner;
                        let enter = rf.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let esc = rf.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape));
                        let clicked_away = ui.input(|i| i.pointer.primary_clicked())
                            && !r2.response.hovered();
                        if enter || esc || clicked_away {
                            self.rename_done =
                                Some((name.clone(), buf.trim().to_string(), enter || clicked_away));
                            self.renaming = None;
                        } else {
                            self.renaming = Some((name.clone(), buf));
                        }
                    } else {
                        paint_cell(&p, cx + name_off, cy, name_w - name_off, &format!("{icon} {name_txt}"), main_font.clone(), txt_c);
                    }
                    cx += name_w;
                    paint_cell(&p, cx, cy, size_w, &size_s, small_font.clone(), txt_c.gamma_multiply(0.85));
                    cx += size_w;
                    if show_modified {
                        paint_cell(&p, cx, cy, date_w, &mod_s, small_font.clone(), txt_c.gamma_multiply(0.85));
                        cx += date_w;
                    }
                    if show_created {
                        paint_cell(&p, cx, cy, date_w, &created_s, small_font.clone(), txt_c.gamma_multiply(0.75));
                    }

                    // Rubber-band intersection marking
                    if let Some((a, b)) = &self.rubber {
                        let rb = egui::Rect::from_min_max((*a).min(*b), (*a).max(*b));
                        if rb.intersects(rect) {
                            self.rubber_sel.insert(name.clone());
                        }
                    }

                    // Interaction
                    if resp.clicked() || resp.secondary_clicked() {
                        *activate = true;
                    }
                    if resp.secondary_clicked() && !is_sel {
                        self.selected.clear();
                        self.selected.insert(name.clone());
                    }
                    resp.context_menu(|ui| {
                        let sel_any = !self.selected.is_empty();
                        let w = egui::vec2(160.0, 0.0);
                        if ui.add_enabled(sel_any, egui::Button::new("Copy\tCtrl+C").min_size(w)).clicked() {
                            self.action_req = Some(RowAction::Copy); ui.close_menu();
                        }
                        if ui.add_enabled(sel_any, egui::Button::new("Cut\tCtrl+X").min_size(w)).clicked() {
                            self.action_req = Some(RowAction::Cut); ui.close_menu();
                        }
                        if ui.add_enabled(sel_any, egui::Button::new("Rename...\tF2").min_size(w)).clicked() {
                            self.action_req = Some(RowAction::Rename); ui.close_menu();
                        }
                        if ui.add_enabled(sel_any, egui::Button::new("Duplicate\tCtrl+D").min_size(w)).clicked() {
                            self.action_req = Some(RowAction::Duplicate); ui.close_menu();
                        }
                        if ui.add_enabled(sel_any, egui::Button::new("Create Symlink").min_size(w)).clicked() {
                            self.action_req = Some(RowAction::Symlink); ui.close_menu();
                        }
                        if name.to_lowercase().ends_with(".zip")
                            && ui.add_enabled(true, egui::Button::new("Browse archive").min_size(w)).clicked()
                        {
                            nav_request = Some(PathBuf::from(format!("{}!", path.display())));
                            ui.close_menu();
                        }
                        if ui.add_enabled(sel_any, egui::Button::new("Open With...").min_size(w)).clicked() {
                            self.action_req = Some(RowAction::OpenWith); ui.close_menu();
                        }
                        let is_archive = [".zip",".7z",".tar",".tar.gz",".tgz",".tar.xz",".tar.bz2"]
                            .iter()
                            .any(|e| name.to_lowercase().ends_with(e));
                        if is_archive && ui.add_enabled(true,
                            egui::Button::new("Extract here...").min_size(w)).clicked() {
                            self.action_req = Some(RowAction::Extract(path.clone())); ui.close_menu();
                        }
                        ui.menu_button("\u{1F3F7} Tag", |ui| {
                            for (name, hex) in crate::tags::TAG_COLORS {
                                if ui.button(name).clicked() {
                                    self.action_req = Some(RowAction::Tag(hex.to_string()));
                                    ui.close_menu();
                                }
                            }
                        });
                        ui.separator();
                        if ui.add_enabled(sel_any, egui::Button::new("Move to Trash\tDel").min_size(w)).clicked() {
                            self.action_req = Some(RowAction::Trash); ui.close_menu();
                        }
                        if ui.add_enabled(sel_any, egui::Button::new(
                            egui::RichText::new("Delete Permanently...\tShift+Del")
                                .color(egui::Color32::from_rgb(0xE8,0x5D,0x75))).min_size(w)).clicked()
                        {
                            self.action_req = Some(RowAction::DeletePermanent); ui.close_menu();
                        }
                        ui.separator();
                        if ui.add_enabled(sel_any, egui::Button::new("Properties\tAlt+Enter").min_size(w)).clicked() {
                            self.action_req = Some(RowAction::Props); ui.close_menu();
                        }
                    });
                    let ctrl = ui.input(|i| i.modifiers.ctrl);
                    let shift = ui.input(|i| i.modifiers.shift);
                    if resp.clicked() && ctrl {
                        if is_sel { self.selected.remove(&name); } else { self.selected.insert(name.clone()); }
                    } else if resp.clicked() && shift {
                        let anchor = self.anchor_idx.unwrap_or(idx);
                        let (lo, hi) = (anchor.min(idx), anchor.max(idx));
                        for &k in &indices[lo..=hi] {
                            if let Some(e) = self.entries.get(k) {
                                self.selected.insert(e.name.clone());
                            }
                        }
                    } else if resp.clicked() {
                        self.selected.clear();
                        self.selected.insert(name.clone());
                        self.anchor_idx = Some(idx);
                        if audio_one_click && is_audio_file(&name_txt) {
                            self.pending_audio = Some(path.clone());
                        }
                    }
                    if resp.middle_clicked() && is_dir {
                        self.open_in_new_tab = Some(path.clone());
                    }
                    let is_zip_row = name.to_lowercase().ends_with(".zip");
                    if resp.double_clicked() && is_zip_row {
                        nav_request = Some(PathBuf::from(format!("{}!", path.display())));
                    }
                    if resp.double_clicked() {
                        if is_dir {
                            nav_request = Some(path.clone());
                        } else if is_audio_file(&name_txt) {
                            // Built-in player for media; external for the rest.
                            self.pending_audio = Some(path.clone());
                        } else {
                            // handled by App to respect default-app memory
                            self.open_req = Some(path.clone());
                        }
                    }
                }
            });

        // ---- rubber band lifecycle + overlay ----
        let list_bottom = ui.cursor().top();
        let list_rect = egui::Rect::from_min_max(
            egui::pos2(ui.cursor().left(), list_top),
            egui::pos2(ui.cursor().left() + avail, list_bottom.max(list_top + 40.0)),
        );
        self.pane_list_rect = Some(list_rect);
        let ptr = ui.input(|i| i.pointer.clone());
        // Start rubber-band ONLY on presses that did not land on a row.
        if self.rubber.is_none() && ptr.primary_pressed() {
            if let Some(p0) = ptr.interact_pos() {
                if list_rect.contains(p0)
                    && !self.row_rects.iter().any(|r| r.contains(p0))
                {
                    self.rubber = Some((p0, p0));
                    self.rubber_sel.clear();
                    self.selected.clear();
                }
            }
        }
        if let Some(rb) = &mut self.rubber {
            if let Some(cur) = ptr.latest_pos() {
                rb.1 = cur;
            }
            ui.ctx().request_repaint();
            if ptr.primary_released() {
                self.selected.extend(self.rubber_sel.drain());
                self.rubber = None;
            } else {
                let r2 = egui::Rect::from_min_max(rb.0.min(rb.1), rb.0.max(rb.1));
                ui.painter().rect_filled(r2, 0.5,
                    ui.visuals().selection.bg_fill.gamma_multiply(0.35));
                ui.painter().rect_stroke(r2, 0.5,
                    egui::Stroke::new(1.0_f32, ui.visuals().selection.stroke.color));
            }
        }

        nav_request
    }
}

// ---------------------------------------------------------------- Tree

#[derive(Default, Clone)]
struct TreeNode {
    expanded: bool,
    loaded: bool,
    children: Vec<Entry>,
}

pub struct DirTree {
    nodes: HashMap<PathBuf, TreeNode>,
    expanded_for: Option<PathBuf>,
}

impl DirTree {
    fn new() -> Self {
        Self { nodes: HashMap::new(), expanded_for: None }
    }

    /// Mark every ancestor of `dir` expanded (auto-reveal current folder).
    /// Change-guarded by `expanded_for` so a user collapsing a branch is not
    /// fought every frame; only re-reveals when the current dir actually moves.
    fn reveal(&mut self, dir: &Path) {
        if self.expanded_for.as_deref() == Some(dir) {
            return;
        }
        self.expanded_for = Some(dir.to_path_buf());
        let mut cur = Some(dir.to_path_buf());
        while let Some(anc) = cur {
            let parent = anc.parent().map(|p| p.to_path_buf());
            let is_root = anc == Path::new("/");
            if let Some(par) = &parent {
                let n = self.nodes.entry(par.clone()).or_default();
                n.expanded = true;
                if !n.loaded {
                    n.children = load_subdirs(par);
                    n.loaded = true;
                }
            }
            if is_root { break; }
            cur = parent;
        }
    }

    fn draw(
        &mut self,
        ui: &mut egui::Ui,
        dir: &PathBuf,
        label: &str,
        depth: u8,
        current: &PathBuf,
    ) -> Option<PathBuf> {
        if depth > 6 {
            return None;
        }
        let is_open_path = current.starts_with(dir);
        let tri = {
            match self.nodes.get(dir) {
                Some(n) if n.expanded => "-",
                _ => "+",
            }
        };

        let mut toggle_req = false;
        let mut click_req = false;
        ui.horizontal(|ui| {
            ui.add_space((depth as f32) * 14.0);
            if ui
                .add(egui::Button::new(egui::RichText::new(tri).small()).frame(false))
                .clicked()
            {
                toggle_req = true;
            }
            let lbl_text = if is_open_path {
                egui::RichText::new(label).small().strong()
            } else {
                egui::RichText::new(label).small()
            };
            if ui
                .add(egui::Label::new(lbl_text).sense(egui::Sense::click()))
                .clicked()
            {
                click_req = true;
            }
        });

        if toggle_req {
            let n = self.nodes.entry(dir.clone()).or_default();
            n.expanded = !n.expanded;
            if n.expanded && !n.loaded {
                n.children = load_subdirs(dir);
                n.loaded = true;
            }
        }
        if click_req {
            return Some(dir.clone());
        }

        let mut nav = None;
        if self.nodes.get(dir).map(|n| n.expanded).unwrap_or(false) {
            let kids = self.nodes.get(dir).map(|n| n.children.clone()).unwrap_or_default();
            for k in kids {
                let lbl = k.name.clone();
                let p = k.path.clone();
                if let Some(n) = self.draw(ui, &p, &lbl, depth + 1, current) {
                    nav = Some(n);
                }
            }
        }
        nav
    }
}

// ---------------------------------------------------------------- App

pub struct HyperDriveApp {
    pub settings: Settings,
    pub panes: [Pane; 2],
    pub show_settings: bool,
    pub tree: DirTree,
    pub dir_sizes: DirSizes,
    pub audio: AudioPlayer,
    pub volume: f32,
    pub dialog: Dialog,
    pub clip_paths: Vec<PathBuf>,
    pub clip_cut: bool,
    pub focus_path_req: bool,
    pub egui_ctx_for_props: Option<egui::Context>,
    pub search: Option<crate::app_search::SearchState>,
    pub thumbs: crate::thumbs::ThumbStore,
    pub thumb_tex: std::collections::HashMap<PathBuf, egui::TextureHandle>,
    pub previews: crate::thumbs::ThumbStore,
    pub jobman: crate::jobs::JobManager,
    pub show_jobs: bool,
    pub tagstore: crate::tags::TagStore,
    pub tag_cache: std::collections::HashMap<PathBuf, String>,
    pub preview_tex: Option<(PathBuf, egui::TextureHandle)>,
    /// Per-side tab stacks (each tab remembers its directory).
    pub side_tabs: [Vec<PathBuf>; 2],
    pub pane_sort_seeded: bool,
    pub dnd: Option<(usize, Vec<PathBuf>, egui::Pos2)>, // from_pane, paths, origin
    pub dnd_hover_dst: Option<usize>,
    pub tab_rects: [Vec<(usize, egui::Rect)>; 2],
    pub dnd_tab: Option<(usize, usize, f32)>, // side, fromIdx, start_x
}


#[derive(Debug, Clone)]
pub enum RowAction {
    Copy,
    Cut,
    Rename,
    Duplicate,
    Symlink,
    Trash,
    DeletePermanent,
    Props,
    OpenWith,
    Tag(String),
    Extract(PathBuf),
}

pub enum Dialog {
    None,
    NewFolder(String),
    SmbConnect {
        server: String,
        share: String,
        user: String,
        domain: String,
        guest: bool,
    },
    ShareFolder {
        path: String,
        name: String,
        readonly: bool,
        guest: bool,
        comment: String,
    },
    OpenWith {
        path: String,
        filter: String,
        remember: bool,
    },
    MultiRename {
        pattern_find: String,
        pattern_repl: String,
        numbered_start: i64,
    },
    ConfirmDelete {
        names: Vec<String>,
    },
    SmbBrowse2 {
        host: String,
        share: String,
        path: String,
        entries: Vec<(String, bool, u64)>,
        status: String,
        newfolder: String,
        selected: Option<(String, bool)>,
    },
    SftpConnect { server: String, user: String },
    SmbBrowse {
        entries: Vec<(String, String)>, // (host, ip)
        picked: Option<String>,
    },
    Properties {
        name: String,
        path: String,
        is_dir: bool,
        size_s: String,
        modified_s: String,
        created_s: String,
        perms_s: String,
        readonly_fs: bool,
        items: Option<(u64, u64)>, // (files, dirs) for folders, when known
    },
}

impl HyperDriveApp {
    pub fn new(settings: Settings) -> Self {
        let home = browser::home_dir();
        let p0 = {
            let c = settings.pane_paths[0].clone();
            if c.is_empty() || !PathBuf::from(&c).is_dir() { home.clone() } else { PathBuf::from(c) }
        };
        let p1 = {
            let c = settings.pane_paths[1].clone();
            if c.is_empty() || !PathBuf::from(&c).is_dir() { home.clone() } else { PathBuf::from(c) }
        };
        let _ = home;
        let mut this = Self {
            panes: [Pane::new(p0.clone()), Pane::new(p1.clone())],
            pane_sort_seeded: false,
            dnd: None,
            dnd_hover_dst: None,
            tab_rects: [Vec::new(), Vec::new()],
            dnd_tab: None,
            side_tabs: {
                let t0: Vec<PathBuf> = settings.side_tabs[0]
                    .iter()
                    .filter(|x| PathBuf::from(x).is_dir())
                    .map(PathBuf::from)
                    .collect();
                let t1: Vec<PathBuf> = settings.side_tabs[1]
                    .iter()
                    .filter(|x| PathBuf::from(x).is_dir())
                    .map(PathBuf::from)
                    .collect();
                [
                    if t0.is_empty() { vec![p0.clone()] } else { t0 },
                    if t1.is_empty() { vec![p1.clone()] } else { t1 },
                ]
            },
            settings,
            show_settings: false,
            tree: DirTree::new(),
            dir_sizes: DirSizes::new(),
            audio: AudioPlayer::new(),
            volume: 0.8,
            dialog: Dialog::None,
            clip_paths: Vec::new(),
            clip_cut: false,
            focus_path_req: false,
            egui_ctx_for_props: None,
            search: None,
            thumbs: crate::thumbs::ThumbStore::new(),
            thumb_tex: std::collections::HashMap::new(),
            previews: crate::thumbs::ThumbStore::with_size(360),
            preview_tex: None,
            jobman: crate::jobs::JobManager::new(),
            show_jobs: true,
            tagstore: crate::tags::TagStore::open(),
            tag_cache: Default::default(),
        };
        this.tag_cache = this.tagstore.all();
        this
    }

    fn persist_ui_state(&mut self) {
        if let Err(e) = self.settings.save() {
            self.panes[0].last_error = Some(format!("config save failed: {e}"));
        }
    }

    fn active_pane(&mut self) -> &mut Pane {
        let idx = self.settings.active_pane.min(1);
        &mut self.panes[idx]
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        let _panel = egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                let bh = ui.spacing().interact_size.y.max(24.0);
                // TEXT buttons only - glyph fonts proved unreliable on target system.
                let mk_btn = |ui: &mut egui::Ui, txt: &str, tip: &str| -> bool {
                    let clicked = ui
                        .add(egui::Button::new(txt).min_size(egui::vec2(0.0, bh)))
                        .on_hover_text(tip)
                        .clicked();
                    clicked
                };
                let a = self.active_pane();
                if mk_btn(ui, "Back", "Go back") {
                    a.go_back();
                }
                if mk_btn(ui, "Fwd", "Go forward") {
                    a.go_forward();
                }
                if mk_btn(ui, "Up", "Up one level") {
                    if let Some(parent) = a.history.current.parent().map(|p| p.to_path_buf()) {
                        a.navigate(parent);
                    }
                }
                if mk_btn(ui, "Home", "Home folder") {
                    let h = browser::home_dir();
                    a.navigate(h);
                }
                if mk_btn(ui, "Reload", "Refresh listing") {
                    a.reload();
                }
                ui.separator();
                if mk_btn(ui, "Tree", "Toggle folder tree panel") {
                    self.settings.show_tree = !self.settings.show_tree;
                    self.persist_ui_state();
                }
                if mk_btn(ui, "Dual pane", "Toggle second file pane") {
                    self.settings.dual_pane = !self.settings.dual_pane;
                    self.persist_ui_state();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if mk_btn(ui, "Settings", "Theme, fonts") {
                        self.show_settings = !self.show_settings;
                    }
                });
            });
            ui.add_space(3.0);
        });
    }

    fn tree_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tree_panel")
            .resizable(true)
            .default_width(215.0)
            .min_width(150.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let active_idx = self.settings.active_pane.min(1);
                        let current = self.panes[active_idx].history.current.clone();
                        self.tree.reveal(&current);

                        // ---- BOOKMARKS ----
                        ui.label(egui::RichText::new(" BOOKMARKS").strong().small());
                        ui.separator();
                        if self.settings.bookmarks.is_empty() {
                            ui.label(egui::RichText::new("  none - use Bookmarks menu").weak().small());
                        }
                        for b in self.settings.bookmarks.clone() {
                            let name = std::path::Path::new(&b)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| b.clone());
                            if ui
                                .selectable_label(false, egui::RichText::new(format!("\u{2605} {name}")).small())
                                .clicked()
                            {
                                let a = self.active_pane();
                                a.navigate(PathBuf::from(&b));
                            }
                        }
                        ui.add_space(4.0);

                        // ---- DEVICES ----
                        ui.label(egui::RichText::new(" DEVICES").strong().small());
                        ui.separator();
                        let roots = drive_roots();
                        for (label, path) in roots {
                            if let Some(nav) = self.tree.draw(ui, &path, &label, 0, &current) {
                                let a = self.active_pane();
                                a.navigate(nav);
                            }
                        }
                        ui.add_space(4.0);

                        // ---- NETWORK ----
                        ui.label(egui::RichText::new(" NETWORK").strong().small());
                        ui.separator();
                        if ui
                            .selectable_label(false, egui::RichText::new("\u{1F5A7} Connect to Windows share...").small())
                            .clicked()
                        {
                            self.dialog = Dialog::SmbConnect {
                                server: "192.168.1.".into(),
                                share: String::new(),
                                user: std::env::var("USER").unwrap_or_default(),
                                domain: "WORKGROUP".into(),
                                guest: false,
                            };
                        }
                        if ui
                            .selectable_label(false, egui::RichText::new("\u{1F5A5} Browse SMB share (smbclient)...").small())
                            .clicked()
                        {
                            self.dialog = Dialog::SmbBrowse2 {
                                host: "192.168.1.10".into(),
                                share: String::new(),
                                path: String::new(),
                                entries: Vec::new(),
                                status: "enter share name".into(),
                                newfolder: String::new(),
                                selected: None,
                            };
                        }
                        if ui
                            .selectable_label(false, egui::RichText::new("\u{2605} Connect to SFTP/SSH...").small())
                            .clicked()
                        {
                            self.dialog = Dialog::SftpConnect {
                                server: String::new(),
                                user: std::env::var("USER").unwrap_or_default(),
                            };
                        }
                        if ui
                            .selectable_label(false, egui::RichText::new("\u{1F50E} Browse SMB servers...").small())
                            .clicked()
                        {
                            self.dialog = Dialog::SmbBrowse {
                                entries: scan_smb_hosts(),
                                picked: None,
                            };
                        }
                        if ui
                            .selectable_label(false, egui::RichText::new("\u{1F4E4} Share this folder...").small())
                            .clicked()
                        {
                            let cur = self.active_pane().history.current.display().to_string();
                            let name = std::path::Path::new(&cur)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "share".to_string());
                            self.dialog = Dialog::ShareFolder {
                                path: cur,
                                name,
                                readonly: true,
                                guest: false,
                                comment: "Shared by HyperDrive".into(),
                            };
                        }
                        if !self.settings.net_locations.is_empty() {
                            ui.add_space(2.0);
                            for (label, url) in self.settings.net_locations.clone() {
                                ui.horizontal(|ui| {
                                    if ui
                                        .selectable_label(
                                            false,
                                            egui::RichText::new(format!("\u{1F310} {label}")).small(),
                                        )
                                        .clicked()
                                    {
                                        self.mount_and_open_smb(&url);
                                    }
                                    if ui.small_button("x").clicked() {
                                        self.settings.net_locations.retain(|(_, u)| u != &url);
                                        let _ = self.settings.save();
                                    }
                                });
                            }
                        }
                        ui.add_space(6.0);

                        // ---- TAGS ----
                        ui.label(egui::RichText::new(" TAGS").strong().small());
                        ui.separator();
                        let counts = self.tagstore.count_by_color();
                        let active_tag = self.settings.tag_filter.clone();
                        for (label, hex) in crate::tags::TAG_COLORS {
                            let cnt = if hex.is_empty() { 0 } else { counts.get(hex).copied().unwrap_or(0) };
                            let is_active = active_tag.as_deref() == Some(hex) && !hex.is_empty();
                            let txt = if hex.is_empty() {
                                egui::RichText::new("  (none)").weak().small()
                            } else {
                                egui::RichText::new(format!("  \u{25CF} {} ({})", label, cnt)).small()
                            };
                            if ui.selectable_label(is_active, txt).clicked() {
                                self.settings.tag_filter = if hex.is_empty() { None } else { Some(hex.to_string()) };
                                for p in &mut self.panes { p.tag_filter = self.settings.tag_filter.clone(); }
                                for p in &mut self.panes { p.apply_sort(); }
                            }
                        }
                        ui.add_space(6.0);
                    });
            });
    }

    /// Mount an smb:// URL via GVFS (no root needed) and open the gvfs path.
    fn mount_and_open_smb(&mut self, url: &str) {
        self.mount_and_open_smb_inner(url, false);
        let auth_failed = self.panes[self.settings.active_pane.min(1)]
            .last_error
            .as_ref()
            .map(|e| {
                e.contains("Authentication") || e.contains("Permission denied")
                    || e.contains("Logon failure")
            })
            .unwrap_or(false);
        if auth_failed {
            self.panes[self.settings.active_pane.min(1)].last_error = None;
            self.mount_and_open_smb_inner(url, true);
        }
        self.locate_gvfs_mount(url);
    }

    fn mount_and_open_smb_inner(&mut self, url: &str, anonymous: bool) {
        // timeout wrapper: a hidden auth prompt must never freeze the UI.
        let mut cmd = std::process::Command::new("timeout");
        cmd.arg("20s").arg("gio");
        if anonymous {
            cmd.arg("--anonymous");
        }
        cmd.arg("mount").arg(url);
        cmd.stdin(std::process::Stdio::null());
        match cmd.output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                let msg = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if !msg.is_empty()
                    && !msg.contains("already mounted")
                    && !msg.contains("No such file or directory")
                {
                    self.panes[self.settings.active_pane.min(1)].last_error =
                        Some(format!("mount failed: {msg}"));
                }
            }
            Err(e) => {
                self.panes[self.settings.active_pane.min(1)].last_error =
                    Some(format!("gio: {e}"));
            }
        }
    }

    /// Find the mounted share under /run/user/$UID/gvfs and navigate into it.
    fn locate_gvfs_mount(&mut self, url: &str) {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let uid = libc_like_uid();
        let gvfs = PathBuf::from(format!("/run/user/{uid}/gvfs"));
        let rest = url.trim_start_matches("smb://");
        let want_server = rest.split('/').next().unwrap_or("").to_lowercase();
        let want_share = rest.split('/').nth(1).unwrap_or("").trim_end_matches('/').to_lowercase();
        if let Ok(rd) = std::fs::read_dir(&gvfs) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_lowercase();
                if n.starts_with("smb-share:")
                    && n.contains(&want_server)
                    && (want_share.is_empty() || n.contains(&format!("share={}", want_share)))
                {
                    self.active_pane().navigate(e.path());
                    return;
                }
            }
        }
        self.panes[self.settings.active_pane.min(1)].last_error =
            Some("Could not find mount under /run/user/$UID/gvfs".into());
        self.panes[self.settings.active_pane.min(1)].err_ttl = 480;
    }

    fn central(&mut self, ctx: &egui::Context) {
        let row_h = (self.settings.font_size * 1.55).max(22.0);

        egui::CentralPanel::default().show(ctx, move |ui| {
            self.draw_tab_strip(ui, 0);
            self.panes[0].ensure_dir_sizes(&self.dir_sizes, ctx);
            let mut activate = false;
            if let Some(nav) = self.panes[0].draw(
                ui,
                row_h,
                self.settings.active_pane == 0,
                &mut activate,
                &self.dir_sizes,
                self.settings.audio_one_click,
                self.focus_path_req && self.settings.active_pane == 0,
                self.settings.grid_view && self.settings.active_pane == 0 || (self.settings.grid_view && !self.settings.dual_pane),
                0,
                &self.thumbs,
                &self.tag_cache,
                &mut self.thumb_tex,
                self.settings.col_date_w,
            ) {
                self.panes[0].navigate(nav);
            }
            self.apply_rename_result(0);
            self.apply_row_actions(0);
            if let Some(p2) = self.panes[0].open_in_new_tab.take() {
                self.side_tabs[0].push(p2.clone());
                self.panes[0].navigate(p2);
            }
            if let Some(pf) = self.panes[0].open_req.take() { self.smart_open(&pf); }
            if let Some(p) = self.panes[0].pending_audio.take() {
                if let Err(e) = self.audio.play(&p) {
                    self.panes[0].last_error = Some(e);
                }
            }
            let clicked_here = ctx.input(|i| i.pointer.primary_clicked())
                && ui.rect_contains_pointer(ui.max_rect());
            if activate || clicked_here {
                self.settings.active_pane = 0;
            }
        });
    }

    /// Dual-pane mode: pane 2 lives in its own resizable right panel,
    /// mirroring how the (working) tree sidebar is built.
    fn right_pane_panel(&mut self, ctx: &egui::Context) {
        let row_h = (self.settings.font_size * 1.55).max(22.0);
        egui::SidePanel::right("dual_right_pane")
            .resizable(true)
            .default_width(430.0)
            .min_width(230.0)
            .show(ctx, move |ui| {
                self.draw_tab_strip(ui, 1);
                self.panes[1].ensure_dir_sizes(&self.dir_sizes, ctx);
                let mut activate = false;
                if let Some(nav) = self.panes[1].draw(
                    ui,
                    row_h,
                    self.settings.active_pane == 1,
                    &mut activate,
                    &self.dir_sizes,
                    self.settings.audio_one_click,
                    self.focus_path_req && self.settings.active_pane == 1,
                    self.settings.grid_view && self.settings.active_pane == 1,
                    1,
                    &self.thumbs,
                    &self.tag_cache,
                    &mut self.thumb_tex,
                    self.settings.col_date_w,
                ) {
                    self.panes[1].navigate(nav);
                }
                self.apply_rename_result(1);
                self.apply_row_actions(1);
                if let Some(p2) = self.panes[1].open_in_new_tab.take() {
                    self.side_tabs[1].push(p2.clone());
                    self.panes[1].navigate(p2);
                }
                if let Some(pf) = self.panes[1].open_req.take() { self.smart_open(&pf); }
                if let Some(p) = self.panes[1].pending_audio.take() {
                    if let Err(e) = self.audio.play(&p) {
                        self.panes[1].last_error = Some(e);
                    }
                }
                let clicked_here = ctx.input(|i| i.pointer.primary_clicked())
                    && ui.rect_contains_pointer(ui.max_rect());
                if activate || clicked_here {
                    self.settings.active_pane = 1;
                }
            });
    }

    fn statusbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("statusbar").show(ctx, |ui| {
            ui.add_space(1.0);
            ui.horizontal(|ui| {
                // TTL display: keep message visible ~8s instead of one frame.
                for pn in self.panes.iter_mut() {
                    if pn.last_error.is_some() && pn.err_ttl == 0 { pn.err_ttl = 480; }
                    if pn.err_ttl > 0 { pn.err_ttl -= 1; if pn.err_ttl == 0 { pn.last_error = None; } }
                }
                let shown = self.panes.iter_mut()
                    .find(|pn| pn.err_ttl > 0 && pn.last_error.is_some())
                    .and_then(|pn| pn.last_error.take());
                if let Some(err) = shown {
                    ui.colored_label(
                        egui::Color32::from_rgb(0xE8, 0x5D, 0x75),
                        egui::RichText::new(err).small(),
                    );
                } else {
                    if self.panes[0].listing_rx.is_some() || self.panes[1].listing_rx.is_some() {
                        ui.spinner();
                        ui.small("loading folder...");
                    }
                    let p = &self.panes[self.settings.active_pane.min(1)];
                    ui.label(
                        egui::RichText::new(format!(
                            "{} items \u{00B7} {} shown \u{00B7} {} selected\u{00B7} disk free: {}",
                            p.entries.len(),
                            p.filtered.len(),
                            p.selected.len(),
                            free_disk_gib(&p.history.current)
                        ))
                        .small(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let tag = if self.settings.dual_pane {
                        format!("pane {}", self.settings.active_pane + 1)
                    } else {
                        String::new()
                    };
                    ui.small(tag);
                });
            });
            ui.add_space(1.0);
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut changed_any = false;
        let mut ok_clicked = false;
        let old = self.settings.clone();

        let mut settings_open = self.show_settings;
        egui::Window::new("Settings")
            .open(&mut settings_open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.heading("Theme");
                ui.horizontal(|ui| {
                    for t in ThemeChoice::ALL {
                        if ui.radio_value(&mut self.settings.theme, t, t.label()).changed() {
                            changed_any = true;
                        }
                    }
                });
                ui.separator();
                ui.heading("Files");
                if ui
                    .checkbox(
                        &mut self.settings.audio_one_click,
                        "One click plays audio files (mp3 / wav / ogg / flac)",
                    )
                    .changed()
                {
                    changed_any = true;
                }
                ui.separator();
                ui.heading("Text");
                if ui
                    .add(egui::Slider::new(&mut self.settings.font_size, 12.0..=28.0).text("Font size"))
                    .changed()
                {
                    changed_any = true;
                }
                let families: Vec<String> = std::iter::once("(App default)".to_string())
                    .chain(
                        theme::curated_fonts()
                            .iter()
                            .filter(|(_, paths)| paths.iter().any(|p| std::path::Path::new(p).is_file()))
                            .map(|(n, _)| n.to_string()),
                    )
                    .collect();
                let current = if self.settings.font_family.is_empty() {
                    "(App default)".to_string()
                } else {
                    self.settings.font_family.clone()
                };
                egui::ComboBox::from_id_source("hd_font_family")
                    .selected_text(current)
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for fam in &families {
                            let value = if fam == "(App default)" { "" } else { fam.as_str() };
                            if ui
                                .selectable_value(&mut self.settings.font_family, value.to_string(), fam)
                                .changed()
                            {
                                changed_any = true;
                            }
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        theme::apply_all(ctx, &self.settings);
                        let _ = self.settings.save();
                        changed_any = true;
                    }
                    ok_clicked = ui.button("OK").clicked();
                });
                ui.separator();
                ui.label(egui::RichText::new("Saved beside binary as hyperdrive.conf \u{00B7} GPL-3.0-or-later").small());
            });
        if ok_clicked { self.show_settings = false; }
        if !settings_open { self.show_settings = false; }

        if changed_any && old != self.settings {
            theme::apply_all(ctx, &self.settings);
            if let Err(e) = self.settings.save() {
                self.panes[self.settings.active_pane.min(1)].last_error =
                    Some(format!("save config failed: {e}"));
            }
        }
    }
}

impl HyperDriveApp {
    fn playback_bar(&mut self, ctx: &egui::Context) {
        if let Some(p) = self.audio.playing_file().cloned() {
            if self.audio.finished() && !self.audio.paused() {
                self.audio.stop();
                return;
            }
            egui::TopBottomPanel::bottom("playback_bar").show(ctx, |ui| {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    let label = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if ui
                        .add(
                            egui::Button::new(if self.audio.paused() { "Play" } else { "Pause" })
                                .min_size(egui::vec2(0.0, 22.0)),
                        )
                        .clicked()
                    {
                        self.audio.toggle_pause();
                    }
                    if ui
                        .add(egui::Button::new("Stop").min_size(egui::vec2(0.0, 22.0)))
                        .clicked()
                    {
                        self.audio.stop();
                        return;
                    }
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!(
                            "\u{266B} {}{}",
                            label,
                            if self.audio.rendering { "  \u{2026} loading module" } else { "" }
                        ))
                        .small(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_sized(
                            [90.0, 20.0],
                            egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false),
                        )
                        .on_hover_text("Volume");
                        ui.small("Vol");
                    });
                });
                ui.add_space(2.0);
            });
        }
}
}


    // ============================ MENU BAR ============================
impl HyperDriveApp {
    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.menu_button("File", |ui| { self.menu_file(ui); });
                ui.menu_button("Edit", |ui| { self.menu_edit(ui); });
                ui.menu_button("View", |ui| { self.menu_view(ui); });
                ui.menu_button("Go", |ui| { self.menu_go(ui); });
                ui.menu_button("Bookmarks", |ui| { self.menu_bookmarks(ui); });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("Help", |ui| {
                        if ui.button("About HyperDrive").clicked() {
                            self.show_settings = false;
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.label(egui::RichText::new(
                            "HyperDrive v0.4\nFast, portable file manager\nGPL-3.0-or-later\n(C) 2026 dragon").small());
                    });
                });
            });
        });
    }

    fn menu_file(&mut self, ui: &mut egui::Ui) {
        if ui.button("New Folder...").clicked() {
            self.dialog = Dialog::NewFolder("New folder".into());
            ui.close_menu();
        }
        let has_sel = !self.panes[self.settings.active_pane.min(1)].selected.is_empty();
        if ui.add_enabled(has_sel, egui::Button::new("Rename...\tF2")).clicked() {
            let p = &self.panes[self.settings.active_pane.min(1)];
            if let Some(n) = p.selected.iter().next() {
                let i = self.settings.active_pane.min(1);
                self.panes[i].renaming = Some((n.clone(), n.clone()));
            }
            ui.close_menu();
        }
        if ui.add_enabled(has_sel, egui::Button::new("Open With...")).clicked() {
            let p = selected_paths(&self.panes[self.settings.active_pane.min(1)])
                .first()
                .cloned()
                .map(|x| x.display().to_string())
                .unwrap_or_default();
            self.dialog = Dialog::OpenWith { path: p, filter: String::new(), remember: false };
            ui.close_menu();
        }
        if ui.add_enabled(has_sel, egui::Button::new("Properties\tAlt+Enter")).clicked() {
            self.open_properties();
            ui.close_menu();
        }
        ui.separator();
        if ui.add_enabled(has_sel, egui::Button::new("Move to Trash\tDel")).clicked() {
            self.trash_selected();
            ui.close_menu();
        }
        if ui.add_enabled(
            has_sel,
            egui::Button::new(
                egui::RichText::new("Delete Permanently...\tShift+Del")
                    .color(egui::Color32::from_rgb(0xE8, 0x5D, 0x75)),
            ),
        )
        .clicked()
        {
            let names = selected_paths(&self.panes[self.settings.active_pane.min(1)])
                .iter().map(|p| p.display().to_string()).collect();
            self.dialog = Dialog::ConfirmDelete { names };
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Quit").clicked() {
            ui.close_menu();
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn menu_edit(&mut self, ui: &mut egui::Ui) {
        let has_sel = !self.panes[self.settings.active_pane.min(1)].selected.is_empty();
        let sel_snapshot = selected_paths(&self.panes[self.settings.active_pane.min(1)]);
        if ui.add_enabled(has_sel, egui::Button::new("Copy")).clicked() {
            self.clip_paths = sel_snapshot.clone();
            self.clip_cut = false;
            ui.close_menu();
        }
        if ui.add_enabled(has_sel, egui::Button::new("Cut")).clicked() {
            self.clip_paths = sel_snapshot.clone();
            self.clip_cut = true;
            ui.close_menu();
        }
        let can_paste = !self.clip_paths.is_empty();
        if ui.add_enabled(can_paste, egui::Button::new("Paste into current folder")).clicked() {
            self.paste_clipboard();
            ui.close_menu();
        }
        if ui.add_enabled(has_sel, egui::Button::new("Duplicate\tCtrl+D")).clicked() {
            let mut dst_captured: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
            let paths = sel_snapshot.clone();
            for p in paths {
                let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                loop {
                    let dst = dir.join(format!("{stem} - Copy{ext}"));
                    if !dst.exists() { dst_captured.push((p.clone(), dst)); break; }
                }
            }
            self.do_duplicates(dst_captured);
            ui.close_menu();
        }
        if ui.add_enabled(
            self.panes[self.settings.active_pane.min(1)].selected.len() > 1,
            egui::Button::new("Multi-Rename..."),
        ).clicked() {
            self.dialog = Dialog::MultiRename {
                pattern_find: String::new(),
                pattern_repl: String::new(),
                numbered_start: 1,
            };
            ui.close_menu();
        }
        if ui.add_enabled(has_sel && cfg!(unix), egui::Button::new("Create Symlink...")).clicked() {
            let first = sel_snapshot.first().cloned();
            if let Some(src) = first {
                let dir = src.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                let name = format!("Link to {}", src.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default());
                let link = dir.join(&name);
                #[cfg(unix)]
                { if let Err(e) = std::os::unix::fs::symlink(&src, &link) {
                    self.panes[0].last_error = Some(format!("symlink: {e}"));
                    self.panes[0].err_ttl = 480; } }
                self.active_pane().reload();
            }
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Select All").clicked() {
            let i = self.settings.active_pane.min(1);
            self.panes[i].selected =
                self.panes[i].entries.iter().map(|e| e.name.clone()).collect();
            ui.close_menu();
        }
        if ui.button("Clear Selection").clicked() {
            self.panes[self.settings.active_pane.min(1)].selected.clear();
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Preferences (Settings)").clicked() {
            self.show_settings = true;
            ui.close_menu();
        }
    }

    fn menu_view(&mut self, ui: &mut egui::Ui) {
        if ui.checkbox(&mut self.settings.show_tree, "Folder tree panel").changed() {
            let _ = self.settings.save();
        }
        if ui.checkbox(&mut self.settings.grid_view, "Icon / grid view").changed() {
            let _ = self.settings.save();
        }
        if ui.checkbox(&mut self.settings.dual_pane, "Dual pane").changed() {
            let _ = self.settings.save();
        }
        if ui.checkbox(&mut self.settings.show_preview, "Preview panel (image)").changed() {
            let _ = self.settings.save();
        }
        if ui.checkbox(&mut self.settings.show_hidden, "Show hidden files").changed() {
            let _ = self.settings.save();
        }
        ui.separator();
        ui.label(egui::RichText::new("Sort by").small());
        let a = &mut self.panes[self.settings.active_pane.min(1)];
        for key in [SortKey::Name, SortKey::Size, SortKey::Modified, SortKey::Created] {
            if ui.radio(a.sort_key == key, key.label()).clicked() {
                a.toggle_sort(key);
            }
        }
        ui.separator();
        if ui.button("Open Terminal Here\tF4").clicked() {
            self.open_terminal_here();
            ui.close_menu();
        }
        if ui.button("Reload\tF5").clicked() {
            self.active_pane().reload();
            ui.close_menu();
        }
    }

    fn menu_go(&mut self, ui: &mut egui::Ui) {
        let home = browser::home_dir();
        let items: Vec<(&str, PathBuf)> = vec![
            ("Back", PathBuf::new()),
            ("Forward", PathBuf::new()),
        ];
        let _ = items;
        if ui.button("Back").clicked() { self.active_pane().go_back(); ui.close_menu(); }
        if ui.button("Forward").clicked() { self.active_pane().go_forward(); ui.close_menu(); }
        if ui.button("Up One Level").clicked() {
            if let Some(p) = self.active_pane().history.current.parent().map(|x| x.to_path_buf()) {
                self.active_pane().navigate(p);
            }
            ui.close_menu();
        }
        ui.separator();
        for (label, path) in [
            ("Home", home),
            ("Filesystem", PathBuf::from("/")),
            ("Documents", browser::home_dir().join("Documents")),
            ("Downloads", browser::home_dir().join("Downloads")),
            ("Pictures", browser::home_dir().join("Pictures")),
            ("Music", browser::home_dir().join("Music")),
            ("Videos", browser::home_dir().join("Videos")),
        ] {
            if path.exists() && ui.button(label).clicked() {
                self.active_pane().navigate(path);
                ui.close_menu();
            }
        }
    }

    fn menu_bookmarks(&mut self, ui: &mut egui::Ui) {
        if ui.button("Bookmark This Folder").clicked() {
            let cur = self.active_pane().history.current.display().to_string();
            if !self.settings.bookmarks.contains(&cur) {
                self.settings.bookmarks.push(cur);
                let _ = self.settings.save();
            }
            ui.close_menu();
        }
        ui.separator();
        if self.settings.bookmarks.is_empty() {
            ui.label(egui::RichText::new("(no bookmarks yet)").weak().small());
        } else {
            for b in self.settings.bookmarks.clone() {
                let name = std::path::Path::new(&b)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| b.clone());
                if ui.button(name).clicked() {
                    let p = PathBuf::from(&b);
                    self.active_pane().navigate(p);
                    ui.close_menu();
                }
            }
        }
    }

    // ===================== FILE OPERATIONS =====================
    fn trash_selected(&mut self) {
        let paths = {
            let a = self.active_pane();
            selected_paths(a)
        };
        let mut errs = Vec::new();
        for p in &paths {
            match std::process::Command::new("gio").arg("trash").arg(p).output() {
                Ok(o) if o.status.success() => {}
                Ok(o) => errs.push(format!(
                    "{}: {}",
                    p.display(),
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => errs.push(format!("gio: {e}")),
            }
        }
        self.active_pane().reload();
        if let Some(e) = errs.first() {
            self.active_pane().last_error = Some(e.clone());
        }
    }

    fn paste_clipboard(&mut self) {
        use std::fs;
        let dest_dir = self.active_pane().history.current.clone();
        let srcs = self.clip_paths.clone();
        let cut = self.clip_cut;
        let mut errs = Vec::new();
        for src in srcs {
            let fname = src.file_name().map(|n| n.to_os_string()).unwrap_or_default();
            let mut dst = dest_dir.join(&fname);
            if dst.exists() {
                let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                let mut n = 2;
                loop {
                    dst = dest_dir.join(format!("{stem} (copy {n}){ext}"));
                    if !dst.exists() { break; }
                    n += 1;
                }
            }
            let r = if src.is_dir() {
                fs::create_dir_all(&dst)
                    .and_then(|_| copy_tree(&src, &dst))
            } else {
                fs::copy(&src, &dst).map(|_| ())
            };
            if let Err(e) = r {
                errs.push(format!("{}: {e}", src.display()));
            } else if cut {
                let _ = fs::remove_file(&src).or_else(|_| fs::remove_dir_all(&src));
            }
        }
        if cut { self.clip_paths.clear(); }
        self.active_pane().reload();
        if let Some(e) = errs.first() {
            self.active_pane().last_error = Some(e.clone());
        }
    }

    fn new_folder(&mut self, name: &str) {
        let dir = self.active_pane().history.current.join(name);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.active_pane().last_error = Some(format!("mkdir: {e}"));
        } else {
            self.active_pane().reload();
        }
    }
}

// free helpers
fn selected_paths(pane: &Pane) -> Vec<PathBuf> {
    pane.entries
        .iter()
        .filter(|e| pane.selected.contains(&e.name))
        .map(|e| e.path.clone())
        .collect()
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::fs;
    if src.is_dir() {
        for e in fs::read_dir(src)?.flatten() {
            let t = dst.join(e.file_name());
            if e.path().is_dir() {
                fs::create_dir_all(&t)?;
                copy_tree(&e.path(), &t)?;
            } else {
                fs::copy(e.path(), t)?;
            }
        }
    }
    Ok(())
}


    // ============================ DIALOGS ============================
impl HyperDriveApp {
    fn draw_dialog(&mut self, ctx: &egui::Context) {
        match std::mem::replace(&mut self.dialog, Dialog::None) {
            Dialog::None => {}
            Dialog::NewFolder(mut name) => {
                let mut open = true;
                egui::Window::new("New Folder").open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.horizontal(|ui| { ui.label("Name:"); ui.text_edit_singleline(&mut name); });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            let n = name.clone();
                            self.new_folder(&n);
                            self.dialog = Dialog::None;
                        }
                        if ui.button("Cancel").clicked() { self.dialog = Dialog::None; }
                    });
                });
                if !open { self.dialog = Dialog::None; } else { self.dialog = Dialog::NewFolder(name); }
            }
            Dialog::Properties { name, path, is_dir, size_s, modified_s, created_s, perms_s, readonly_fs, items } => {
                let mut open = true;
                egui::Window::new("Properties").open(&mut open).resizable(false).show(ctx, |ui| {
                    egui::Grid::new("props_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
                        ui.label("Name:"); ui.label(egui::RichText::new(&name).strong()); ui.end_row();
                        ui.label("Type:"); ui.label(if is_dir { "Folder" } else { "File" }); ui.end_row();
                        ui.label("Size:"); ui.label(size_s); ui.end_row();
                        if let Some((f, d)) = items {
                            ui.label("Contents:"); ui.label(format!("{f} files, {d} folders")); ui.end_row();
                        }
                        ui.label("Modified:"); ui.label(modified_s); ui.end_row();
                        ui.label("Created:"); ui.label(created_s); ui.end_row();
                        ui.label("Permissions:"); ui.label(format!("{} ({})", perms_s, if readonly_fs {"read-only"} else {"writable"})); ui.end_row();
                        ui.label("Full path:");
                        ui.add_sized([340.0, 18.0], egui::TextEdit::singleline(&mut path.clone()).font(egui::TextStyle::Small));
                        ui.end_row();
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Close").clicked() { self.dialog = Dialog::None; }
                    });
                });
                if !open { self.dialog = Dialog::None; }
            }
            Dialog::SmbBrowse { mut entries, mut picked } => {
                let mut open = true;
                let mut connect_host: Option<(String, String)> = None;
                egui::Window::new("\u{1F310} SMB servers on this network")
                    .open(&mut open).default_width(420.0).default_height(320.0)
                    .show(ctx, |ui| {
                        if ui.button("Rescan").clicked() {
                            entries = scan_smb_hosts();
                            picked = None;
                        }
                        if picked.is_some() && ui.small_button("\u{2190} back to servers").clicked() {
                            picked = None;
                        }
                        ui.small(format!("{} found via mDNS (_smb._tcp)", entries.len()));
                        ui.separator();
                        egui::ScrollArea::vertical().auto_shrink([false,false]).show(ui, |ui| {
                            // If a host is picked, show ITS shares instead.
                            if let Some(h) = &picked {
                                ui.label(egui::RichText::new(format!("Shares on {h}:")).strong().small());
                                let shares = scan_smb_shares(h);
                                if shares.is_empty() {
                                    ui.small("(none visible - server may need auth)");
                                }
                                for sh in &shares {
                                    if ui.selectable_label(false,
                                        egui::RichText::new(format!("\u{1F4C2} {sh}")).small()).clicked()
                                    {
                                        connect_host = Some((h.clone(), sh.clone()));
                                    }
                                }
                            } else {
                            for (host, ip) in entries.iter() {
                                ui.horizontal(|ui| {
                                    let sel = picked.as_deref() == Some(host.as_str());
                                    if ui.selectable_label(sel, format!("\u{1F5A5} {host} ({ip})")).clicked() {
                                        picked = Some(host.clone());
                                    }
                                });
                            }
                            }
                            if entries.is_empty() {
                                ui.label(egui::RichText::new(
                                    "None found. Servers must advertise SMB via mDNS;\n                                     you can still use Connect... and type the address.").weak().small());
                            }
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if let Some(h) = &picked {
                                if ui.button(format!("Connect to {}...", h)).clicked() {
                                    connect_host = Some((h.clone(), entries.iter()
                                        .find(|(hh, _)| hh == h).map(|(_, i)| i.clone()).unwrap_or_default()));
                                }
                            }
                            if ui.button("Close").clicked() { self.dialog = Dialog::None; }
                        });
                    });
                let cancelled = !open;
                if cancelled { self.dialog = Dialog::None; return; }
                if let Some((host, share)) = connect_host {
                    self.dialog = Dialog::SmbConnect {
                        server: host,
                        share,
                        user: std::env::var("USER").unwrap_or_default(),
                        domain: "WORKGROUP".into(),
                        guest: false,
                    };
                } else {
                    self.dialog = Dialog::SmbBrowse { entries, picked };
                }
            }
            Dialog::MultiRename { mut pattern_find, mut pattern_repl, mut numbered_start } => {
                let mut open = true;
                let mut apply = false;
                let items: Vec<(String,String)> = {
                    let p = &self.panes[self.settings.active_pane.min(1)];
                    p.entries.iter()
                        .filter(|e| p.selected.contains(&e.name))
                        .map(|e| (e.name.clone(), e.name.clone()))
                        .collect()
                };
                // compute new names
                let mut counter = numbered_start;
                let computed: Vec<(String,String)> = items.iter().map(|(_,old)| {
                    let mut n = old.replace(&pattern_find, &pattern_repl);
                    if n == *old && !pattern_repl.is_empty() {
                        // append numbering token usage: {n} supported in repl
                    }
                    if pattern_repl.contains("{n}") {
                        n = pattern_repl.replacen("{n}", &format!("{counter:03}"), 1);
                        counter += 1;
                    }
                    (old.clone(), n)
                }).collect();

                egui::Window::new("Multi-Rename").open(&mut open)
                    .default_width(560.0).default_height(400.0)
                    .show(ctx, |ui| {
                        egui::Grid::new("mr_grid").num_columns(2).spacing([8.0,6.0]).show(ui,|ui|{
                            ui.label("Find:"); ui.text_edit_singleline(&mut pattern_find); ui.end_row();
                            ui.label("Replace ({n} = counter):"); ui.text_edit_singleline(&mut pattern_repl); ui.end_row();
                            ui.label("Start #:"); ui.add(egui::DragValue::new(&mut numbered_start)); ui.end_row();
                        });
                        ui.separator();
                        egui::ScrollArea::vertical().auto_shrink([false,false]).show(ui,|ui|{
                            for (o,n) in &computed {
                                let changed = o!=n;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(o).small());
                                    ui.label("\u{27A4}");
                                    ui.label(egui::RichText::new(n).small()
                                        .color(if changed { egui::Color32::from_rgb(0x77,0xd1,0x7a) }
                                               else { ui.visuals().weak_text_color() }));
                                });
                            }
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("Apply Rename").clicked() { apply = true; }
                            if ui.button("Cancel").clicked() { self.dialog = Dialog::None; }
                        });
                    });
                if !open { self.dialog = Dialog::None; }
                if apply {
                    let dir = self.panes[self.settings.active_pane.min(1)].history.current.clone();
                    let mut errs=Vec::new();
                    for (o,n) in &computed {
                        if o==n || n.trim().is_empty() { continue; }
                        if let Err(e)=std::fs::rename(dir.join(o), dir.join(n)) {
                            errs.push(format!("{o}: {e}"));
                        }
                    }
                    self.panes[self.settings.active_pane.min(1)].reload();
                    if let Some(e)=errs.first() {
                        self.panes[self.settings.active_pane.min(1)].last_error=Some(e.clone());
                        self.panes[self.settings.active_pane.min(1)].err_ttl=480;
                    }
                    self.dialog = Dialog::None;
                } else if open {
                    self.dialog = Dialog::MultiRename { pattern_find, pattern_repl, numbered_start };
                }
            }
            Dialog::ConfirmDelete { names } => {
                let mut open = true;
                let mut confirmed = false;
                egui::Window::new("\u{26A0} Delete permanently?")
                    .open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.label(egui::RichText::new(format!(
                        "This will PERMANENTLY delete {} item{}:", names.len(),
                        if names.len()==1 {""} else {"s"})).strong());
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                        for n in names.iter().take(12) {
                            ui.label(egui::RichText::new(n).small());
                        }
                    });
                    if names.len() > 12 { ui.small(format!("...and {} more", names.len()-12)); }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() { self.dialog = Dialog::None; }
                        if ui.button(egui::RichText::new("Delete forever")
                            .color(egui::Color32::from_rgb(0xE8,0x5D,0x75))).clicked()
                        { confirmed = true; }
                    });
                });
                if !open { self.dialog = Dialog::None; }
                if confirmed {
                    let paths: Vec<PathBuf> = names.iter().map(PathBuf::from).collect();
                    self.delete_permanent(paths);
                    self.dialog = Dialog::None;
                }
            }
            Dialog::SftpConnect { mut server, mut user } => {
                let mut open = true;
                let mut go = false;
                egui::Window::new("\u{1F5A7} Connect to SFTP/SSH server")
                    .open(&mut open).resizable(false).show(ctx, |ui| {
                    egui::Grid::new("sftp_grid").num_columns(2).spacing([8.0,6.0]).show(ui, |ui| {
                        ui.label("Server:"); ui.text_edit_singleline(&mut server); ui.end_row();
                        ui.label("Username:"); ui.text_edit_singleline(&mut user); ui.end_row();
                    });
                    ui.small("GVFS will prompt for password/key passphrase.");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        go = ui.button("Connect").clicked();
                        if ui.button("Cancel").clicked() { self.dialog = Dialog::None; }
                    });
                });
                if !open { self.dialog = Dialog::None; return; }
                if go && !server.trim().is_empty() {
                    let url = format!("sftp://{}@{}", user.trim(), server.trim());
                    self.mount_and_open_smb(&url);
                    self.dialog = Dialog::None;
                } else if !go {
                    self.dialog = Dialog::SftpConnect { server, user };
                }
            }
            Dialog::OpenWith { path, mut filter, mut remember } => {
                let mut open = true;
                let mut launch: Option<(String, String)> = None; // (desktop_path, exec)
                let ext_now = std::path::Path::new(&path)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                    .unwrap_or_default();
                egui::Window::new("Open With...")
                    .open(&mut open).default_width(460.0).default_height(380.0)
                    .show(ctx, |ui| {
                        ui.label(egui::RichText::new(&path).small().weak());
                        if !ext_now.is_empty() {
                            let def = self.settings.openwith.iter()
                                .find(|(e, _)| e == &ext_now).map(|(_, d)| d.clone());
                            ui.small(match def {
                                Some(d) => format!("default for {ext_now}: {d}"),
                                None => format!("no default set for {ext_now}"),
                            });
                        }
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label("Filter:");
                            ui.text_edit_singleline(&mut filter);
                        });
                        if !ext_now.is_empty() {
                            ui.checkbox(&mut remember,
                                format!("Always use selected app for {}", ext_now));
                        }
                        ui.separator();
                        egui::ScrollArea::vertical().auto_shrink([false,false]).show(ui, |ui| {
                            for (app_name, desktop_path, exec) in list_desktop_apps() {
                                let flt = filter.to_lowercase();
                                if !flt.is_empty() && !app_name.to_lowercase().contains(&flt) {
                                    continue;
                                }
                                if ui.selectable_label(false, &app_name).clicked() {
                                    launch = Some((desktop_path, exec));
                                }
                            }
                        });
                    });
                let cancelled = !open;
                if cancelled { self.dialog = Dialog::None; return; }
                if let Some((dp, _exec)) = launch {
                    if remember && !ext_now.is_empty() {
                        self.settings.openwith.retain(|(e, _)| e != &ext_now);
                        self.settings.openwith.push((ext_now.clone(), dp.clone()));
                        let _ = self.settings.save();
                    }
                    let _ = std::process::Command::new("gio")
                        .arg("launch").arg(&dp).arg(&path).spawn();
                    self.dialog = Dialog::None;
                } else {
                    self.dialog = Dialog::OpenWith { path, filter, remember };
                }
            }
            Dialog::SmbBrowse2 { mut host, mut share, mut path, mut entries, mut status, mut newfolder, selected } => {
                let _sel_ref = selected.clone();
                let mut open = true;
                let mut action: Option<(String, bool)> = None; // (name, is_dir)
                let mut go_up = false;
                let mut connect_req = false;
                egui::Window::new(format!("\u{1F5A5} \\\\{}\\{}", host, share))
                    .open(&mut open).default_width(520.0).default_height(400.0)
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Host:");
                            ui.add(egui::TextEdit::singleline(&mut host).desired_width(120.0));
                            ui.label("Share:");
                            ui.add(egui::TextEdit::singleline(&mut share).desired_width(90.0));
                            if ui.button("Open").clicked() { connect_req = true; entries.clear(); }
                        });
                        if share.is_empty() {
                            ui.small("type the share name then press Open");
                        } else {
                            ui.horizontal(|ui| {
                                if ui.button("\u{2B06} Up").clicked() { go_up = true; }
                                ui.small(format!("\\\\{}\\{}\\{}", host, share, path));
                                ui.separator();
                                let _do_mkdir = false;
                                ui.add(egui::TextEdit::singleline(&mut newfolder)
                                    .hint_text("new folder").desired_width(120.0));
                                if ui.small_button("MkDir").clicked() && !newfolder.trim().is_empty() {
                                    use std::process::Command as C3;
                                    let dirpart = if path.is_empty(){String::new()}else{format!("cd \"{path}\"; ")};
                                    let _ = C3::new("timeout").arg("20s")
                                        .arg("smbclient").arg(format!("//{}/{}", host, share)).arg("-N")
                                        .arg("-c").arg(format!("{}mkdir \"{}\"", dirpart, newfolder.trim()))
                                        .output();
                                    entries = smb_ls(&host, &share, &path, &mut status);
                                    newfolder.clear();
                                }
                                if !entries.is_empty() && ui.small_button("Download selected...").clicked() {}
                            });
                            ui.separator();
                            egui::ScrollArea::vertical().auto_shrink([false,false]).show(ui, |ui| {
                                for (name, is_dir, size) in entries.clone() {
                                    if name == "." || name == ".." { continue; }
                                    let icon = if is_dir {"\u{1F4C1}"} else {"\u{1F4C4}"};
                                    let szs = if is_dir { "<dir>".into() } else { browser::format_size(size) };
                                    ui.horizontal(|ui| {
                                        if ui.selectable_label(false,
                                            egui::RichText::new(format!("{icon} {name}")).small()).clicked()
                                        { action = Some((name.clone(), is_dir)); }
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.small(szs);
                                        });
                                    });
                                }
                            });
                        }
                        if !status.is_empty() {
                            ui.colored_label(egui::Color32::from_rgb(0xE8,0x5D,0x75), egui::RichText::new(&status).small());
                        }
                    });
                if !open { self.dialog = Dialog::None; return; }

                if connect_req && !host.trim().is_empty() && !share.trim().is_empty() {
                    path.clear();
                    entries = smb_ls(&host, &share, "", &mut status);
                    self.dialog = Dialog::SmbBrowse2 { host: host.trim().to_string(), share: share.trim().to_string(), path, entries, status, newfolder: String::new(), selected: None, };
                    return;
                }
                if go_up {
                    if let Some(pos)=path.rfind('\\') { path.truncate(pos); } else { path.clear(); }
                    entries = smb_ls(&host,&share,&path,&mut status);
                    self.dialog = Dialog::SmbBrowse2 { host, share, path, entries, status, newfolder: String::new(), selected: None, };
                    return;
                }
                if let Some((name,is_dir)) = action {
                    let newpath = if path.is_empty() { name.clone() } else { format!("{path}\\{name}") };
                    if is_dir {
                        entries = smb_ls(&host,&share,&newpath,&mut status);
                        self.dialog = Dialog::SmbBrowse2 { host, share, path:newpath, entries, status, newfolder: String::new(), selected: None, };
                    } else {
                        // download to cache dir and open with default app
                        let dl = std::env::var("HOME").map(|h| PathBuf::from(h).join(".cache/hyperdrive/smb")).unwrap_or_else(|_| PathBuf::from("/tmp"));
                        let _ = std::fs::create_dir_all(&dl);
                        let local = dl.join(&name);
                        use std::process::Command as C2;
                        let dirpart = if path.is_empty(){String::new()}else{format!("cd \"{path}\"; ")};
                        let out = C2::new("timeout").arg("60s")
                            .arg("smbclient").arg(format!("//{}/{}",host,share)).arg("-N")
                            .arg("-c").arg(format!("{}get \"{name}\" \"{}\"", dirpart, local.display()))
                            .output();
                        match out {
                            Ok(o) if o.status.success() => {
                                self.smart_open(&local);
                                status.clear();
                            }
                            Ok(o)=>{ status=String::from_utf8_lossy(&o.stderr).trim().to_string(); if status.is_empty(){status="download failed".into();} }
                            Err(e)=>{ status=format!("{e}"); }
                        }
                        self.dialog = Dialog::SmbBrowse2 { host, share, path, entries, status, newfolder: String::new(), selected: None, };
                    }
                    return;
                }
                self.dialog = Dialog::SmbBrowse2 { host, share, path, entries, status, newfolder: String::new(), selected: None, };
            }
            Dialog::SmbConnect { mut server, mut share, mut user, mut domain, mut guest } => {
                let mut open = true;
                let mut connect = false;
                egui::Window::new("\u{1F5A7} Connect to Windows Share (SMB)")
                    .open(&mut open).resizable(false).show(ctx, |ui| {
                    egui::Grid::new("smb_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label("Server (IP or name):"); ui.text_edit_singleline(&mut server); ui.end_row();
                        ui.label("Share name:"); ui.text_edit_singleline(&mut share); ui.end_row();
                        ui.label("Username:"); ui.text_edit_singleline(&mut user); ui.end_row();
                        ui.label("Domain:"); ui.text_edit_singleline(&mut domain); ui.end_row();
                    });
                    ui.checkbox(&mut guest, "Guest access (no password)");
                    ui.separator();
                    ui.collapsing("\u{1F6E1} Tips for safe connecting", |ui| {
                        ui.label(egui::RichText::new(
                            "- Connect only on networks you trust.\n- Prefer username/password over Guest.\n                             Guest works only if the Windows side enabled it.\n                             - Firewall must allow TCP 445 to the server.\n                             - GVFS will ask for the password and remember it per session.").small());
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        connect = ui.button("Connect").clicked();
                        if ui.button("Cancel").clicked() { self.dialog = Dialog::None; }
                    });
                });
                if !open { self.dialog = Dialog::None; return; }
                if connect {
                    let url = if guest || user.trim().is_empty() {
                        format!("smb://{server}/{share}")
                    } else if domain.trim().is_empty() || domain.trim().eq_ignore_ascii_case("workgroup") {
                        format!("smb://{user}@{server}/{share}")
                    } else {
                        format!("smb://{};{}@{server}/{share}", domain.trim(), user.trim())
                    };
                    self.mount_and_open_smb(&url);
                    let label = format!("{}/{}", server.trim(), share.trim());
                    if !self.settings.net_locations.iter().any(|(_, u)| u == &url) {
                        self.settings.net_locations.push((label, url));
                        let _ = self.settings.save();
                    }
                    self.dialog = Dialog::None;
                } else {
                    self.dialog = Dialog::SmbConnect { server, share, user, domain, guest };
                }
            }
            Dialog::ShareFolder { path, mut name, mut readonly, mut guest, mut comment } => {
                let mut open = true;
                let mut save_script = false;
                egui::Window::new("\u{1F4E4} Share Folder Safely")
                    .open(&mut open).resizable(true).default_width(520.0).show(ctx, |ui| {
                    egui::Grid::new("share_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label("Folder:"); ui.label(egui::RichText::new(&path).small()); ui.end_row();
                        ui.label("Share name:"); ui.text_edit_singleline(&mut name); ui.end_row();
                        ui.label("Comment:"); ui.text_edit_singleline(&mut comment); ui.end_row();
                    });
                    ui.checkbox(&mut readonly, "Read-only share (recommended default)");
                    ui.checkbox(&mut guest, "Allow guest access (no password)");
                    ui.separator();
                    ui.collapsing("\u{1F6E1} Security checklist - read before sharing", |ui| {
                        ui.label(egui::RichText::new(
"- Read-only ON unless others must write.\n\
- Guest OFF on shared/office networks; anyone can read/write.\n\
- Share is limited to your LAN subnet (hosts allow) by the script.\n\
- Never share your whole home directory; pick one folder.\n\
- Samba sets its OWN password: sudo smbpasswd -a $USER (not your login pw).\n\
- Firewall: script opens Samba only if ufw is active.\n\
- Files written by guests get umask 022 (world-readable) by design.\n\
- testparm validates config before samba restarts.").small());
                    });
                    ui.separator();
                    ui.label(egui::RichText::new(format!(
                        "Creates a ready-to-run script you review then apply with:\nsudo bash setup_share_{}.sh", name)).small());
                    ui.add_space(4.0);
                    save_script = ui.button("Generate setup script...").clicked()
                        || ui.button("Save").clicked();
                    if ui.button("Cancel").clicked() { self.dialog = Dialog::None; }
                });
                if !open { self.dialog = Dialog::None; return; }
                if save_script && !name.trim().is_empty() {
                    let script_path =
                        std::path::PathBuf::from(format!("/tmp/hyperdrive_setup_share_{}.sh", sanitize(&name)));
                    let script = build_share_script(&path, name.trim(), &comment, readonly, guest);
                    match std::fs::write(&script_path, script) {
                        Ok(_) => {
                            let _ = std::process::Command::new("chmod")
                                .arg("+x").arg(&script_path).status();
                            self.active_pane().last_error = Some(format!(
                                "Script saved: {} - review it, then run: sudo bash {}",
                                script_path.display(),
                                script_path.display()));
                        }
                        Err(e) => {
                            self.active_pane().last_error = Some(format!("write failed: {e}"));
                        }
                    }
                    self.dialog = Dialog::None;
                } else if !save_script {
                    self.dialog = Dialog::ShareFolder { path, name, readonly, guest, comment };
                }
            }
        }
    }

}


impl HyperDriveApp {
    // ========================= KEYBOARD SHORTCUTS =========================
    fn handle_hotkeys(&mut self, ctx: &egui::Context) {
        use egui::Key::*;
        let wants = ctx.memory(|m| m.focused().is_some());
        if wants {
            return; // typing in path/filter/rename fields
        }
        let idx = self.settings.active_pane.min(1);

        let mut open_props = false;
        ctx.input(|i| {
            // Navigation
            if i.key_pressed(F2) {
                if let Some(n) = first_selected(&self.panes[idx]) {
                    self.panes[idx].renaming = Some((n.clone(), n));
                }
            }
            if i.key_pressed(Delete) {
                if i.modifiers.shift { 
                    let names = selected_paths(&self.panes[idx])
                        .iter().map(|p| p.display().to_string()).collect();
                    self.dialog = Dialog::ConfirmDelete { names };
                } else {
                    self.trash_selected();
                }
            }
            if i.key_pressed(F5) { self.panes[idx].reload(); }
            if i.key_pressed(F4) { self.open_terminal_here(); }
            if self.settings.dual_pane && i.key_pressed(F5) { self.cross_pane(false); }
            if self.settings.dual_pane && i.key_pressed(F6) { self.cross_pane(true); }
            if i.modifiers.ctrl && i.key_pressed(H) {
                self.settings.show_hidden = !self.settings.show_hidden;
                let _ = self.settings.save();
            }
            if i.modifiers.alt && i.key_pressed(ArrowLeft) { self.panes[idx].go_back(); }
            if i.modifiers.alt && i.key_pressed(ArrowRight) { self.panes[idx].go_forward(); }
            if i.modifiers.alt && (i.key_pressed(ArrowUp) || i.key_pressed(Backspace)) {
                if let Some(p) = self.panes[idx].history.current.parent().map(|x| x.to_path_buf()) {
                    self.panes[idx].navigate(p);
                }
            }
            // Clipboard
            if i.modifiers.ctrl && i.key_pressed(C) { self.clip_paths = selected_paths(&self.panes[idx]); self.clip_cut = false; }
            if i.modifiers.ctrl && i.key_pressed(X) { self.clip_paths = selected_paths(&self.panes[idx]); self.clip_cut = true; }
            if i.modifiers.ctrl && i.key_pressed(V) { self.paste_clipboard(); }
            if i.modifiers.ctrl && i.key_pressed(A) {
                self.panes[idx].selected = self.panes[idx].entries.iter()
                    .filter(|e| self.settings.show_hidden || !e.name.starts_with('.'))
                    .map(|e| e.name.clone()).collect();
            }
            if i.modifiers.ctrl && i.key_pressed(L) { self.focus_path_req = true; }
            if i.modifiers.ctrl && i.key_pressed(F) {
                if self.search.is_none() {
                    let dir = self.panes[idx].history.current.clone();
                    self.search = Some(crate::app_search::SearchState::new(dir));
                } else {
                    self.search = None;
                }
            }
            if i.modifiers.alt && i.key_pressed(Enter) { open_props = true; }
            // Enter opens first selection
            if i.key_pressed(Enter) && !self.selected_names_empty(idx) {
                if let Some(p) = first_selected_path(&self.panes[idx]) {
                    if p.is_dir() {
                        let pp = p.clone();
                        self.panes[idx].navigate(pp);
                    } else if is_audio_file(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()) {
                        self.audio.play(&p).ok();
                    } else {
                        self.smart_open(&p);
                    }
                }
            }
        });
        if open_props {
            self.open_properties();
        }
    }

    // ============================ TABS ============================
    fn new_tab(&mut self, side: usize) {
        let cur = self.panes[side].history.current.clone();
        self.side_tabs[side].push(cur);
        // Fresh pane state at same dir is fine; user can navigate.
    }

    fn close_tab(&mut self, side: usize, tab_index: usize) {
        if self.side_tabs[side].len() <= 1 {
            return; // never close last tab
        }
        let closing_dir = self.side_tabs[side][tab_index].clone();
        let cur = self.panes[side].history.current.clone();
        let _ = closing_dir;
        // Remove the clicked tab (tab_index is the tab the user chose to close).
        let remove_at = tab_index;
        if self.side_tabs[side].len() > remove_at {
            self.side_tabs[side].remove(remove_at);
        }
        // If we closed the visible one, show neighbour.
        if cur == closing_dir || !self.side_tabs[side].contains(&cur) {
            let next = self.side_tabs[side]
                .get(remove_at.saturating_sub(1))
                .cloned()
                .or_else(|| self.side_tabs[side].first().cloned());
            if let Some(d) = next {
                self.panes[side].navigate(d);
            }
        }
    }

    fn switch_tab(&mut self, side: usize, dir: PathBuf) {
        self.panes[side].navigate(dir);
    }

    fn draw_tab_strip(&mut self, ui: &mut egui::Ui, side: usize) {
        let mut to_close: Option<usize> = None;
        let mut continue_outer = false;
        self.tab_rects[side].clear();
        ui.horizontal(|ui| {
            let tabs = self.side_tabs[side].clone();
            for (ti, d) in tabs.iter().enumerate() {
                let name = d
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "/".to_string());
                let is_cur = self.panes[side].history.current == *d;
                let btn = ui.add(
                    egui::Button::new(egui::RichText::new(&name).small())
                        .frame(is_cur),
                );
                if btn.clicked() {
                    self.switch_tab(side, d.clone());
                }
                if btn.secondary_clicked() {
                    self.close_tab(side, ti);
                }
                self.tab_rects[side].push((ti, btn.rect));
                let xid = egui::Id::new(format!("tabx_{}_{}", side, ti));
                ui.push_id(xid, |ui| {
                    if ui.small_button("\u{00D7}").clicked()
                        || ui
                            .add(
                                egui::Label::new("")
                                    .sense(egui::Sense::click()),
                            )
                            .middle_clicked()
                    {
                        to_close = Some(ti);
                    }
                });
            }
            if ui.small_button("+").on_hover_text("New tab (Ctrl+T)").clicked() {
                self.new_tab(side);
            }
        });

        // --- drag to reorder ---
        let ptr_down = ui.input(|i| i.pointer.primary_down());
        let pressed = ui.input(|i| i.pointer.primary_pressed());
        let released = ui.input(|i| i.pointer.primary_released());
        let cur_x = ui.input(|i| i.pointer.latest_pos()).map(|p| p.x);

        if pressed && self.dnd_tab.is_none() {
            if let Some(x) = cur_x {
                for (ti, r) in &self.tab_rects[side] {
                    if r.contains(egui::pos2(x, r.center().y)) {
                        self.dnd_tab = Some((side, *ti, x));
                        break;
                    }
                }
            }
        }
        if let Some((tside, ti, start_x)) = self.dnd_tab {
            if tside != side {
                continue_outer = true;
            } else if !ptr_down || released {
                self.dnd_tab = None;
            } else if let Some(x) = cur_x {
                let moved = (x - start_x).abs() > 6.0;
                if moved {
                    if x < start_x && ti > 0 {
                        self.side_tabs[side].swap(ti, ti - 1);
                        self.dnd_tab = Some((side, ti - 1, x));
                    } else if x > start_x && ti + 1 < self.side_tabs[side].len() {
                        self.side_tabs[side].swap(ti, ti + 1);
                        self.dnd_tab = Some((side, ti + 1, x));
                    }
                    ui.ctx().request_repaint();
                }
            }
        }

        let _ = continue_outer;
        if let Some(ti) = to_close {
            self.close_tab(side, ti);
        }
        ui.separator();
    }

    /// Execute a context-menu action requested by pane `idx`.
    fn apply_row_actions(&mut self, idx: usize) {
        use RowAction::*;
        let Some(act) = self.panes[idx].action_req.take() else { return };
        match act {
            Copy => {
                self.clip_paths = selected_paths(&self.panes[idx]);
                self.clip_cut = false;
            }
            Cut => {
                self.clip_paths = selected_paths(&self.panes[idx]);
                self.clip_cut = true;
            }
            Rename => {
                if let Some(n) = first_selected(&self.panes[idx]) {
                    self.panes[idx].renaming = Some((n.clone(), n));
                }
            }
            Duplicate => {
                let pairs = dup_targets(selected_paths(&self.panes[idx]));
                self.do_duplicates(pairs);
            }
            Symlink => self.symlink_first(),
            Trash => self.trash_selected(),
            DeletePermanent => {
                let names = selected_paths(&self.panes[idx])
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                self.dialog = Dialog::ConfirmDelete { names };
            }
            Props => self.open_properties(),
            Extract(arch) => {
                let dest = arch.parent().unwrap_or(Path::new("/")).to_path_buf();
                let name = arch.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                use std::process::Command;
                let cmd: Option<Command> =
                    if name.to_lowercase().ends_with(".zip") && which_exists("unzip") {
                        let mut c = Command::new("unzip");
                        c.arg("-o").arg(&arch).current_dir(&dest);
                        Some(c)
                    } else if which_exists("7z") {
                        let mut c = Command::new("7z");
                        c.arg("x").arg("-y").arg(format!("-o{}", dest.display())).arg(&arch);
                        Some(c)
                    } else if name.contains(".tar.") || name.ends_with(".tgz") {
                        let mut c = Command::new("tar");
                        c.arg("-xf").arg(&arch).current_dir(&dest);
                        Some(c)
                    } else { None };
                match cmd {
                    Some(c) => {
                        self.jobman.enqueue_extract(arch.clone(), c, dest.display().to_string());
                        self.show_jobs = true;
                    }
                    None => {
                        self.panes[idx].last_error =
                            Some("no extractor found (sudo apt install p7zip-full)".into());
                        self.panes[idx].err_ttl = 480;
                    }
                }
                self.panes[idx].reload();
            }
            Tag(color) => {
                let paths = selected_paths(&self.panes[idx]);
                for p in paths {
                    let key = p.display().to_string();
                    self.tagstore.set(&key, &color);
                    if color.is_empty() { self.tag_cache.remove(&PathBuf::from(&key)); }
                    else { self.tag_cache.insert(PathBuf::from(key), color.clone()); }
                }
                self.panes[idx].reload();
            }
            OpenWith => {
                if let Some(p) = selected_paths(&self.panes[idx]).first().cloned() {
                    self.dialog = Dialog::OpenWith {
                        path: p.display().to_string(),
                        filter: String::new(),
                        remember: false,
                    };
                }
            }
        }
    }

    fn delete_permanent(&mut self, paths: Vec<PathBuf>) {
        self.jobman.enqueue_delete(paths);
        self.show_jobs = true;
    }

    fn symlink_first(&mut self) {
        if let Some(src) = selected_paths(&self.panes[self.settings.active_pane.min(1)]).first().cloned() {
            let dir = src.parent().map(|d| d.to_path_buf()).unwrap_or_default();
            let name = format!("Link to {}", src.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default());
            #[cfg(unix)]
            { if let Err(e) = std::os::unix::fs::symlink(&src, dir.join(&name)) {
                self.panes[self.settings.active_pane.min(1)].last_error =
                    Some(format!("symlink: {e}"));
                self.panes[self.settings.active_pane.min(1)].err_ttl = 480; } }
            self.active_pane().reload();
        }
    }

    /// Copy (cut=false) or move (true) current selection to the other pane's dir.
    fn cross_pane(&mut self, move_it: bool) {
        let src_idx = self.settings.active_pane.min(1);
        let dst_idx = 1 - src_idx;
        let paths = selected_paths(&self.panes[src_idx]);
        if paths.is_empty() { return; }
        let kind = if move_it { crate::jobs::JobKind::Move } else { crate::jobs::JobKind::Copy };
        let dest_dir = self.panes[dst_idx].history.current.clone();
        self.jobman.enqueue_transfer(kind, paths, dest_dir);
        self.show_jobs = true;
    }


    fn apply_rename_result(&mut self, idx: usize) {
        if let Some((old, new, do_commit)) = self.panes[idx].rename_done.take() {
            if do_commit && !new.is_empty() && new != old {
                let dir = self.panes[idx].history.current.clone();
                let from = dir.join(&old);
                let to = dir.join(&new);
                if let Err(e) = std::fs::rename(&from, &to) {
                    self.panes[idx].last_error = Some(format!("rename: {e}"));
                    self.panes[idx].err_ttl = 480;
                } else {
                    self.tagstore.rename(&from.display().to_string(), &to.display().to_string());
                    self.tag_cache = self.tagstore.all();
                }
            }
            self.panes[idx].reload();
        }
    }

    // ============================ PREVIEW ============================
    fn preview_panel(&mut self, ctx: &egui::Context) {
        // Only when exactly one image file is selected in active pane.
        let sel_path = {
            let p = &self.panes[self.settings.active_pane.min(1)];
            if p.selected.len() != 1 { None } else {
                p.entries.iter().find(|e| p.selected.contains(&e.name))
                    .filter(|e| !e.is_dir && crate::thumbs::ThumbStore::is_image(&e.path))
                    .map(|e| e.path.clone())
            }
        };
        // Always show the panel while exactly one item is selected.
        if sel_path.is_none() {
            let any_single = {
                let p = &self.panes[self.settings.active_pane.min(1)];
                if p.selected.len() != 1 { None } else {
                    p.entries.iter().find(|e| p.selected.contains(&e.name)).map(|e| (e.path.clone(), e.is_dir, e.size))
                }
            };
            if let Some((path, is_dir, size)) = any_single {
                egui::SidePanel::right("preview_panel")
                    .resizable(false).exact_width(300.0)
                    .show(ctx, |ui| {
                        ui.label(egui::RichText::new(" PREVIEW").strong().small());
                        ui.separator();
                        ui.label(if is_dir { "\u{1F4C1} Folder" } else { "\u{1F4C4} File" });
                        ui.small(format!("{} bytes", size));
                        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                            ui.small(path.display().to_string());
                        });
                    });
            }
            return;
        }

        // Text fallback when no image selected.
        let sel_text = {
            let p = &self.panes[self.settings.active_pane.min(1)];
            if p.selected.len() != 1 { None } else {
                p.entries.iter().find(|e| p.selected.contains(&e.name))
                    .filter(|e| !e.is_dir && !crate::thumbs::ThumbStore::is_image(&e.path)
                        && e.size < 2_000_000
                        && is_text_ish(&e.path))
                    .map(|e| e.path.clone())
            }
        };
        let Some(path) = sel_path.or(sel_text) else {
            self.preview_tex = None;
            return;
        };
        let is_img = crate::thumbs::ThumbStore::is_image(&path);

        if !is_img {
            // TEXT PREVIEW
            egui::SidePanel::right("preview_panel")
                .resizable(false).exact_width(300.0)
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new(" PREVIEW").strong().small());
                    ui.separator();
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            let mut txt = String::from_utf8_lossy(&bytes[..bytes.len().min(120_000)]).into_owned();
                            egui::ScrollArea::both().auto_shrink([false,false]).show(ui, |ui| {
                                ui.add(egui::TextEdit::multiline(&mut txt)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_rows(24)
                                    .interactive(false));
                            });
                        }
                        Err(e) => { ui.colored_label(egui::Color32::RED, format!("{e}")); }
                    }
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                        ui.small(path.display().to_string());
                    });
                });
            return;
        }

        let c2 = ctx.clone();
        self.previews.request(path.clone(), move || c2.request_repaint());
        if self.preview_tex.as_ref().map(|(p, _)| *p != path).unwrap_or(true) {
            // Load full-resolution image for preview (not the 360px thumbnail).
            if let Ok(img_bytes) = std::fs::read(&path) {
                if let Ok(img) = image::load_from_memory(&img_bytes) {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    let tex = ctx.load_texture(
                        format!("preview_{}", path.display()),
                        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
                        egui::TextureOptions::LINEAR,
                    );
                    self.preview_tex = Some((path.clone(), tex));
                    // Don't return — fall through to show panel
                } else {
                    return; // decode failed
                }
            } else {
                return; // read failed
            }
        }

        egui::SidePanel::right("preview_panel")
            .resizable(false)
            .exact_width(300.0)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(" PREVIEW").strong().small());
                ui.separator();
                if let Some((_, tex)) = &self.preview_tex {
                    let avail_w = ui.available_width();
                    let avail_h = ui.available_height() - 30.0; // room for bottom path label
                    let tw = tex.size()[0] as f32;
                    let th = tex.size()[1] as f32;
                    let scale = (avail_w / tw).min(avail_h / th).max(0.01);
                    let size = [tw * scale, th * scale];
                    ui.add_space(4.0);
                                        let tid = tex.id();
                    let img = egui::Image::new(egui::load::SizedTexture::new(
                        tid,
                        egui::vec2(size[0], size[1]),
                    ));
                    ui.add(img);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if let Some((p, _)) = &self.preview_tex {
                        ui.small(p.display().to_string());
                    }
                });
            });
    }

    // ============================ JOBS WINDOW ============================
    fn draw_jobs(&mut self, ctx: &egui::Context) {
        if !self.show_jobs { return; }
        self.jobman.prune_finished();
        let snap = self.jobman.snapshot();
        if snap.is_empty() { self.show_jobs = false; return; }

        let mut any_running = false;
        egui::Window::new("\u{1F4E6} Operations")
            .open(&mut self.show_jobs)
            .default_width(420.0)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -30.0])
            .show(ctx, |ui| {
                for jarc in &snap {
                    let Ok(j) = jarc.lock() else { continue };
                    if j.state == crate::jobs::JobState::Running { any_running = true; }
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&j.label).small());
                    });
                    let frac = if j.total > 0 {
                        (j.done.load(std::sync::atomic::Ordering::Relaxed) as f32 / j.total as f32).clamp(0.0, 1.0)
                    } else { 0.0 };
                    let bar = egui::ProgressBar::new(frac)
                        .show_percentage()
                        .desired_height(14.0);
                    ui.add(bar);
                    ui.horizontal(|ui| {
                        match j.state {
                            crate::jobs::JobState::Running => {
                                if ui.small_button("Cancel").clicked() {
                                    j.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                }
                                ui.small(format!("{}/{} bytes", j.done.load(std::sync::atomic::Ordering::Relaxed), j.total));
                            }
                            crate::jobs::JobState::Done => { ui.colored_label(egui::Color32::from_rgb(0x77,0xd1,0x7a), "done"); }
                            crate::jobs::JobState::Cancelled => { ui.small("cancelled"); }
                            crate::jobs::JobState::Failed => {
                                if let Some(e)=&j.error { ui.colored_label(egui::Color32::from_rgb(0xE8,0x5D,0x75), e); }
                            }
                        }
                    });
                    ui.separator();
                }
            });
        if any_running {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    /// Open user's terminal emulator at the active pane's directory.
    fn open_terminal_here(&mut self) {
        let dir = self.panes[self.settings.active_pane.min(1)].history.current.clone();
        let candidates: Vec<(&str, Vec<&str>)> = vec![
            ("x-terminal-emulator", vec!["-e"]),
            ("gnome-terminal", vec!["--working-directory"]),
            ("xfce4-terminal", vec!["--working-directory"]),
            ("konsole", vec!["--workdir"]),
            ("xterm", vec![]),
        ];
        for (bin, args) in candidates {
            if which_exists(bin) {
                let d = dir.clone();
                let mut cmd = std::process::Command::new(bin);
                if bin == "xterm" {
                    cmd.args(["-e", "bash"]).current_dir(&d);
                } else if args.is_empty() {
                    cmd.current_dir(&d);
                } else {
                    cmd.arg(args[0]).arg(&d);
                    if bin == "x-terminal-emulator" {
                        cmd.arg("bash");
                    }
                }
                match cmd.spawn() {
                    Ok(_) => return,
                    Err(_) => continue,
                }
            }
        }
        let p = &mut self.panes[self.settings.active_pane.min(1)];
        p.last_error = Some("No terminal emulator found".into());
        p.err_ttl = 480;
    }

    fn selected_names_empty(&self, idx: usize) -> bool {
        self.panes[idx].selected.is_empty()
    }

    fn open_properties(&mut self) {
        let pane = &self.panes[self.settings.active_pane.min(1)];
        let Some(path) = selected_paths(pane).into_iter().next() else { return };
        let Ok(md) = std::fs::metadata(&path) else { return };
        let is_dir = md.is_dir();
        let size_s = if is_dir {
            match self.dir_sizes.get(&path) {
                Some(n) => browser::format_size(n),
                None => "\u{2026}".into(),
            }
        } else {
            browser::format_size(md.len())
        };
        if is_dir {
            if let Some(cx) = self.egui_ctx_for_props.clone() {
                self.dir_sizes.request(&path, browser::egui_ctx::Ctx(cx));
            }
        }
        let perms = md.permissions();
        let props = Dialog::Properties {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            path: path.display().to_string(),
            is_dir,
            size_s,
            modified_s: browser::format_time(browser::secs_of(&md.modified())),
            created_s: browser::format_time(browser::secs_of_created(&md)),
            perms_s: {
                #[cfg(unix)]
                { format!("{:o}", std::os::unix::fs::PermissionsExt::mode(&perms)) }
                #[cfg(not(unix))]
                { if perms.readonly() { "r--".into() } else { "rw-".into() } }
            },
            readonly_fs: perms.readonly(),
            items: None,
        };
        self.dialog = props;
    }

    // ============================ SEARCH ============================
    fn draw_search(&mut self, ctx: &egui::Context) {
        let Some(sr) = &mut self.search else { return };
        sr.maybe_start();

        let mut open = true;
        let mut go_to: Option<PathBuf> = None;
        let results = sr.snapshot();
        let searched = sr.searched_for.lock().map(|q| q.clone()).unwrap_or_default();
        let running = !sr.done.load(std::sync::atomic::Ordering::Relaxed);

        egui::Window::new(format!("\u{1F50D} Search in {}", sr.dir.display()))
            .open(&mut open)
            .default_width(560.0)
            .default_height(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Find:");
                    ui.add(
                        egui::TextEdit::singleline(&mut sr.input)
                            .hint_text("name contains... (min 2 chars)")
                            .desired_width(220.0),
                    )
                    .request_focus();
                    ui.label("ext:");
                    ui.add(egui::TextEdit::singleline(&mut sr.ext_filter)
                        .desired_width(60.0));
                    ui.label("min KB:");
                    ui.add(egui::DragValue::new(&mut sr.min_size_kb).speed(10));
                    ui.checkbox(&mut sr.content_grep, "content");
                    ui.separator();
                    if running {
                        ui.spinner();
                        ui.small("searching...");
                        ctx.request_repaint_after(std::time::Duration::from_millis(200));
                    } else if !searched.is_empty() {
                        ui.small("done");
                    }
                });
                ui.small(format!(
                    "{} result{}{}",
                    results.len(),
                    if results.len() == 1 { "" } else { "s" },
                    if results.len() >= crate::app_search::MAX_RESULTS { " (capped)" } else { "" }
                ));
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for p in results.iter().take(400) {
                        let label = p.display().to_string();
                        if ui
                            .add(
                                egui::Label::new(egui::RichText::new(&label).small())
                                    .sense(egui::Sense::click()),
                            )
                            .clicked()
                        {
                            go_to = Some(p.clone());
                        }
                    }
                });
            });

        if !open || go_to.is_some() {
            if let Some(sr) = self.search.take() {
                sr.cancel();
            }
        }
        if let Some(p) = go_to {
            self.active_pane().navigate(p);
        }
    }
}

impl eframe::App for HyperDriveApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let h = self.settings.show_hidden;
        self.panes[0].show_hidden = h;
        self.panes[1].show_hidden = h;
        if !self.pane_sort_seeded {
            for (i, (k, asc)) in self.settings.pane_sort.iter().enumerate().take(2) {
                let key = match k { 1 => SortKey::Size, 2 => SortKey::Modified, 3 => SortKey::Created, _ => SortKey::Name };
                self.panes[i].sort_key = key;
                self.panes[i].sort_asc = *asc;
                self.panes[i].apply_sort_pub();
            }
            self.pane_sort_seeded = true;
        }
        self.egui_ctx_for_props = Some(ctx.clone());
        self.handle_hotkeys(ctx);
        self.menu_bar(ctx);
        self.toolbar(ctx);
        self.playback_bar(ctx);
        self.statusbar(ctx);
        self.draw_jobs(ctx);
        self.audio.poll();
        if let Some(e) = self.audio.last_err.take() {
            self.panes[self.settings.active_pane.min(1)].last_error = Some(e);
            self.panes[self.settings.active_pane.min(1)].err_ttl = 480;
        }
        self.audio.set_volume(self.volume);
        if self.audio.path.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if self.settings.show_tree {
            self.tree_sidebar(ctx);
        }
        if self.show_settings {
            self.settings_window(ctx);
        }
        self.draw_dialog(ctx);
        self.draw_search(ctx);
        if self.settings.dual_pane {
            self.right_pane_panel(ctx);
        }
        if self.settings.show_preview {
            self.preview_panel(ctx);
        }
        self.central(ctx);
        self.focus_path_req = false;
        sync_session(self);
        self.handle_dnd(ctx);
    }
}

impl HyperDriveApp {
    fn handle_dnd(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering;
        let _ = Ordering::Relaxed;
        let pos = ctx.input(|i| i.pointer.latest_pos());
        let released = ctx.input(|i| i.pointer.primary_released());

        // Promote row presses into drags once movement exceeds threshold.
        for i in 0..2 {
            if self.dnd.is_none() {
                if let Some(origin) = self.panes[i].press_origin.take() {
                    let paths = self.panes[i].pending_press_paths.take().unwrap_or_default();
                    self.dnd = Some((i, paths, origin.0));
                }
            } else {
                let _ = self.panes[i].press_origin.take();
                let _ = self.panes[i].pending_press_paths.take();
            }
        }

        if let Some((from, paths, origin)) = &mut self.dnd {
            let Some(cur) = pos else { return };
            if origin.distance(cur) < 12.0 && !released {
                return; // not yet a drag
            }
            // Destination detection
            self.dnd_hover_dst = None;
            if *from == 0 && !self.settings.dual_pane {
                // single pane: no external target
            } else {
                let dst = 1 - *from;
                if let Some(Some(r)) = Some(self.panes[dst].pane_list_rect.as_ref().map(|r| *r)) {
                    if r.contains(cur) {
                        self.dnd_hover_dst = Some(dst);
                    }
                }
            }
            // Ghost tooltip near cursor
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("hd_dnd"),
            ));
            let text = format!(
                "{} {}item(s)\n{}",
                paths.len(),
                if self.dnd_hover_dst.is_some() { "\u{27A4} " } else { "" },
                if self.dnd_hover_dst.is_some() { "release to COPY (Shift = move)" } else { "drag onto the other pane" }
            );
            painter.rect_filled(
                egui::Rect::from_min_size(cur + egui::vec2(10.0, 10.0), egui::vec2(190.0, 38.0)),
                4.0,
                egui::Color32::from_rgba_unmultiplied(20, 20, 26, 235),
            );
            painter.text(
                cur + egui::vec2(16.0, 18.0),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
            ctx.request_repaint_after(std::time::Duration::from_millis(33));

            if released {
                let shift = ctx.input(|i| i.modifiers.shift);
                if let Some(dst) = self.dnd_hover_dst.take() {
                    let kind = if shift { crate::jobs::JobKind::Move } else { crate::jobs::JobKind::Copy };
                    let dest_dir = self.panes[dst].history.current.clone();
                    let src_list = paths.clone();
                    self.jobman.enqueue_transfer(kind, src_list, dest_dir);
                    self.show_jobs = true;
                }
                self.dnd = None;
            }
        }
    }


}

fn libc_like_uid() -> u32 {
    // $UID is always set on Linux desktop sessions; fallback to reading /proc/self.
    std::env::var("UID").ok().and_then(|v| v.parse().ok()).unwrap_or(1000)
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn build_share_script(path: &str, name: &str, comment: &str, readonly: bool, guest: bool) -> String {
    let subnet_note = "adjust hosts allow to your real subnet if different";
    format!(
r#"#!/usr/bin/env bash
# HyperDrive safe-share installer for "{name}"
# Generated {date}. Review every line before running with sudo.
set -euo pipefail

SHARE_NAME="{name}"
SHARE_PATH="{path}"
SMB_CONF=/etc/samba/smb.conf

echo "== 1/5 Checking prerequisites =="
command -v smbd >/dev/null || {{ echo "Install samba first:  sudo apt install samba"; exit 1; }}
[ -d "$SHARE_PATH" ] || {{ echo "Folder missing: $SHARE_PATH"; exit 1; }}

echo "== 2/5 Removing any previous block [$SHARE_NAME] =="
sudo sed -i "/^\[$SHARE_NAME\]/,/^(^$|^\[.*\]$)/{{/^\[$SHARE_NAME\]/d}}" "$SMB_CONF" || true
sudo sed -i "/^\[$SHARE_NAME\]$/,/^$/d" "$SMB_CONF" 2>/dev/null || true

echo "== 3/5 Appending share block =="
sudo tee -a "$SMB_CONF" >/dev/null <<EOF

[$SHARE_NAME]
   path = $SHARE_PATH
   comment = {comment}
   browseable = yes
   read only = {ro}
   guest ok = {guest}
   # Limit exposure to local LAN only ({subnet_note}):
   hosts allow = 192.168.0.0/16 127.0.0.1
   create mask = 0644
   directory mask = 0755
EOF

echo "== 4/5 Validating config =="
testparm -s "$SMB_CONF" >/dev/null || {{ echo "testparm FAILED - fix errors above"; exit 1; }}

echo "== 5/5 Enabling service + firewall =="
sudo systemctl enable --now smbd
if command -v ufw >/dev/null && sudo ufw status | grep -q "active"; then
    sudo ufw allow Samba
    echo "ufw: Samba allowed"
fi
{passwd_hint}
echo "DONE. Connect from Windows: \\\\<this-machine-ip>\\{name}"
"#,
        date = "2026-08-25",
        ro = if readonly { "yes" } else { "no" },
        guest = if guest { "yes" } else { "no" },
        passwd_hint = if guest { String::new() } else {
            r#"
echo "NOTE: non-guest shares need a Samba password:"
echo "      sudo smbpasswd -a $USER""#.to_string()
        },
    )
}

impl HyperDriveApp {
    fn do_duplicates(&mut self, pairs: Vec<(PathBuf, PathBuf)>) {
        let dir = self.panes[self.settings.active_pane.min(1)].history.current.clone();
        let sources: Vec<PathBuf> = pairs.iter().map(|(s, _)| s.clone()).collect();
        self.jobman.enqueue_transfer(crate::jobs::JobKind::Copy, sources, dir);
        self.show_jobs = true;
    }
}

fn dup_targets(paths: Vec<PathBuf>) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for p in paths {
        let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let dst = dir.join(format!("{stem} - Copy{ext}"));
        out.push((p, dst));
    }
    out
}

fn sync_session(app: &mut HyperDriveApp) {
    let p0 = app.panes[0].history.current.display().to_string();
    let p1 = app.panes[1].history.current.display().to_string();
    let changed_dirs =
        app.settings.pane_paths[0] != p0 || app.settings.pane_paths[1] != p1;
    let cur_tabs = [
        app.side_tabs[0]
            .iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(";"),
        app.side_tabs[1]
            .iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(";"),
    ];
    let cur_sorts: [(usize, bool); 2] = [
        (app.panes[0].sort_key as usize, app.panes[0].sort_asc),
        (app.panes[1].sort_key as usize, app.panes[1].sort_asc),
    ];
    let changed_sorts =
        app.settings.pane_sort != cur_sorts;
    let changed_tabs = app.settings.side_tabs[0].join(";") != cur_tabs[0]
        || app.settings.side_tabs[1].join(";") != cur_tabs[1];
    if changed_dirs || changed_tabs || changed_sorts {
        if changed_sorts { app.settings.pane_sort = cur_sorts; }
        app.settings.pane_paths = [p0, p1];
        app.settings.side_tabs = [
            cur_tabs[0]
                .split(';')
                .filter(|x| !x.is_empty())
                .map(|x| x.to_string())
                .collect(),
            cur_tabs[1]
                .split(';')
                .filter(|x| !x.is_empty())
                .map(|x| x.to_string())
                .collect(),
        ];
        let _ = app.settings.save();
    }
}

impl HyperDriveApp {
    /// Open a file honoring per-extension default apps, else system opener.
    pub fn smart_open(&mut self, path: &PathBuf) {
        let ext = path.extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        if is_audio_file(&path.display().to_string()) && !self.settings.openwith.iter().any(|(e, _)| e == &ext) {
            let _ = self.audio.play(path);
            return;
        }
        if let Some((_, dp)) = self.settings.openwith.iter().find(|(e, _)| *e == ext) {
            let dp = dp.clone();
            let p = path.clone();
            let _ = std::process::Command::new("gio").arg("launch").arg(dp).arg(p).spawn();
            return;
        }
        if path.to_string_lossy().contains('!') {
            match crate::browser::extract_zip_entry(path) {
                Ok(out) => {
                    let p = out.clone();
                    browser::open_with_system(&p);
                }
                Err(e) => {
                    self.active_pane().last_error = Some(format!("archive: {e}"));
                }
            }
            return;
        }
        browser::open_with_system(path);
    }
}

fn first_selected(pane: &Pane) -> Option<String> {
    pane.selected.iter().next().cloned()
}
fn first_selected_path(pane: &Pane) -> Option<PathBuf> {
    pane.entries.iter().find(|e| pane.selected.contains(&e.name)).map(|e| e.path.clone())
}

fn which_exists(bin: &str) -> bool {
    std::env::var("PATH")
        .map(|paths| {
            paths.split(':').any(|d| {
                std::path::Path::new(d).join(bin).is_file()
            })
        })
        .unwrap_or(false)
}

/// Scan application launchers for the Open With dialog.
fn list_desktop_apps() -> Vec<(String, String, String)> {
    let dirs = [
        std::path::PathBuf::from("/usr/share/applications"),
        xdg_local_apps(),
    ];
    let mut out: Vec<(String, String, String)> = Vec::new();
    for d in dirs.iter() {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "desktop").unwrap_or(true) { continue; }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let mut name = String::new();
            let mut exec = String::new();
            let mut hidden = false;
            for line in text.lines() {
                if let Some(v) = line.strip_prefix("Name=") { if name.is_empty() { name = v.to_string(); } }
                else if let Some(v) = line.strip_prefix("Exec=") { if exec.is_empty() { exec = v.to_string(); } }
                else if line.trim().eq_ignore_ascii_case("Hidden=true") || line.trim().eq_ignore_ascii_case("NoDisplay=true") { hidden = true; }
            }
            if !hidden && !name.is_empty() && !exec.is_empty() {
                out.push((name, p.display().to_string(), exec));
            }
        }
    }
    out.sort_by_key(|a| a.0.to_lowercase());
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn xdg_local_apps() -> std::path::PathBuf {
    browser::home_dir().join(".local/share/applications")
}

/// Discover SMB servers: mDNS first (Apple/Linux), then GVFS workgroup
/// enumeration which is how Windows machines are actually visible.
fn scan_smb_hosts() -> Vec<(String, String)> {
    let mut hosts = scan_smb_avahi();
    if hosts.is_empty() {
        hosts = scan_smb_gvfs();
    }
    hosts.sort();
    hosts
}

/// Enumerate servers from the GVFS workgroup view: `gio list smb:///`.
fn scan_smb_gvfs() -> Vec<(String, String)> {
    use std::process::Command;
    let Ok(out) = Command::new("gio").arg("list").arg("smb:///").output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut v = Vec::new();
    for line in text.lines() {
        // Lines look like: smb://workgroup%3Buser@server/  or  smb://server/
        if !line.starts_with("smb://") { continue; }
        let mut host = line
            .trim_start_matches("smb://")
            .trim_end_matches('/')
            .to_string();
        if let Some(at) = host.rfind('@') { host = host[at + 1..].to_string(); }
        if host.is_empty() { continue; }
        if !v.iter().any(|(h, _): &(String, String)| h == &host) {
            v.push((host.clone(), String::new())); // IP resolved at mount time
        }
    }
    v
}

/// List shares on one server via GVFS: `gio list smb://server/`.
fn scan_smb_shares(server: &str) -> Vec<String> {
    use std::process::Command;
    let uri = format!("smb://{}/", server);
    let Ok(out) = Command::new("gio").arg("list").arg(&uri).output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut v = Vec::new();
    for line in text.lines() {
        let name = line
            .trim_start_matches(&format!("smb://{server}/"))
            .trim_end_matches('/')
            .to_string();
        if !name.is_empty() && !v.contains(&name) {
            v.push(name);
        }
    }
    v
}

#[allow(dead_code)]
fn scan_smb_avahi_old_unused() {}

/// Original mDNS scan kept as first choice.
fn scan_smb_avahi() -> Vec<(String, String)> {
    use std::process::Command;
    let out = Command::new("avahi-browse")
        .args(["-rtp", "_smb._tcp"])
        .output();
    let Ok(o) = out else { return Vec::new() };
    let text = String::from_utf8_lossy(&o.stdout);
    let mut hosts: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        // format: =  ;eth0;IPv4;hostname;_smb._tcp;local;name;ip;port
        let parts: Vec<&str> = line.split(';').collect();
        if parts.len() >= 9 && parts[0] == "=" && parts[2].contains("IPv4") {
            let host = parts[3].to_string();
            let ip = parts[7].to_string();
            if !hosts.iter().any(|(h, _)| h == &host) {
                hosts.push((host, ip));
            }
        }
    }
    hosts.sort();
    hosts
}

fn free_disk_gib(dir: &Path) -> String {
    use std::process::Command;
    let out = Command::new("df").arg("-BG").arg("--output=avail").arg(dir).output();
    if let Ok(o) = out {
        let t = String::from_utf8_lossy(&o.stdout);
        if let Some(line) = t.lines().nth(1) {
            return line.trim().trim_end_matches('G').to_string() + " GB";
        }
    }
    "?".into()
}

fn is_text_ish(p: &Path) -> bool {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(ext.as_str(),
        "txt" | "md" | "rs" | "c" | "h" | "cpp" | "hpp" | "py" | "js" | "ts" |
        "html" | "css" | "json" | "toml" | "yaml" | "yml" | "ini" | "conf" |
        "sh" | "bash" | "log" | "csv" | "xml" | "svg" | "desktop" | "service")
}

/// List one SMB directory via smbclient guest access.
fn smb_ls(host: &str, share: &str, path: &str, err_out: &mut String) -> Vec<(String, bool, u64)> {
    use std::process::Command;
    let cd = if path.is_empty() { String::new() } else { format!("cd \"{path}\"; ") };
    let out = Command::new("timeout").arg("20s")
        .arg("smbclient").arg(format!("//{host}/{share}")).arg("-N")
        .arg("-c").arg(format!("{cd}ls"))
        .output();
    *err_out = String::new();
    let Ok(o) = out else { *err_out = "smbclient not found".into(); return Vec::new(); };
    if !o.status.success() {
        *err_out = String::from_utf8_lossy(&o.stderr).trim().to_string();
        if err_out.is_empty() { *err_out = "connection failed".into(); }
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&o.stdout);
    let mut v: Vec<(String,bool,u64)> = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        if t.is_empty() || t.starts_with('.') || t.contains("blocks available") { continue; }
        let tok: Vec<&str> = t.split_whitespace().collect();
        if tok.len() < 8 { continue; }
        let n = tok.len();
        // [name...] attr size weekday month day hh:mm:ss year
        let attr = tok[n-7];
        let size: u64 = tok[n-6].parse().unwrap_or(0);
        let name = tok[..n-7].join(" ");
        if name.is_empty() { continue; }
        let is_dir = attr.contains('D');
        v.push((name, is_dir, size));
    }
    v.sort_by(|a,b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    v
}