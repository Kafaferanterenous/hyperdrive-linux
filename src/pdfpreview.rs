//! Built-in PDF preview: renders PDF pages via the bundled Pdfium engine into
//! egui textures, lazily (only pages actually on screen are rasterized).
//!
//! Pdfium (https://pdfium.googlesource.com/) is the PDF engine used by the
//! Chromium/Chrome and Edge browsers. Its shared library rides inside the
//! AppImage so HyperDrive stays self-contained.
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use eframe::egui::{ColorImage, TextureHandle, TextureOptions};
use pdfium_render::prelude::*;

/// Max number of page textures held at once. Older pages are evicted LRU-style
/// so a long document never eats unbounded RAM.
const MAX_CACHED_PAGES: usize = 24;

/// Render width for a single page in logical px (before zoom).
const RENDER_WIDTH: f32 = 1100.0;

/// PdfDocument borrows the Pdfium instance, so the engine lives for the whole
/// process in a OnceLock and documents get a 'static lifetime.
fn pdfium() -> Option<&'static Pdfium> {
    static ENGINE: OnceLock<Option<Pdfium>> = OnceLock::new();
    ENGINE.get_or_init(load_engine).as_ref()
}

fn load_engine() -> Option<Pdfium> {
    let mut candidate: Option<PathBuf> = None;
    if let Ok(p) = std::env::var("PDFIUM_LIB_PATH") {
        let p = PathBuf::from(p);
        candidate = Some(if p.is_dir() {
            p.join(Pdfium::pdfium_platform_library_name())
        } else {
            p
        });
    } else if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["libpdfium.so", "pdfium.so"] {
                let p = dir.join(name);
                if p.exists() {
                    candidate = Some(p);
                    break;
                }
            }
        }
    }
    if let Some(p) = candidate {
        if let Ok(b) = Pdfium::bind_to_library(p) {
            return Some(Pdfium::new(b));
        }
    }
    Pdfium::bind_to_system_library()
        .ok()
        .map(Pdfium::new)
}

pub struct PdfPreview {
    doc: Option<PdfDocument<'static>>,
    cache: HashMap<i32, TextureHandle>,
    /// most recently used page indices at the front (for eviction)
    lru: Vec<i32>,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
    pub page: i32,
    pub pages: i32,
    pub zoom: f32,
}

impl Default for PdfPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfPreview {
    pub fn new() -> Self {
        Self {
            doc: None,
            cache: HashMap::new(),
            lru: Vec::new(),
            path: None,
            error: None,
            page: 0,
            pages: 0,
            zoom: 1.0,
        }
    }

    pub fn open(&mut self, path: &Path) {
        self.close();
        self.path = Some(path.to_path_buf());
        let Some(engine) = pdfium() else {
            self.error = Some("PDFium library not found".into());
            return;
        };
        match engine.load_pdf_from_file(path, None) {
            Ok(doc) => {
                self.doc = Some(doc);
                self.pages = self.doc.as_ref().map(|d| d.pages().len()).unwrap_or(0);
            }
            Err(_) => self.error = Some("Could not open this PDF".into()),
        }
    }

    pub fn close(&mut self) {
        self.doc = None;
        self.cache.clear();
        self.lru.clear();
        self.path = None;
        self.error = None;
        self.page = 0;
        self.pages = 0;
        self.zoom = 1.0;
    }

    fn evict_lru(&mut self) {
        while self.cache.len() > MAX_CACHED_PAGES {
            if let Some(old) = self.lru.pop() {
                self.cache.remove(&old);
            }
        }
    }

    /// Rasterize `index` into a texture if it is not already cached.
    /// Returns false when the page cannot be rendered.
    pub fn ensure_page(&mut self, ctx: &eframe::egui::Context, index: i32) -> bool {
        if self.cache.contains_key(&index) {
            self.lru.retain(|&p| p != index);
            self.lru.insert(0, index);
            return true;
        }
        let Some(doc) = self.doc.as_ref() else {
            return false;
        };
        let Ok(page) = doc.pages().get(index) else {
            return false;
        };
        let target = (RENDER_WIDTH * self.zoom) as i32;
        let config = PdfRenderConfig::new().set_target_width(target);
        let Ok(bitmap) = page.render_with_config(&config) else {
            return false;
        };
        let Ok(image) = bitmap.as_image() else {
            return false;
        };
        let rgba = image.to_rgba8();
        let (w, h) = rgba.dimensions();
        let color_image =
            ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
        let texture = ctx.load_texture(index.to_string(), color_image, TextureOptions::LINEAR);
        self.cache.insert(index, texture);
        self.lru.retain(|&p| p != index);
        self.lru.insert(0, index);
        self.evict_lru();
        true
    }

    /// Screen-space dimensions (in egui points) of a cached page.
    pub fn cached_size(&self, index: i32) -> Option<eframe::egui::Vec2> {
        self.cache.get(&index).map(|t| t.size_vec2())
    }

    /// Texture handle of a cached page, if already rasterized.
    pub fn cached_texture(&self, index: i32) -> Option<&TextureHandle> {
        self.cache.get(&index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid single-page PDF (a filled square) as bytes, with a
    /// correct xref table so Pdfium parses it without repair.
    fn minimal_pdf() -> Vec<u8> {
        let content = b"0 0 200 200 re f\n";
        let mut objs: Vec<Vec<u8>> = Vec::new();
        objs.push(format!(
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n"
        ).into_bytes());
        objs.push(format!(
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n"
        ).into_bytes());
        objs.push(format!(
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>\nendobj\n"
        ).into_bytes());
        objs.push(format!(
            "4 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            content.len(),
            String::from_utf8_lossy(content),
        ).into_bytes());

        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.4\n");
        let mut offsets = Vec::new();
        for o in &objs {
            offsets.push(out.len() as u64);
            out.extend_from_slice(o);
        }
        let xref_pos = out.len() as u64;
        out.extend_from_slice(format!("xref\n0 {}\n", objs.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for off in &offsets {
            out.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
        }
        out.extend_from_slice(format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objs.len() + 1,
            xref_pos
        ).as_bytes());
        out
    }

    /// Renders the first page of a real PDF into an egui texture using the
    /// same code path the GUI preview uses, but headless (no window).
    /// Skips silently when the Pdfium library is not available at runtime:
    /// `cargo test --release` with PDFIUM_LIB_PATH set exercises it fully.
    #[test]
    fn renders_first_page_headless() {
        let Some(_) = pdfium() else {
            eprintln!("PDFium library not available; set PDFIUM_LIB_PATH to run this test");
            return;
        };
        let pdf = minimal_pdf();
        let dir = std::env::temp_dir().join("hyperdrive-pdf-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("one_page.pdf");
        std::fs::write(&path, &pdf).unwrap();

        let mut pv = PdfPreview::new();
        pv.open(&path);
        assert_eq!(pv.pages, 1, "should parse exactly one page");
        assert!(pv.error.is_none(), "no error expected: {:?}", pv.error);

        let ctx = eframe::egui::Context::default();
        assert!(pv.ensure_page(&ctx, 0), "page 0 must rasterize");
        let size = pv.cached_size(0).expect("texture should be cached");
        assert!(size.x > 10.0, "rendered page should be wider than 10px: {size:?}");
        assert!(size.y > 10.0);

        // Non-existent page must not create a texture.
        assert!(!pv.ensure_page(&ctx, 99));
    }
}