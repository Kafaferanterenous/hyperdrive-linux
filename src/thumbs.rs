//! HyperDrive - background thumbnail decoding for images.
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const THUMB_SIZE: u32 = 48;

#[derive(Clone)]
pub struct Decoded {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

#[derive(Default)]
pub struct ThumbStore {
    map: Arc<Mutex<HashMap<PathBuf, Option<Decoded>>>>,
    inflight: Arc<AtomicUsize>,
    size: u32,
}

const MAX_INFLIGHT: usize = 6;

impl ThumbStore {
    pub fn new() -> Self {
        Self { size: THUMB_SIZE, ..Default::default() }
    }

    /// Store that decodes at a custom max dimension (e.g. preview pane).
    pub fn with_size(size: u32) -> Self {
        Self { size, ..Default::default() }
    }

    pub fn is_image(path: &Path) -> bool {
        let n = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        matches!(n.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
    }

    /// None = pending/unknown; Some(d) = ready.
    pub fn get(&self, path: &Path) -> Option<Decoded> {
        self.map.lock().ok()?.get(path).cloned().flatten()
    }

    /// Queue decode if never requested.
    pub fn request(&self, path: PathBuf, repaint: impl Send + FnOnce() + 'static + Clone) {
        {
            let mut m = match self.map.lock() {
                Ok(m) => m,
                Err(_) => return,
            };
            if m.contains_key(&path) {
                return;
            }
            if self.inflight.load(Ordering::Relaxed) >= MAX_INFLIGHT {
                return; // try again next frame
            }
            m.insert(path.clone(), None);
        }
        self.inflight.fetch_add(1, Ordering::Relaxed);
        let store = Self {
            map: Arc::clone(&self.map),
            inflight: Arc::clone(&self.inflight),
            size: self.size,
        };
        let repaint2 = repaint.clone();
        std::thread::spawn(move || {
            let sz = store.size;
            let decoded = decode_thumb(&path, sz);
            if let Ok(mut m) = store.map.lock() {
                m.insert(path, decoded);
            }
            store.inflight.fetch_sub(1, Ordering::Relaxed);
            repaint2();
            drop(store); // keep clone alive semantics explicit
        });
    }
}

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var("HOME").ok()?;
    let d = PathBuf::from(base).join(".cache/hyperdrive/thumbs");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn cache_key(path: &Path) -> String {
    // cheap stable hash (FNV-1a 64)
    let mut h: u64 = 0xcbf29ce484222325;
    for b in path.to_string_lossy().as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}.png")
}

fn decode_thumb(path: &Path, target: u32) -> Option<Decoded> {
    // 1) disk cache hit?
    if let Some(cd) = cache_dir() {
        let cf = cd.join(cache_key(path));
        if let Ok(img) = image::ImageReader::open(&cf).map(|r| r.decode()) {
            if let Ok(thumb) = img.map(|i| i.to_rgba8()) {
                return Some(Decoded {
                    w: thumb.width(),
                    h: thumb.height(),
                    rgba: thumb.into_raw(),
                });
            }
        }
        // 2) decode source, downscale, save to disk
        let img = image::ImageReader::open(path).ok()?.decode().ok()?;
        let thumb = img.thumbnail(target, target).to_rgba8();
        let _ = thumb.save(&cf);
        return Some(Decoded {
            w: thumb.width(),
            h: thumb.height(),
            rgba: thumb.into_raw(),
        });
    }
    // no HOME/cache: memory-only
    let img = image::ImageReader::open(path).ok()?.decode().ok()?;
    let thumb = img.thumbnail(target, target).to_rgba8();
    Some(Decoded {
        w: thumb.width(),
        h: thumb.height(),
        rgba: thumb.into_raw(),
    })
}
