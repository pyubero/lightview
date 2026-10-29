use crate::exif_edit;
use crate::load::{self, Frame, Loaded, Loader, Msg, Notify};
use crate::save;
use crate::settings::{self, Interpolation, Settings};
use crate::view::{self, Transform, View};
use eframe::egui::{self, Color32, ColorImage, Key, KeyboardShortcut, Modifiers, Pos2, Rect, Sense, TextureFilter, TextureHandle, TextureOptions, Vec2, ViewportCommand};
use fast_image_resize::{PixelType, ResizeOptions, Resizer, images::{Image, ImageRef}};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

const RECENT_MAX: usize = 10;

#[derive(Clone, PartialEq)]
enum Action {
    Open,
    SaveAs,
    OpenRecent(PathBuf),
    Close,
    Info,
    EditExif,
    Settings,
    ZoomIn,
    ZoomOut,
    Fit,
    ActualSize,
    Fullscreen,
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
}

const fn ctrl(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::COMMAND, key)
}

/// Keyboard shortcuts, also shown in the menus. Ctrl+Plus is an alias for Ctrl+=.
const SHORTCUTS: [(KeyboardShortcut, Action); 12] = [
    (ctrl(Key::O), Action::Open),
    (KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::S), Action::SaveAs),
    (ctrl(Key::W), Action::Close),
    (ctrl(Key::I), Action::Info),
    (ctrl(Key::Equals), Action::ZoomIn),
    (ctrl(Key::Plus), Action::ZoomIn),
    (ctrl(Key::Minus), Action::ZoomOut),
    (ctrl(Key::Num0), Action::Fit),
    (ctrl(Key::Num1), Action::ActualSize),
    (KeyboardShortcut::new(Modifiers::NONE, Key::F11), Action::Fullscreen),
    (ctrl(Key::L), Action::RotateLeft),
    (ctrl(Key::R), Action::RotateRight),
];

/// What the current texture holds; it's rebuilt when this changes.
#[derive(Clone, Copy, PartialEq)]
struct TexKey {
    generation: u64,
    anim_frame: usize,
    /// GPU scaling uploads the whole frame, so transform/crop/size don't matter then.
    gpu: bool,
    interpolation: Interpolation,
    transform: Transform,
    crop: [f32; 4],
    size: [u32; 2],
}

pub struct App {
    t0: Instant,
    timings: bool,
    bench: bool,
    rx: mpsc::Receiver<Msg>,
    loader: Loader,
    notify: Notify,
    /// Explicitly opened files, newest first. Saved between runs.
    recent: Vec<PathBuf>,
    settings: Settings,
    show_info: bool,
    show_settings: bool,
    exif_dialog: Option<exif_edit::Dialog>,
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
    /// View-only rotate/flip of the current image; never written to the file.
    transform: Transform,
    title: String,
    // --timings bookkeeping.
    window_reported: bool,
    // The size of the last preview reported, so the EXIF thumbnail and the reduced-size decode both show.
    preview_reported: Option<[u32; 2]>,
    full_reported: bool,
    nav_start: Option<Instant>,
    close_next: bool,
    /// A short message shown at the bottom (e.g. "Saved …"), with when it appeared.
    status: Option<(String, Instant)>,
}

