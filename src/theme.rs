//! HyperDrive - themes and runtime font management.
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

use crate::config::{Settings, ThemeChoice};
use eframe::egui;
use eframe::egui::{Color32, Context, FontData, FontDefinitions, Style, TextStyle, Visuals};

/// Curated LEGIBLE fonts only (no decorative/curly faces).
/// Each entry lists candidate TTF paths per platform; first existing wins.
pub fn curated_fonts() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        (
            "DejaVu Sans",
            vec![
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "C:\\Windows\\Fonts\\DejaVuSans.ttf",
            ],
        ),
        (
            "Noto Sans",
            vec![
                "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                "C:\\Windows\\Fonts\\NotoSans-Regular.ttf",
            ],
        ),
        (
            "Liberation Sans",
            vec![
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                "C:\\Windows\\Fonts\\LiberationSans-Regular.ttf",
            ],
        ),
        (
            "Ubuntu",
            vec!["/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf"],
        ),
        ("Segoe UI", vec!["C:\\Windows\\Fonts\\segoeui.ttf"]),
    ]
}

pub fn font_file_for(family: &str) -> Option<std::path::PathBuf> {
    for (name, paths) in curated_fonts() {
        if name == family {
            for p in paths {
                let pb = std::path::PathBuf::from(p);
                if pb.is_file() {
                    return Some(pb);
                }
            }
        }
    }
    None
}

pub fn apply_all(ctx: &Context, settings: &Settings) {
    apply_theme(ctx, settings.theme);
    apply_text_size(ctx, settings.font_size);
    apply_font_family(ctx, &settings.font_family);
}

fn palette(theme: ThemeChoice) -> (Color32, Color32, Color32, Color32, Color32, Color32) {
    // (bg, bg2, extreme, alt_row, text, accent)
    match theme {
        ThemeChoice::Normal => (
            Color32::from_rgb(0xFF, 0xFF, 0xFF),
            Color32::from_rgb(0xF4, 0xF4, 0xF5),
            Color32::from_rgb(0xFF, 0xFF, 0xFF),
            Color32::from_rgb(0xF7, 0xF7, 0xF8),
            Color32::from_rgb(0x1A, 0x1A, 0x1A),
            Color32::from_rgb(0x25, 0x63, 0xEB),
        ),
        ThemeChoice::Dark => (
            Color32::from_rgb(0x1E, 0x1E, 0x1E),
            Color32::from_rgb(0x26, 0x26, 0x26),
            Color32::from_rgb(0x18, 0x18, 0x18),
            Color32::from_rgb(0x24, 0x24, 0x24),
            Color32::from_rgb(0xE6, 0xE6, 0xE6),
            Color32::from_rgb(0x4C, 0x9A, 0xFF),
        ),
        ThemeChoice::Pastel => (
            Color32::from_rgb(0xAC, 0xA7, 0xA0), // 32% darker warm grey
            Color32::from_rgb(0xAA, 0xA2, 0x98),
            Color32::from_rgb(0xAD, 0xAB, 0xA6),
            Color32::from_rgb(0xAA, 0xA4, 0x9B),
            Color32::from_rgb(0x3E, 0x37, 0x30),
            Color32::from_rgb(0x9E, 0x61, 0x6D),
        ),
        ThemeChoice::Blue => (
            Color32::from_rgb(0x09, 0x15, 0x29), // deep navy (45% darker)
            Color32::from_rgb(0x0C, 0x1A, 0x34),
            Color32::from_rgb(0x06, 0x10, 0x20),
            Color32::from_rgb(0x0B, 0x18, 0x2E),
            Color32::from_rgb(0xF8, 0xFA, 0xFF), // off-white text
            Color32::from_rgb(0x6A, 0xA6, 0xFF), // bright accent
        ),
    }
}

fn base_visuals(theme: ThemeChoice) -> Visuals {
    match theme {
        ThemeChoice::Dark => Visuals::dark(),
        _ => Visuals::light(),
    }
}

pub fn apply_theme(ctx: &Context, theme: ThemeChoice) {
    let (bg, bg2, extreme, alt_row, text, accent) = palette(theme);
    let mut v = base_visuals(theme);

    v.panel_fill = bg;
    v.window_fill = bg2;
    v.extreme_bg_color = extreme;
    v.faint_bg_color = alt_row;
    v.override_text_color = Some(text);
    v.hyperlink_color = accent;
    v.selection.bg_fill = accent.gamma_multiply(0.45);
    v.selection.stroke.color = accent;

    v.widgets.noninteractive.fg_stroke.color = text;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, separator_color(theme));
    v.widgets.inactive.fg_stroke.color = text;
    v.widgets.hovered.fg_stroke.color = text;
    v.widgets.active.fg_stroke.color = text;
    v.widgets.inactive.weak_bg_fill = bg2;

    ctx.set_visuals(v);
}

fn separator_color(theme: ThemeChoice) -> Color32 {
    match theme {
        ThemeChoice::Normal => Color32::from_rgb(0xDD, 0xDD, 0xDD),
        ThemeChoice::Dark => Color32::from_rgb(0x4A, 0x4A, 0x54),
        ThemeChoice::Pastel => Color32::from_rgb(0x97, 0x93, 0x8B),
        ThemeChoice::Blue => Color32::from_rgb(0x17, 0x29, 0x49),
    }
}

pub fn apply_text_size(ctx: &Context, base: f32) {
    let base = base.clamp(12.0, 28.0);
    let styles: Vec<(TextStyle, f32)> = vec![
        (TextStyle::Body, 1.0),
        (TextStyle::Button, 1.0),
        (TextStyle::Heading, 1.5),
        (TextStyle::Small, 0.8),
        (TextStyle::Monospace, 0.9),
    ];
    ctx.style_mut(|style: &mut Style| {
        for (key, factor) in styles {
            if let Some(ts) = style.text_styles.get_mut(&key) {
                ts.size = base * factor;
            }
        }
    });
}

pub fn apply_font_family(ctx: &Context, family: &str) {
    let mut fonts = FontDefinitions::default();
    if !family.is_empty() {
        if let Some(path) = font_file_for(family) {
            if let Ok(bytes) = std::fs::read(&path) {
                fonts
                    .font_data
                    .insert("hd_user_font".to_string(), FontData::from_owned(bytes));
                if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                    list.insert(0, "hd_user_font".to_string());
                }
            }
        }
    }
    ctx.set_fonts(fonts);
}
