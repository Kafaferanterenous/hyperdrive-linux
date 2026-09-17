//! HyperDrive - fast, portable, self-contained file manager.
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

mod app;
mod jobs;
mod thumbs;
mod tags;
mod app_search;
mod audio;
mod browser;
mod config;
mod theme;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let settings = config::Settings::load_or_default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1020.0, 680.0])
            .with_min_inner_size([640.0, 420.0])
            .with_title("HyperDrive"),
        ..Default::default()
    };

    eframe::run_native(
        "HyperDrive",
        options,
        Box::new(move |cc| {
            theme::apply_all(&cc.egui_ctx, &settings);
            Box::new(app::HyperDriveApp::new(settings))
        }),
    )
}
