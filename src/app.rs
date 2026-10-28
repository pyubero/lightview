use crate::load::{Frame, Loaded, Loader, Msg};
use crate::view::{self, View};
use eframe::egui::{self, Color32, ColorImage, Key, Modifiers, Pos2, Rect, Sense, TextureHandle, TextureOptions, Vec2, ViewportCommand};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::{Image, ImageRef}};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

/// What the current texture shows: (generation, animation frame, source crop, size in px).
type TexKey = (u64, usize, [f32; 4], [u32; 2]);

pub struct App {
    t0: Instant,
    timings: bool,
    bench: bool,
    rx: mpsc::Receiver<Msg>,
    loader: Loader,
    files: Vec<PathBuf>,
    index: usize,
    /// Decoded images for current ±1 only.
    cache: HashMap<PathBuf, Loaded>,
    preview: Option<Frame>,
    error: Option<String>,
    view: View,
    texture: Option<TextureHandle>,
    tex_key: Option<TexKey>,
    /// Bumped whenever what's shown changes source (navigation, preview → full).
    generation: u64,
    anim_frame: usize,
    anim_due: Option<Instant>,
    title: String,
    // --timings bookkeeping.
    window_reported: bool,
    preview_reported: bool,
    full_reported: bool,
    nav_start: Option<Instant>,
    close_next: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, t0: Instant, timings: bool, bench: bool, rx: mpsc::Receiver<Msg>, loader: Loader, start: Option<PathBuf>) -> Self {
        // Otherwise egui's own Ctrl +/− scales the whole UI.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        App {
            t0, timings, bench, rx, loader,
            files: start.into_iter().collect(), index: 0,
            cache: HashMap::new(), preview: None, error: None, view: View::Fit,
            texture: None, tex_key: None, generation: 0, anim_frame: 0, anim_due: None, title: String::new(),
            window_reported: false, preview_reported: false, full_reported: false, nav_start: None, close_next: false,
        }
    }

    fn current(&self) -> Option<&PathBuf> {
        self.files.get(self.index)
    }

    /// Current, next, previous: the order they should be decoded in.
    fn window(&self) -> Vec<PathBuf> {
        [Some(self.index), self.index.checked_add(1), self.index.checked_sub(1)]
            .into_iter()
            .flatten()
            .filter_map(|i| self.files.get(i).cloned())
            .collect()
    }

    fn request_decodes(&mut self) {
        let window = self.window();
        self.cache.retain(|p, _| window.contains(p));
        self.loader.want(window.into_iter().filter(|p| !self.cache.contains_key(p)).collect());
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Folder(files) => {
                if let Some(i) = self.current().and_then(|cur| files.iter().position(|p| p == cur)) {
                    self.files = files;
                    self.index = i;
                    self.request_decodes();
                }
            }
            Msg::Preview(path, frame) => {
                if self.current() == Some(&path) && !self.cache.contains_key(&path) {
                    self.preview = Some(frame);
                    self.generation += 1;
                }
            }
            Msg::Full(path, loaded) => {
                if self.window().contains(&path) {
                    if self.current() == Some(&path) {
                        self.preview = None;
                        self.generation += 1;
                    }
                    self.cache.insert(path, loaded);
                }
            }
            Msg::Error(path, e) => {
                if self.current() == Some(&path) {
                    self.error = Some(e);
                }
            }
        }
    }

    /// Moves by `delta` in the folder, stopping at the ends.
    fn navigate(&mut self, delta: isize) {
        let Some(index) = self.index.checked_add_signed(delta).filter(|&i| i < self.files.len()) else { return };
        self.index = index;
        self.view = View::Fit;
        self.preview = None;
        self.error = None;
        self.generation += 1;
        self.anim_frame = 0;
        self.anim_due = None;
        self.nav_start = Some(Instant::now());
        self.request_decodes();
    }

    fn make_title(&self) -> String {
        let Some(path) = self.current() else { return "lightview".into() };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        match self.cache.get(path) {
            Some(l) => format!("{name} — {}×{} — {}", l.frames[0].w, l.frames[0].h, format_size(l.file_size)),
            None => name.into_owned(),
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::BLACK.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.close_next {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if self.timings && !self.window_reported {
            eprintln!("window      {:.1} ms", ms(self.t0));
            self.window_reported = true;
        }
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(msg);
        }

        // Navigation and fullscreen.
        let (left, right, f11, esc, fullscreen) = ctx.input(|i| {
            (i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight), i.key_pressed(Key::F11), i.key_pressed(Key::Escape), i.viewport().fullscreen.unwrap_or(false))
        });
        if left {
            self.navigate(-1);
        }
        if right {
            self.navigate(1);
        }
        if f11 {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
        }
        if esc && fullscreen {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }

        let viewport = ui.max_rect();
        let ppp = ctx.pixels_per_point();
        let full = self.current().and_then(|p| self.cache.get(p));

        // Zoom and pan only once the full image is in (the preview has different pixel sizes).
        if let Some(l) = full {
            let img = Vec2::new(l.frames[0].w as f32, l.frames[0].h as f32);
            let drag = ui.interact(viewport, ui.scope_id().with("image"), Sense::drag());
            if drag.dragged() {
                self.view = view::pan(self.view, drag.drag_delta(), img, viewport, ppp);
            }
            let (factor, hover) = ctx.input(|i| (i.zoom_delta() * (i.smooth_scroll_delta.y / 200.0).exp(), i.pointer.hover_pos()));
            if factor != 1.0 {
                self.view = view::zoom_at(self.view, factor, hover.unwrap_or(viewport.center()), img, viewport, ppp);
            }
            let (zoom_in, zoom_out, fit, actual) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::COMMAND, Key::Equals) || i.consume_key(Modifiers::COMMAND, Key::Plus),
                    i.consume_key(Modifiers::COMMAND, Key::Minus),
                    i.consume_key(Modifiers::COMMAND, Key::Num0),
                    i.consume_key(Modifiers::COMMAND, Key::Num1),
                )
            });
            if zoom_in || zoom_out {
                let factor = if zoom_in { 1.25 } else { 0.8 };
                self.view = view::zoom_at(self.view, factor, viewport.center(), img, viewport, ppp);
            }
            if fit {
                self.view = View::Fit;
            }
            if actual {
                self.view = view::actual_size(img);
            }

            // Animation.
            let frames = &l.frames;
            if frames.len() > 1 {
                let now = Instant::now();
                let due = *self.anim_due.get_or_insert(now + frames[self.anim_frame].delay);
                if now >= due {
                    self.anim_frame = (self.anim_frame + 1) % frames.len();
                    self.anim_due = Some(now + frames[self.anim_frame].delay);
                }
                ctx.request_repaint_after(self.anim_due.unwrap() - now);
            }
        }

        let (frame, view) = match full {
            Some(l) => (Some(&l.frames[self.anim_frame]), self.view),
            None => (self.preview.as_ref(), View::Fit),
        };
        if let Some(frame) = frame {
            let (dest, crop) = view::layout(view, Vec2::new(frame.w as f32, frame.h as f32), viewport, ppp);
            let size = [(dest.width() * ppp).round().max(1.0) as u32, (dest.height() * ppp).round().max(1.0) as u32];
            let key = (self.generation, self.anim_frame, [crop.min.x, crop.min.y, crop.width(), crop.height()], size);
            if self.tex_key != Some(key) {
                // shortcut: resize runs on the UI thread every pan/zoom frame; the GPU scaling setting (M5) is the smooth path.
                let image = resize(frame, crop, size);
                match &mut self.texture {
                    Some(t) => t.set(image, TextureOptions::LINEAR),
                    None => self.texture = Some(ctx.load_texture("image", image, TextureOptions::LINEAR)),
                }
                self.tex_key = Some(key);
            }
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().image(self.texture.as_ref().unwrap().id(), dest, uv, Color32::WHITE);

            if self.timings {
                if full.is_none() && !self.preview_reported {
                    eprintln!("preview     {:.1} ms", ms(self.t0));
                    self.preview_reported = true;
                }
                if full.is_some() && !self.full_reported {
                    eprintln!("full image  {:.1} ms", ms(self.t0));
                    self.full_reported = true;
                    self.close_next = self.bench;
                    // The image reaches the screen after this frame; closing waits one more.
                    ctx.request_repaint();
                }
                if let Some(t) = self.nav_start.filter(|_| full.is_some()) {
                    eprintln!("navigation  {:.1} ms", ms(t));
                    self.nav_start = None;
                }
            }
        } else if let Some(e) = &self.error {
            ui.painter().text(viewport.center(), egui::Align2::CENTER_CENTER, format!("Can't open this image:\n{e}"), egui::FontId::proportional(14.0), Color32::GRAY);
        }

        let title = self.make_title();
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// Resizes the `crop` part of `frame` to `size` px.
fn resize(frame: &Frame, crop: Rect, size: [u32; 2]) -> ColorImage {
    let pixel_type = if frame.alpha { PixelType::U8x4 } else { PixelType::U8x3 };
    let src = ImageRef::new(frame.w, frame.h, &frame.pixels, pixel_type).unwrap();
    let mut dst = Image::new(size[0], size[1], pixel_type);
    let opts = ResizeOptions::new()
        .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
        .crop(crop.min.x.into(), crop.min.y.into(), crop.width().into(), crop.height().into());
    Resizer::new().resize(&src, &mut dst, &opts).unwrap();
    let size = [size[0] as usize, size[1] as usize];
    if frame.alpha { ColorImage::from_rgba_unmultiplied(size, dst.buffer()) } else { ColorImage::from_rgb(size, dst.buffer()) }
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000 { format!("{:.1} MB", bytes as f64 / 1e6) } else { format!("{} KB", bytes.div_ceil(1000)) }
}

pub fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
