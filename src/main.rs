use eframe::egui::{self, Color32, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::{Image, ImageRef}};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Instant;

struct Decoded {
    w: u32,
    h: u32,
    // RGB unless the image has alpha: resizing RGBA is ~7× slower.
    pixels: Vec<u8>,
    alpha: bool,
    decode_ms: f64,
}

struct App {
    t0: Instant,
    bench: bool,
    rx: Option<mpsc::Receiver<Decoded>>,
    full: Option<Decoded>,
    texture: Option<TextureHandle>,
    tex_size: [u32; 2],
    resize_ms: f64,
    first_frame_ms: Option<f64>,
}

fn main() -> eframe::Result {
    let t0 = Instant::now();
    let mut bench = false;
    let mut path = None;
    for arg in std::env::args().skip(1) {
        if arg == "--bench" { bench = true } else { path = Some(arg) }
    }

    // Decode before the window exists so it overlaps OpenGL/eframe setup.
    let ctx_cell: Arc<OnceLock<egui::Context>> = Arc::default();
    let rx = path.map(|path| {
        let (tx, rx) = mpsc::channel();
        let ctx_cell = ctx_cell.clone();
        std::thread::spawn(move || {
            let t = Instant::now();
            let img = match image::open(&path) {
                Ok(img) => img,
                Err(e) => {
                    eprintln!("lightview: {path}: {e}");
                    std::process::exit(1);
                }
            };
            let (w, h, alpha) = (img.width(), img.height(), img.color().has_alpha());
            let pixels = if alpha { img.into_rgba8().into_raw() } else { img.into_rgb8().into_raw() };
            let _ = tx.send(Decoded { w, h, pixels, alpha, decode_ms: ms(t) });
            // If the context isn't set yet, the first frame picks the image up anyway.
            if let Some(ctx) = ctx_cell.get() {
                ctx.request_repaint();
            }
        });
        rx
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native("lightview", options, Box::new(move |cc| {
        let _ = ctx_cell.set(cc.egui_ctx.clone());
        Ok(Box::new(App {
            t0, bench, rx, full: None, texture: None, tex_size: [0, 0], resize_ms: 0.0, first_frame_ms: None,
        }))
    }))
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::BLACK.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(ms) = self.first_frame_ms {
            if self.bench {
                let decode_ms = self.full.as_ref().map_or(0.0, |d| d.decode_ms);
                eprintln!("decode {decode_ms:.1} ms, resize {:.1} ms, first frame {ms:.1} ms", self.resize_ms);
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            self.bench = false;
        }

        if let Some(d) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.full = Some(d);
            self.rx = None;
        }
        let Some(full) = &self.full else { return };

        let panel = ui.max_rect();
        let rect = fit_rect(full.w as f32, full.h as f32, panel);
        let ppp = ui.ctx().pixels_per_point();
        let size = [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32];
        if size != self.tex_size {
            // shortcut: resize runs on the UI thread on window resize; move it off-thread if resizing big images stutters (milestone 2).
            let t = Instant::now();
            let pixel_type = if full.alpha { PixelType::U8x4 } else { PixelType::U8x3 };
            let src = ImageRef::new(full.w, full.h, &full.pixels, pixel_type).unwrap();
            let mut dst = Image::new(size[0], size[1], pixel_type);
            let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));
            Resizer::new().resize(&src, &mut dst, &opts).unwrap();
            let color_size = [size[0] as usize, size[1] as usize];
            let color = if full.alpha {
                egui::ColorImage::from_rgba_unmultiplied(color_size, dst.buffer())
            } else {
                egui::ColorImage::from_rgb(color_size, dst.buffer())
            };
            self.texture = Some(ui.ctx().load_texture("image", color, TextureOptions::LINEAR));
            self.tex_size = size;
            self.resize_ms = ms(t);
        }

        let tex = self.texture.as_ref().unwrap();
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        ui.painter().image(tex.id(), rect, uv, Color32::WHITE);

        if self.first_frame_ms.is_none() {
            self.first_frame_ms = Some(ms(self.t0));
            // The image reaches the screen after this frame; the next frame reports it.
            ui.ctx().request_repaint();
        }
    }
}

/// Largest rect with the image's proportions that fits in `panel`, centred.
fn fit_rect(img_w: f32, img_h: f32, panel: Rect) -> Rect {
    let scale = (panel.width() / img_w).min(panel.height() / img_h);
    Rect::from_center_size(panel.center(), Vec2::new(img_w * scale, img_h * scale))
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h))
    }

    #[test]
    fn fit_rect_cases() {
        // Wide image in a tall panel: full width, centred vertically.
        assert_eq!(fit_rect(400.0, 200.0, panel(100.0, 300.0)), Rect::from_min_max(Pos2::new(0.0, 125.0), Pos2::new(100.0, 175.0)));
        // Tall image in a wide panel: full height, centred horizontally.
        assert_eq!(fit_rect(200.0, 400.0, panel(300.0, 100.0)), Rect::from_min_max(Pos2::new(125.0, 0.0), Pos2::new(175.0, 100.0)));
        // Small image is enlarged.
        assert_eq!(fit_rect(48.0, 36.0, panel(480.0, 360.0)), panel(480.0, 360.0));
        // Exact fit.
        assert_eq!(fit_rect(480.0, 360.0, panel(480.0, 360.0)), panel(480.0, 360.0));
    }
}