impl App {
    /// `start` is already being decoded (see `load::start` in main).
    #[allow(clippy::too_many_arguments)]
    pub fn new(cc: &eframe::CreationContext, t0: Instant, timings: bool, bench: bool, rx: mpsc::Receiver<Msg>, loader: Loader, notify: Notify, settings: Settings, start: Option<PathBuf>) -> Self {
        // Otherwise egui's own Ctrl +/− scales the whole UI.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let mut recent: Vec<PathBuf> = cc.storage.and_then(|s| s.get_string("recent")).unwrap_or_default().lines().map(PathBuf::from).collect();
        if let Some(path) = &start {
            push_recent(&mut recent, path);
        }
        App {
            t0, timings, bench, rx, loader, notify, recent, settings, show_info: false, show_settings: false, exif_dialog: None,
            files: start.into_iter().collect(), index: 0,
            cache: HashMap::new(), preview: None, error: None, view: View::Fit,
            texture: None, tex_key: None, generation: 0, anim_frame: 0, anim_due: None, transform: Transform::default(), title: String::new(),
            window_reported: false, preview_reported: None, full_reported: false, nav_start: None, close_next: false, status: None,
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
            Msg::Open(path) => self.open(path),
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
            Msg::Saved(path, result) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                self.status = Some((match &result {
                    Ok(()) => format!("Saved {name}"),
                    Err(e) => format!("Couldn't save {name}: {e}"),
                }, Instant::now()));
                let (Ok(()), Ok(path), Some(current)) = (result, std::fs::canonicalize(&path), self.current().cloned()) else { return };
                if path == current {
                    // Overwrote the image on screen: show the saved version.
                    self.cache.remove(&path);
                    self.reset_shown();
                    self.request_decodes();
                }
                if path.parent() == current.parent() {
                    load::relist(&current, self.settings.sort, &self.notify);
                }
            }
        }
    }

    /// Opens `path` as if passed on the command line.
    fn open(&mut self, path: PathBuf) {
        self.cache.clear();
        self.reset_shown();
        match std::fs::canonicalize(&path) {
            Ok(path) => {
                push_recent(&mut self.recent, &path);
                load::start(&path, self.settings.sort, &self.loader, &self.notify);
                self.files = vec![path];
            }
            Err(e) => {
                self.files.clear();
                self.error = Some(format!("{}: {e}", path.display()));
            }
        }
        self.index = 0;
    }

    /// Moves by `delta` in the folder, stopping at the ends.
    fn navigate(&mut self, delta: isize) {
        let Some(index) = self.index.checked_add_signed(delta).filter(|&i| i < self.files.len()) else { return };
        self.index = index;
        self.reset_shown();
        self.nav_start = Some(Instant::now());
        self.request_decodes();
    }

    /// Clears what's shown, before switching to another image.
    fn reset_shown(&mut self) {
        self.view = View::Fit;
        self.transform = Transform::default();
        self.preview = None;
        self.error = None;
        self.generation += 1;
        self.anim_frame = 0;
        self.anim_due = None;
    }

    fn open_dialog(&self) {
        let mut dialog = rfd::FileDialog::new().add_filter("Images", &load::EXTENSIONS);
        if let Some(dir) = self.current().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        // On its own thread so the window keeps repainting while the dialog is open.
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            if let Some(path) = dialog.pick_file() {
                notify.send(Msg::Open(path));
            }
        });
    }

    /// Asks where to save the current image (with its rotate/flip), then saves on that thread.
    fn save_dialog(&self) {
        let Some((path, l)) = self.current().and_then(|p| Some((p, self.cache.get(p)?))) else { return };
        if l.frames.len() != 1 {
            return;
        }
        let (frame, transform, source) = (l.frames[0].clone(), self.transform, path.clone());
        let mut dialog = rfd::FileDialog::new()
            .set_file_name(path.file_name().unwrap_or_default().to_string_lossy())
            .add_filter("JPEG", &["jpg", "jpeg"])
            .add_filter("PNG", &["png"])
            .add_filter("WebP", &["webp"])
            .add_filter("BMP", &["bmp"]);
        if let Some(dir) = path.parent() {
            dialog = dialog.set_directory(dir);
        }
        let notify = self.notify.clone();
        // The system dialog asks before replacing an existing file.
        std::thread::spawn(move || {
            if let Some(dest) = dialog.save_file() {
                let result = save::save(&frame, transform, &source, &dest);
                notify.send(Msg::Saved(dest, result));
            }
        });
    }

    fn menu_bar(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let loaded = self.current().and_then(|p| self.cache.get(p));
        let has_image = loaded.is_some();
        // shortcut: animated images can't be saved; add GIF re-encoding if it's wanted.
        let can_save = loaded.is_some_and(|l| l.frames.len() == 1);
        fn item(ui: &mut egui::Ui, label: &str, action: Action, enabled: bool, actions: &mut Vec<Action>) {
            let mut button = egui::Button::new(label);
            if let Some((sc, _)) = SHORTCUTS.iter().find(|(_, a)| *a == action) {
                button = button.shortcut_text(ui.ctx().format_shortcut(sc));
            }
            if ui.add_enabled(enabled, button).clicked() {
                actions.push(action);
            }
        }
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                item(ui, "Open…", Action::Open, true, actions);
                ui.menu_button("Open Recent", |ui| {
                    if self.recent.is_empty() {
                        ui.add_enabled(false, egui::Button::new("No recent files"));
                    }
                    for path in &self.recent {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
                            actions.push(Action::OpenRecent(path.clone()));
                        }
                    }
                });
                item(ui, "Save As…", Action::SaveAs, can_save, actions);
                item(ui, "Close", Action::Close, true, actions);
                item(ui, "Info", Action::Info, has_image, actions);
            });
            ui.menu_button("Edit", |ui| {
                item(ui, "Rotate Left", Action::RotateLeft, has_image, actions);
                item(ui, "Rotate Right", Action::RotateRight, has_image, actions);
                item(ui, "Flip Horizontally", Action::FlipHorizontal, has_image, actions);
                item(ui, "Flip Vertically", Action::FlipVertical, has_image, actions);
            });
            ui.menu_button("View", |ui| {
                item(ui, "Zoom In", Action::ZoomIn, has_image, actions);
                item(ui, "Zoom Out", Action::ZoomOut, has_image, actions);
                item(ui, "Fit to Window", Action::Fit, has_image, actions);
                item(ui, "Actual Size", Action::ActualSize, has_image, actions);
                ui.separator();
                item(ui, "Fullscreen", Action::Fullscreen, true, actions);
            });
            ui.menu_button("Tools", |ui| {
                let jpeg = self.current().and_then(|p| p.extension()).is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
                item(ui, "Edit EXIF…", Action::EditExif, has_image && jpeg, actions);
            });
            if ui.button("Settings").clicked() {
                actions.push(Action::Settings);
            }
        });
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let sort = self.settings.sort;
        let modal = egui::Modal::new(egui::Id::unique("settings")).show(ctx, |ui| {
            ui.heading("Settings");
            settings::ui(ui, &mut self.settings);
            ui.add_space(8.0);
            ui.button("Close").clicked()
        });
        if modal.inner || modal.should_close() {
            self.show_settings = false;
        }
        if self.settings.sort != sort && let Some(path) = self.current() {
            load::relist(path, self.settings.sort, &self.notify);
        }
    }

    fn exif_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.exif_dialog else { return };
        let modal = egui::Modal::new(egui::Id::unique("exif")).show(ctx, |ui| exif_edit::ui(ui, dialog));
        match modal.inner {
            Some(exif_edit::Choice::Save) => {
                let Some(d) = self.exif_dialog.take() else { return };
                let stem = d.path.file_stem().unwrap_or_default().to_string_lossy();
                let mut file_dialog = rfd::FileDialog::new().set_file_name(format!("{stem}-edited.jpg")).add_filter("JPEG", &["jpg", "jpeg"]);
                if let Some(dir) = d.path.parent() {
                    file_dialog = file_dialog.set_directory(dir);
                }
                let notify = self.notify.clone();
                std::thread::spawn(move || {
                    if let Some(dest) = file_dialog.save_file() {
                        let result = exif_edit::write_copy(&d.path, &dest, &d.original, &d.values, d.remove_gps);
                        notify.send(Msg::Saved(dest, result));
                    }
                });
            }
            Some(exif_edit::Choice::Cancel) => self.exif_dialog = None,
            None if modal.should_close() => self.exif_dialog = None,
            None => {}
        }
    }

    fn info_dialog(&mut self, ctx: &egui::Context) {
        let (Some(path), true) = (self.current(), self.show_info) else { return };
        let Some(l) = self.cache.get(path) else { return };
        let modal = egui::Modal::new(egui::Id::unique("info")).show(ctx, |ui| {
            ui.heading("Info");
            egui::Grid::new("info-grid").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                let ext = path.extension().unwrap_or_default().to_string_lossy().to_uppercase();
                let rows = [
                    ("Name", path.file_name().unwrap_or_default().to_string_lossy().into_owned()),
                    ("Folder", path.parent().map(|p| p.display().to_string()).unwrap_or_default()),
                    ("Dimensions", format!("{} × {} px", l.frames[0].w, l.frames[0].h)),
                    ("File size", format_size(l.file_size)),
                    ("Type", if l.frames.len() > 1 { format!("{ext}, {} frames", l.frames.len()) } else { ext }),
                ];
                for (k, v) in rows {
                    ui.label(k);
                    ui.label(v);
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.button("Close").clicked()
        });
        if modal.inner || modal.should_close() {
            self.show_info = false;
        }
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
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let recent: Vec<String> = self.recent.iter().map(|p| p.display().to_string()).collect();
        storage.set_string("recent", recent.join("\n"));
        eframe::set_value(storage, settings::KEY, &self.settings);
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let [r, g, b] = self.settings.background;
        Color32::from_rgb(r, g, b).to_normalized_gamma_f32()
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

        let mut actions: Vec<Action> = ctx.input_mut(|i| SHORTCUTS.iter().filter(|(sc, _)| i.consume_shortcut(sc)).map(|(_, a)| a.clone()).collect());
        let (left, right, esc, fullscreen) = ctx.input(|i| {
            (i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight), i.key_pressed(Key::Escape), i.viewport().fullscreen.unwrap_or(false))
        });
        if !fullscreen {
            egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui, &mut actions));
        }
        for action in &actions {
            match action {
                Action::Open => self.open_dialog(),
                Action::SaveAs => self.save_dialog(),
                Action::OpenRecent(path) => self.open(path.clone()),
                Action::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
                Action::Info => self.show_info = true,
                Action::Settings => self.show_settings = true,
                Action::EditExif => match self.current().map(|p| exif_edit::read(p)) {
                    Some(Ok(dialog)) => self.exif_dialog = Some(dialog),
                    Some(Err(e)) => self.status = Some((format!("Can't edit EXIF: {e}"), Instant::now())),
                    None => {}
                },
                Action::RotateLeft | Action::RotateRight | Action::FlipHorizontal | Action::FlipVertical => {
                    self.transform = match action {
                        Action::RotateLeft => self.transform.rotate_left(),
                        Action::RotateRight => self.transform.rotate_right(),
                        Action::FlipHorizontal => self.transform.flip_horizontal(),
                        _ => self.transform.flip_vertical(),
                    };
                    self.view = View::Fit;
                }
                Action::Fullscreen => ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen)),
                _ => {} // zoom actions are applied below, once the image size is known
            }
        }
        if left {
            self.navigate(-1);
        }
        if right {
            self.navigate(1);
        }
        if esc && fullscreen {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }

        let viewport = ui.available_rect_before_wrap();
        let ppp = ctx.pixels_per_point();
        self.loader.set_target([(viewport.width() * ppp) as u32, (viewport.height() * ppp) as u32]);
        let full = self.current().and_then(|p| self.cache.get(p));

        // Zoom and pan only once the full image is in (the preview has different pixel sizes).
        if let Some(l) = full {
            let img = self.transform.size(Vec2::new(l.frames[0].w as f32, l.frames[0].h as f32));
            let drag = ui.interact(viewport, ui.scope_id().with("image"), Sense::drag());
            if drag.dragged() {
                self.view = view::pan(self.view, drag.drag_delta(), img, viewport, ppp);
            }
            let (factor, hover) = ctx.input(|i| (i.zoom_delta() * (i.smooth_scroll_delta.y / 200.0).exp(), i.pointer.hover_pos()));
            if factor != 1.0 {
                self.view = view::zoom_at(self.view, factor, hover.unwrap_or(viewport.center()), img, viewport, ppp);
            }
            for action in &actions {
                self.view = match action {
                    Action::ZoomIn => view::zoom_at(self.view, 1.25, viewport.center(), img, viewport, ppp),
                    Action::ZoomOut => view::zoom_at(self.view, 0.8, viewport.center(), img, viewport, ppp),
                    Action::Fit => View::Fit,
                    Action::ActualSize => view::actual_size(img),
                    _ => self.view,
                };
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
            let frame_size = Vec2::new(frame.w as f32, frame.h as f32);
            let (dest, crop) = view::layout(view, self.transform.size(frame_size), viewport, ppp);
            let interpolation = self.settings.interpolation();
            // Images bigger than the GPU's texture limit always take the CPU path.
            let gpu = self.settings.gpu_scaling && frame.w.max(frame.h) as usize <= ctx.input(|i| i.max_texture_side);
            let key = if gpu {
                TexKey { generation: self.generation, anim_frame: self.anim_frame, gpu, interpolation, transform: Transform::default(), crop: [0.0; 4], size: [frame.w, frame.h] }
            } else {
                let src = self.transform.to_source(crop, frame_size);
                let size = [(dest.width() * ppp).round().max(1.0) as u32, (dest.height() * ppp).round().max(1.0) as u32];
                TexKey { generation: self.generation, anim_frame: self.anim_frame, gpu, interpolation, transform: self.transform, crop: [src.min.x, src.min.y, src.width(), src.height()], size }
            };
            if self.tex_key != Some(key) {
                let (image, options) = if gpu {
                    let options = match interpolation {
                        Interpolation::Nearest => TextureOptions::NEAREST,
                        // Mipmaps keep big images from aliasing when shrunk.
                        _ => TextureOptions { mipmap_mode: Some(TextureFilter::Linear), ..TextureOptions::LINEAR },
                    };
                    (to_color_image(frame.alpha, [frame.w as usize, frame.h as usize], &frame.pixels), options)
                } else {
                    // shortcut: resize runs on the UI thread every pan/zoom frame; GPU scaling is the smooth path.
                    let [x, y, w, h] = key.crop;
                    (resize(frame, Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)), key.size, self.transform, interpolation), TextureOptions::LINEAR)
                };
                match &mut self.texture {
                    Some(t) => t.set(image, options),
                    None => self.texture = Some(ctx.load_texture("image", image, options)),
                }
                self.tex_key = Some(key);
            }
            let texture = self.texture.as_ref().unwrap().id();
            if gpu {
                // Each screen corner samples the source point the transform maps it from.
                let mut mesh = egui::Mesh::with_texture(texture);
                let shown = [crop.left_top(), crop.right_top(), crop.right_bottom(), crop.left_bottom()];
                let screen = [dest.left_top(), dest.right_top(), dest.right_bottom(), dest.left_bottom()];
                for (pos, p) in screen.into_iter().zip(shown) {
                    let uv = (self.transform.source_point(p, frame_size).to_vec2() / frame_size).to_pos2();
                    mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
                }
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                ui.painter().add(mesh);
            } else {
                ui.painter().image(texture, dest, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }

            if self.timings {
                if full.is_none() && self.preview_reported != Some([frame.w, frame.h]) {
                    eprintln!("preview     {:.1} ms  ({}×{})", ms(self.t0), frame.w, frame.h);
                    self.preview_reported = Some([frame.w, frame.h]);
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

        if let Some((text, at)) = &self.status {
            const SHOWN: std::time::Duration = std::time::Duration::from_secs(4);
            if at.elapsed() < SHOWN {
                let painter = ui.painter();
                let galley = painter.layout_no_wrap(text.clone(), egui::FontId::proportional(14.0), Color32::WHITE);
                let rect = Rect::from_center_size(Pos2::new(viewport.center().x, viewport.bottom() - 24.0), galley.size() + Vec2::new(16.0, 8.0));
                painter.rect_filled(rect, 4.0, Color32::from_black_alpha(190));
                painter.galley(rect.min + Vec2::new(8.0, 4.0), galley, Color32::WHITE);
                ctx.request_repaint_after(SHOWN - at.elapsed());
            } else {
                self.status = None;
            }
        }

        self.info_dialog(&ctx);
        self.settings_dialog(&ctx);
        self.exif_dialog(&ctx);

        let title = self.make_title();
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// Resizes the `crop` part of `frame` so that, after `transform`, it's `size` px.
/// Transforming the small resized output instead of the full image keeps rotating cheap.
fn resize(frame: &Frame, crop: Rect, size: [u32; 2], transform: Transform, interpolation: Interpolation) -> ColorImage {
    let pixel_type = if frame.alpha { PixelType::U8x4 } else { PixelType::U8x3 };
    let src = ImageRef::new(frame.w, frame.h, &frame.pixels, pixel_type).unwrap();
    let [w, h] = if transform.rot % 2 == 1 { [size[1], size[0]] } else { size };
    let mut dst = Image::new(w, h, pixel_type);
    let opts = ResizeOptions::new()
        .resize_alg(interpolation.cpu())
        .crop(crop.min.x.into(), crop.min.y.into(), crop.width().into(), crop.height().into());
    Resizer::new().resize(&src, &mut dst, &opts).unwrap();
    let size = [size[0] as usize, size[1] as usize];
    let pixels = if transform == Transform::default() {
        dst.into_vec()
    } else {
        let img = if frame.alpha {
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, dst.into_vec()).unwrap())
        } else {
            image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, dst.into_vec()).unwrap())
        };
        transform.apply(img).into_bytes()
    };
    to_color_image(frame.alpha, size, &pixels)
}

