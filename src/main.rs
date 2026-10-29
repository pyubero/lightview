// No console window behind the app on Windows (debug builds keep it, for --timings output).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ai;
mod app;
mod edit;
mod exif_edit;
mod i18n;
mod inspector;
mod load;
mod markup;
mod platform;
mod save;
mod settings;
mod turbo;
mod view;

use eframe::egui;
use load::{Loader, Notify};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Instant;

fn main() -> eframe::Result {
    let t0 = Instant::now();
    let (mut timings, mut bench, mut path) = (false, false, None);
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--timings" => timings = true,
            "--bench" => (timings, bench) = (true, true),
            _ => path = Some(arg),
        }
    }
    let path = path.map(|p| std::fs::canonicalize(&p).unwrap_or_else(|e| {
        eprintln!("lightview: {p}: {e}");
        std::process::exit(1);
    }));

    let mut settings = settings::load();
    i18n::set(*settings.language.get_or_insert_with(i18n::system_language));

    // Decoding starts before the window exists so it overlaps OpenGL/eframe setup.
    let (tx, rx) = mpsc::channel();
    let notify = Notify { tx, ctx: Arc::new(OnceLock::new()) };
    // The first decode starts before the window exists, so it targets the configured window size.
    let loader = Loader::new(notify.clone(), 3, settings.window_size.map(|v| v as u32));
    if let Some(path) = &path {
        load::start(path, settings.sort, &loader, &notify);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(settings.window_size)
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../icon/lightview-256.png")).unwrap()),
        // Always open at the default size (brief); persistence is only for app data.
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native("lightview", options, Box::new(move |cc| {
        let _ = notify.ctx.set(cc.egui_ctx.clone());
        Ok(Box::new(app::App::new(cc, t0, timings, bench, rx, loader, notify, settings, path)))
    }))
}
