mod app;
mod load;
mod view;

use eframe::egui;
use load::{Loader, Msg, Notify};
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

    // Decoding starts before the window exists so it overlaps OpenGL/eframe setup.
    let (tx, rx) = mpsc::channel();
    let notify = Notify { tx, ctx: Arc::new(OnceLock::new()) };
    let loader = Loader::new(notify.clone(), 3);
    if let Some(path) = &path {
        loader.want(vec![path.clone()]);
        let (path, notify) = (path.clone(), notify.clone());
        std::thread::spawn(move || notify.send(Msg::Folder(load::list_folder(&path))));
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native("lightview", options, Box::new(move |cc| {
        let _ = notify.ctx.set(cc.egui_ctx.clone());
        Ok(Box::new(app::App::new(cc, t0, timings, bench, rx, loader, path)))
    }))
}