fn to_color_image(alpha: bool, size: [usize; 2], pixels: &[u8]) -> ColorImage {
    if alpha { ColorImage::from_rgba_unmultiplied(size, pixels) } else { ColorImage::from_rgb(size, pixels) }
}

/// Moves `path` to the front of `recent`, keeping at most `RECENT_MAX`.
fn push_recent(recent: &mut Vec<PathBuf>, path: &std::path::Path) {
    recent.retain(|p| p != path);
    recent.insert(0, path.to_path_buf());
    recent.truncate(RECENT_MAX);
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000 { format!("{:.1} MB", bytes as f64 / 1e6) } else { format!("{} KB", bytes.div_ceil(1000)) }
}

pub fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_with_transform() {
        // 3×2 RGB frame, red channel = pixel index.
        let pixels: Vec<u8> = (0..6u8).flat_map(|i| [i, 0, 0]).collect();
        let frame = Frame { w: 3, h: 2, pixels, alpha: false, delay: std::time::Duration::ZERO };
        let t = Transform::default().rotate_right();
        let crop = t.to_source(Rect::from_min_size(Pos2::ZERO, Vec2::new(2.0, 3.0)), Vec2::new(3.0, 2.0));
        let out = resize(&frame, crop, [2, 3], t, Interpolation::Bilinear);
        assert_eq!(out.size, [2, 3]);
        // Rotated right, the top row reads bottom-left then top-left of the source: 3, 0.
        let red: Vec<u8> = out.pixels.iter().map(|p| p.r()).collect();
        assert_eq!(red, [3, 0, 4, 1, 5, 2]);
    }

    #[test]
    fn recent_list() {
        let mut recent = Vec::new();
        for p in ["/a", "/b", "/a"] {
            push_recent(&mut recent, std::path::Path::new(p));
        }
        assert_eq!(recent, [PathBuf::from("/a"), PathBuf::from("/b")]);
        for i in 0..20 {
            push_recent(&mut recent, std::path::Path::new(&format!("/{i}")));
        }
        assert_eq!(recent.len(), RECENT_MAX);
        assert_eq!(recent[0], PathBuf::from("/19"));
    }
}
