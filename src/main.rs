// No console window behind the app on Windows (debug builds keep it, for --timings output).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ai;
mod app;
mod color;
mod curves;
mod dedupe;
mod edit;
mod exif_edit;
mod files;
mod heif;
mod i18n;
mod inspector;
mod resize_optimally;
mod jpegxl;
mod load;
mod markup;
mod platform;
mod raw;
mod retouch;
mod save;
mod settings;
mod svg;
mod theme;
mod tiff;
mod turbo;
mod view;

use eframe::egui;
use load::{Loader, Notify};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Instant;

fn main() -> eframe::Result {
    let t0 = Instant::now();
    let (mut timings, mut bench, mut path) = (false, false, None);
    // `args()` panics on a file name that isn't valid Unicode.
    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("--timings") => timings = true,
            Some("--bench") => (timings, bench) = (true, true),
            _ => path = Some(arg),
        }
    }
    let path = path.map(|p| files::canonicalize(p.as_ref()).unwrap_or_else(|e| {
        eprintln!("lightview: {}: {e}", p.display());
        std::process::exit(1);
    }));

    let mut settings = settings::load();
    i18n::set(*settings.language.get_or_insert_with(i18n::system_language));

    // Decoding starts before the window exists so it overlaps OpenGL/eframe setup.
    let (tx, rx) = mpsc::channel();
    let notify = Notify { tx, ctx: Arc::new(OnceLock::new()) };
    // winit reports the system theme on Windows only, so egui's fallback comes from the desktop portal,
    // read while the window opens. shortcut: read once per start; watch the portal's SettingChanged to follow live.
    #[cfg(target_os = "linux")]
    {
        let ctx = notify.ctx.clone();
        std::thread::spawn(move || if let Some(theme) = platform::system_theme() {
            ctx.wait().options_mut(|o| o.fallback_theme = theme);
            ctx.wait().request_repaint();
        });
    }
    // The first decode starts before the window exists, so it targets the configured window size.
    let loader = Loader::new(notify.clone(), 3, settings.window_size.map(|v| v as u32), settings.color_managed);
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
