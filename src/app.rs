use crate::edit::{self, Adjust, Doc, ResizeDialog};
use crate::exif_edit;
use crate::inspector::{self, Inspector};
use crate::load::{self, Frame, Loaded, Loader, Msg, Notify};
use crate::markup::{self, Element, Pick, Selection, Shape, Style};
use crate::save;
use crate::settings::{self, Interpolation, Settings};
use crate::view::{self, Transform, View};
use eframe::egui::{self, Color32, ColorImage, CursorIcon, Key, KeyboardShortcut, Modifiers, Pos2, Rect, Sense, TextureFilter, TextureHandle, TextureOptions, Vec2, ViewportCommand};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
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
    PrevImage,
    NextImage,
    Play,
    Pause,
    NextFrame,
    PrevFrame,
    Undo,
    Redo,
    Crop,
    Resize,
    /// Save As in one format: an index into `save::FORMATS`.
    Export(usize),
    AdjustColor,
    AutoLevels,
    ResetAdjust,
    Measure,
    ShowToolbar,
    Pick(Pick),
    /// Border/Fill Color…: show the toolbar's style controls.
    StyleControls,
    Describe,
    SelectTool(SelKind),
    SelectAll,
    Deselect,
    Copy,
    Cut,
    Paste,
    DeleteSelection,
    ImageFromSelection,
    /// Remove Background, and crop to the subject when true.
    Cutout(bool),
    /// Denoise or Enhance Image: shows a before/after preview first.
    Restore(crate::ai::Task),
    DownloadRuntime,
    AiModels,
}

#[derive(Clone, Copy, PartialEq)]
enum SelKind {
    Rect,
    Ellipse,
    Lasso,
}

/// What dragging on the image does.
#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Pan,
    Crop,
    Measure,
    Select,
    Draw(Shape),
    Selection(SelKind),
}

/// What dragging with the Select tool does.
#[derive(Clone, Copy, PartialEq)]
enum Grab {
    None,
    Move,
    Resize,
}

/// A Denoise / Enhance run in progress.
struct Restoring {
    task: crate::ai::Task,
    progress: f32,
    started: Instant,
    cancel: Arc<AtomicBool>,
    /// Applying to a selection: its outline, to mask the result with.
    selection: Option<Selection>,
}

/// The before/after preview of Denoise / Enhance on part of the image.
struct Compare {
    task: crate::ai::Task,
    /// Output px.
    region: Rect,
    texture: TextureHandle,
    /// Where the before/after line is, 0–1 across the region.
    split: f32,
    ms_per_mp: f64,
}

/// A download waiting for the user to accept its licence.
#[derive(Clone, Copy)]
enum Download {
    /// An index into `ai::CATALOG`.
    Model(usize),
    Runtime,
    /// ONNX Runtime's GPU build and NVIDIA's CUDA libraries.
    Gpu,
}

/// What to do once the user agrees to discard unsaved edits.
enum Pending {
    Navigate(isize),
    Open(PathBuf),
    Close,
}

const fn ctrl(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::COMMAND, key)
}

const fn key(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::NONE, key)
}

/// Keyboard shortcuts, also shown in the menus. Ctrl+Plus is an alias for Ctrl+=.
const SHORTCUTS: [(KeyboardShortcut, Action); 18] = [
    (ctrl(Key::O), Action::Open),
    (KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::S), Action::SaveAs),
    (ctrl(Key::W), Action::Close),
    (ctrl(Key::I), Action::Info),
    (ctrl(Key::Equals), Action::ZoomIn),
    (ctrl(Key::Plus), Action::ZoomIn),
    (ctrl(Key::Minus), Action::ZoomOut),
    (ctrl(Key::Num0), Action::Fit),
    (ctrl(Key::Num1), Action::ActualSize),
    (key(Key::F11), Action::Fullscreen),
    (key(Key::ArrowLeft), Action::PrevImage),
    (key(Key::ArrowRight), Action::NextImage),
    (ctrl(Key::L), Action::RotateLeft),
    (ctrl(Key::R), Action::RotateRight),
    // Before Ctrl+Z, which would also match Ctrl+Shift+Z.
    (KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::Z), Action::Redo),
    (ctrl(Key::Z), Action::Undo),
    (KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::A), Action::Deselect),
    (ctrl(Key::A), Action::SelectAll),
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
    adjust: Adjust,
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
    inspector: Option<Inspector>,
    files: Vec<PathBuf>,
    index: usize,
    /// Decoded images for current ±1 only.
    // Arc so a frame can hold the current image while calling `&mut self` methods.
    cache: HashMap<PathBuf, Arc<Loaded>>,
    preview: Option<Frame>,
    error: Option<String>,
    view: View,
    texture: Option<TextureHandle>,
    tex_key: Option<TexKey>,
    /// The CPU path's resized pixels before colour adjustments, and what they hold.
    resized: Option<(TexKey, Vec<u8>)>,
    /// Bumped whenever what's shown changes source (navigation, preview → full).
    generation: u64,
    anim_frame: usize,
    anim_due: Option<Instant>,
    anim_paused: bool,
    /// Edits to the current image (None: none yet), with undo/redo history.
    doc: Option<Doc>,
    undo: Vec<Doc>,
    redo: Vec<Doc>,
    /// The edits last saved to a copy, and the ones being saved now.
    saved: Option<Doc>,
    saving: Option<Doc>,
    tool: Tool,
    /// Crop tool: where the drag started and the rect so far, in image px.
    drag_start: Option<Pos2>,
    crop_rect: Option<Rect>,
    /// Measure tool: the two endpoints (image px) and which one is being dragged.
    measure: Option<[Pos2; 2]>,
    measure_drag: usize,
    show_toolbar: bool,
    /// The style for new markup (None until the toolbar is first used on this image).
    style: Option<Style>,
    selected: Option<usize>,
    grab: Grab,
    /// The text element being typed into.
    editing_text: Option<usize>,
    focus_text: bool,
    /// The edits before the markup change in progress (a drag or typing), for one undo step.
    change_base: Option<Doc>,
    /// The element whose style the last undo step changed, so style tweaks merge into one step.
    last_restyle: Option<usize>,
    /// The selection, in output px.
    selection: Option<Selection>,
    /// Textures for Erase and Image elements, by `Element::texture_key`.
    element_textures: HashMap<u64, TextureHandle>,
    resize_dialog: Option<ResizeDialog>,
    /// While the Adjust Color panel is open: the edits from before it opened.
    adjusting: Option<Doc>,
    pending: Option<Pending>,
    show_ai_models: bool,
    /// The download whose licences are being shown, and which one (they come one at a time).
    license_prompt: Option<(Download, usize)>,
    restoring: Option<Restoring>,
    compare: Option<Compare>,
    /// Enhance Image waiting for the user to read the "may invent detail" notice.
    enhance_notice: bool,
    /// The part of the image on screen (output px), for the Denoise / Enhance preview.
    visible: Option<Rect>,
    /// Model files being downloaded.
    downloading: HashSet<String>,
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
        let show_toolbar = cc.storage.and_then(|s| s.get_string("toolbar")).is_some_and(|v| v == "shown");
        if let Some(path) = &start {
            push_recent(&mut recent, path);
        }
        App {
            t0, timings, bench, rx, loader, notify, recent, settings, show_info: false, show_settings: false, inspector: None,
            files: start.into_iter().collect(), index: 0,
            cache: HashMap::new(), preview: None, error: None, view: View::Fit,
            texture: None, tex_key: None, resized: None, generation: 0, anim_frame: 0, anim_due: None, anim_paused: false,
            doc: None, undo: Vec::new(), redo: Vec::new(), saved: None, saving: None, tool: Tool::Pan, drag_start: None, crop_rect: None, measure: None, measure_drag: 1,
            show_toolbar, style: None, selected: None, grab: Grab::None, editing_text: None, focus_text: false, change_base: None, last_restyle: None, selection: None, element_textures: HashMap::new(),
            resize_dialog: None, adjusting: None, pending: None, show_ai_models: false, license_prompt: None, restoring: None, compare: None, enhance_notice: false, visible: None, downloading: HashSet::new(),
            title: String::new(),
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
            Msg::Status(text) => self.status = Some((text, Instant::now())),
            Msg::Progress(p) => {
                if let Some(r) = &mut self.restoring {
                    r.progress = p;
                }
            }
            Msg::Restored(path, region, whole, result) => {
                let Some(run) = self.restoring.take() else { return };
                match result {
                    Ok(_) if self.current() != Some(&path) => {}
                    Ok((img, ms_per_mp)) if !whole => {
                        let image = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                        let texture = self.notify.ctx.get().map(|ctx| ctx.load_texture("restored", image, TextureOptions::LINEAR));
                        self.compare = texture.map(|texture| Compare { task: run.task, region, texture, split: 0.5, ms_per_mp });
                    }
                    Ok((mut img, _)) => {
                        if let Some(sel) = &run.selection
                            && let Some(mask) = markup::polygon_mask(&sel.outline(), None, region.min, 1.0, img.width(), img.height())
                        {
                            for (px, m) in img.pixels_mut().zip(mask.data()) {
                                px[3] = (px[3] as u32 * *m as u32 / 255) as u8;
                            }
                        }
                        let style = Style::for_image(region.size());
                        self.edit(|d| d.elements.push(Element::pasted(Arc::new(img), region.min, region.size(), style)));
                        self.status = Some((format!("{} done", run.task.title()), Instant::now()));
                    }
                    Err(e) if e == "cancelled" => self.status = Some(("Cancelled".into(), Instant::now())),
                    Err(e) => self.status = Some((format!("{} failed: {e}", run.task.title()), Instant::now())),
                }
            }
            Msg::Downloaded(file, result) => {
                self.downloading.remove(&file);
                self.status = Some((match result {
                    Ok(()) if file == "gpu" => "GPU support installed: restart lightview to use it".into(),
                    Ok(()) => format!("Downloaded {file}"),
                    Err(e) => format!("Couldn't download {file}: {e}"),
                }, Instant::now()));
            }
            Msg::Detected(file, result) => match result {
                Ok(size) => {
                    let base = crate::ai::preset(&file, &self.settings.ai.overrides).unwrap_or(crate::ai::Preprocess::auto(size));
                    self.settings.ai.overrides.insert(file, crate::ai::Preprocess { size, ..base });
                }
                Err(e) => self.status = Some((format!("Couldn't read {file}: {e}"), Instant::now())),
            },
            Msg::Cutout(path, result) => match result {
                Ok((img, object)) if self.current() == Some(&path) => {
                    let Some(doc) = self.doc() else { return };
                    let bounds = Rect::from_min_size(Pos2::ZERO, doc.size_vec());
                    let style = *self.style.get_or_insert_with(|| Style::for_image(doc.size_vec()));
                    // Erase everything, then put the cut-out back on top: one undo step.
                    self.edit(|d| {
                        d.elements.push(Element::erase(&Selection::Rect(bounds), false, bounds, Color32::TRANSPARENT));
                        d.elements.push(Element::pasted(Arc::new(img), Pos2::ZERO, bounds.size(), style));
                        if let Some(r) = object {
                            d.crop_to(r);
                        }
                    });
                    self.status = Some(("Done".into(), Instant::now()));
                }
                Ok(_) => {}
                Err(e) => self.status = Some((format!("Couldn't process the image: {e}"), Instant::now())),
            },
            Msg::Open(path) => self.confirm(Pending::Open(path)),
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
                    self.cache.insert(path, Arc::new(loaded));
                }
            }
            Msg::Error(path, e) => {
                if self.current() == Some(&path) {
                    self.error = Some(e);
                }
            }
            Msg::Saved(path, result) => {
                let saving = self.saving.take();
                if result.is_ok() && saving.is_some() {
                    self.saved = saving;
                }
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

    /// The current image with no edits, once it's decoded.
    fn base_doc(&self) -> Option<Doc> {
        let l = self.current().and_then(|p| self.cache.get(p))?;
        Some(Doc::new(l.frames[0].w, l.frames[0].h))
    }

    /// The current image's edits (none yet is the identity), once it's decoded.
    fn doc(&self) -> Option<Doc> {
        self.doc.clone().or_else(|| self.base_doc())
    }

    /// Whether the current edits haven't been saved to a copy.
    fn dirty(&self) -> bool {
        self.doc.is_some() && self.doc != self.saved.clone().or_else(|| self.base_doc())
    }

    /// Applies an edit to the current image as one undo step.
    fn edit(&mut self, f: impl FnOnce(&mut Doc)) {
        let Some(mut doc) = self.doc() else { return };
        let before = doc.clone();
        f(&mut doc);
        if doc != before {
            if doc.geometry() != before.geometry() {
                // The measurement's and selection's points are in the old image coordinates.
                (self.view, self.measure, self.selection) = (View::Fit, None, None);
            }
            self.undo.push(before);
            self.redo.clear();
            self.doc = Some(doc);
            self.last_restyle = None;
        }
    }

    /// The current edits, to change in place; pair with `commit`.
    fn doc_mut(&mut self) -> Option<&mut Doc> {
        if self.doc.is_none() {
            self.doc = self.base_doc();
        }
        self.doc.as_mut()
    }

    /// Ends an in-place change that started from `before`, as one undo step.
    fn commit(&mut self, before: Doc) {
        if self.doc.as_ref() != Some(&before) {
            self.undo.push(before);
            self.redo.clear();
        }
        self.last_restyle = None;
    }

    /// Changes element `i`'s style; consecutive changes to one element are one undo step.
    fn restyle(&mut self, i: usize, style: Style) {
        if self.last_restyle == Some(i) {
            if let Some(e) = self.doc_mut().and_then(|d| d.elements.get_mut(i)) {
                e.style = style;
            }
        } else {
            self.edit(|d| d.elements[i].style = style);
            self.last_restyle = Some(i);
        }
    }

    fn pick(&self) -> Option<Pick> {
        match self.tool {
            Tool::Select => Some(Pick::Select),
            Tool::Draw(s) => Some(Pick::Draw(s)),
            Tool::Measure => Some(Pick::Measure),
            _ => None,
        }
    }

    fn set_pick(&mut self, pick: Pick) {
        self.finish_text();
        (self.crop_rect, self.selected) = (None, None);
        self.tool = match pick {
            Pick::Select => Tool::Select,
            Pick::Draw(s) => Tool::Draw(s),
            Pick::Measure => Tool::Measure,
        };
        self.show_toolbar = true;
    }

    /// Stops typing into a text element; an empty one is removed.
    fn finish_text(&mut self) {
        let Some(i) = self.editing_text.take() else { return };
        if let Some(d) = self.doc_mut()
            && d.elements.get(i).is_some_and(Element::is_empty)
        {
            d.elements.remove(i);
        }
        if let Some(before) = self.change_base.take() {
            self.commit(before);
        }
    }

    fn undo_redo(&mut self, undo: bool) {
        let Some(current) = self.doc() else { return };
        let (from, to) = if undo { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some(doc) = from.pop() else { return };
        (self.selected, self.editing_text, self.change_base, self.last_restyle) = (None, None, None, None);
        to.push(current);
        if self.doc.as_ref().is_none_or(|d| d.geometry() != doc.geometry()) {
            (self.view, self.measure, self.selection) = (View::Fit, None, None);
        }
        self.doc = Some(doc);
    }

    /// Runs `p` now, or asks first when that would lose unsaved edits.
    fn confirm(&mut self, p: Pending) {
        if self.dirty() {
            self.pending = Some(p);
            return;
        }
        match p {
            Pending::Navigate(delta) => self.navigate(delta),
            Pending::Open(path) => self.open(path),
            Pending::Close => self.close_next = true,
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
        self.preview = None;
        self.error = None;
        self.generation += 1;
        self.anim_frame = 0;
        self.anim_due = None;
        self.anim_paused = false;
        (self.doc, self.saved, self.saving) = (None, None, None);
        self.undo.clear();
        self.redo.clear();
        self.tool = Tool::Pan;
        self.crop_rect = None;
        self.measure = None;
        self.adjusting = None;
        (self.style, self.selected, self.grab, self.editing_text, self.change_base, self.last_restyle) = (None, None, Grab::None, None, None, None);
        self.selection = None;
        self.element_textures.clear();
        if let Some(r) = self.restoring.take() {
            r.cancel.store(true, Ordering::Relaxed);
        }
        self.compare = None;
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

    /// Asks where to save the current image (with its edits), then saves on that thread.
    /// `format` (an index into `save::FORMATS`) limits the dialog to one format.
    fn save_dialog(&mut self, format: Option<usize>) {
        let Some((path, l)) = self.current().and_then(|p| Some((p, self.cache.get(p)?))) else { return };
        let Some(doc) = self.doc().filter(|_| l.frames.len() == 1) else { return };
        let (frame, source) = (l.frames[0].clone(), path.clone());
        let mut dialog = rfd::FileDialog::new();
        dialog = match format {
            Some(i) => {
                let (name, exts) = save::FORMATS[i];
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                dialog.set_file_name(format!("{stem}.{}", exts[0])).add_filter(name, exts)
            }
            None => save::FORMATS.iter().fold(dialog.set_file_name(path.file_name().unwrap_or_default().to_string_lossy()), |d, (name, exts)| d.add_filter(*name, exts)),
        };
        if let Some(dir) = path.parent() {
            dialog = dialog.set_directory(dir);
        }
        let notify = self.notify.clone();
        self.saving = Some(doc.clone());
        // The system dialog asks before replacing an existing file.
        std::thread::spawn(move || {
            if let Some(dest) = dialog.save_file() {
                let result = save::save(&frame, &doc, &source, &dest);
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
                item(ui, "Close", Action::Close, true, actions);
                ui.separator();
                item(ui, "Save As…", Action::SaveAs, can_save, actions);
                ui.add_enabled_ui(can_save, |ui| {
                    ui.menu_button("Export", |ui| {
                        for (i, (name, _)) in save::FORMATS.iter().enumerate() {
                            item(ui, &format!("{name}…"), Action::Export(i), true, actions);
                        }
                    });
                });
            });
            ui.menu_button("Edit", |ui| {
                item(ui, "Undo", Action::Undo, !self.undo.is_empty(), actions);
                item(ui, "Redo", Action::Redo, !self.redo.is_empty(), actions);
                ui.separator();
                let selected = self.selection.is_some();
                item(ui, "Select All", Action::SelectAll, has_image, actions);
                item(ui, "Deselect", Action::Deselect, selected, actions);
                // Ctrl+C/X/V come as clipboard events, not key presses.
                for (label, action, enabled, keys) in [
                    ("Copy", Action::Copy, has_image, "Ctrl+C"),
                    ("Paste", Action::Paste, has_image, "Ctrl+V"),
                    ("Cut", Action::Cut, selected, "Ctrl+X"),
                ] {
                    if ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys)).clicked() {
                        actions.push(action);
                    }
                }
                item(ui, "Delete Selection", Action::DeleteSelection, selected, actions);
                item(ui, "Copy Selected Region", Action::Copy, selected, actions);
                item(ui, "Create Image from Selection", Action::ImageFromSelection, selected, actions);
                ui.separator();
                item(ui, "Crop", Action::Crop, has_image, actions);
                item(ui, "Resize…", Action::Resize, has_image, actions);
                ui.separator();
                item(ui, "Adjust Color…", Action::AdjustColor, has_image, actions);
                item(ui, "Reset Color Adjustments", Action::ResetAdjust, has_image, actions);
                ui.separator();
                item(ui, "Rotate Left", Action::RotateLeft, has_image, actions);
                item(ui, "Rotate Right", Action::RotateRight, has_image, actions);
                item(ui, "Flip Horizontally", Action::FlipHorizontal, has_image, actions);
                item(ui, "Flip Vertically", Action::FlipVertical, has_image, actions);
            });
            ui.menu_button("View", |ui| {
                item(ui, "Previous Image", Action::PrevImage, self.index > 0, actions);
                item(ui, "Next Image", Action::NextImage, self.index + 1 < self.files.len(), actions);
                ui.separator();
                item(ui, "Zoom In", Action::ZoomIn, has_image, actions);
                item(ui, "Zoom Out", Action::ZoomOut, has_image, actions);
                item(ui, "Actual Size", Action::ActualSize, has_image, actions);
                item(ui, "Fit to Window", Action::Fit, has_image, actions);
                ui.separator();
                item(ui, "Enter Full Screen", Action::Fullscreen, true, actions);
                ui.separator();
                let label = if self.show_info { "Hide Image Information" } else { "Show Image Information" };
                item(ui, label, Action::Info, has_image, actions);
                ui.separator();
                let animated = loaded.is_some_and(|l| l.frames.len() > 1);
                ui.add_enabled_ui(animated, |ui| {
                    ui.menu_button("Animation", |ui| {
                        item(ui, "Play Animation", Action::Play, self.anim_paused, actions);
                        item(ui, "Pause Animation", Action::Pause, !self.anim_paused, actions);
                        item(ui, "Next Frame", Action::NextFrame, true, actions);
                        item(ui, "Previous Frame", Action::PrevFrame, true, actions);
                    });
                });
            });
            ui.menu_button("Tools", |ui| {
                let jpeg = self.current().and_then(|p| p.extension()).is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
                item(ui, "Edit EXIF…", Action::EditExif, has_image && jpeg, actions);
                for (label, kind) in [("Rectangular Selection", SelKind::Rect), ("Elliptical Selection", SelKind::Ellipse), ("Lasso", SelKind::Lasso)] {
                    if ui.add_enabled(has_image, egui::Button::selectable(self.tool == Tool::Selection(kind), label)).clicked() {
                        actions.push(Action::SelectTool(kind));
                    }
                }
                {
                    ui.separator();
                    ui.weak("AI Image Processing");
                    let runtime = crate::ai::runtime_installed();
                    let models = crate::ai::installed();
                    use crate::ai::Task;
                    let ready = |task| has_image && runtime && self.restoring.is_none() && crate::ai::choose(self.settings.ai.choice(task), task, &models).is_some();
                    item(ui, "Remove Background", Action::Cutout(false), ready(Task::Background), actions);
                    item(ui, "Remove Background and Crop", Action::Cutout(true), ready(Task::Background), actions);
                    item(ui, "Denoise", Action::Restore(Task::Denoise), ready(Task::Denoise), actions);
                    item(ui, "Enhance Image", Action::Restore(Task::Enhance), ready(Task::Enhance), actions);
                    item(ui, "AI Models…", Action::AiModels, true, actions);
                    if !runtime {
                        item(ui, "Download AI Runtime…", Action::DownloadRuntime, true, actions);
                    }
                }
                ui.separator();
                let measuring = self.tool == Tool::Measure;
                if ui.add_enabled(has_image, egui::Button::selectable(measuring, "Measure")).clicked() {
                    actions.push(Action::Measure);
                }
                ui.add_enabled_ui(has_image, |ui| {
                    ui.menu_button("Adjust", |ui| {
                        // They all open the same panel.
                        for label in ["Adjust Color…", "Exposure…", "Contrast…", "Saturation…", "Tint…"] {
                            item(ui, label, Action::AdjustColor, true, actions);
                        }
                        item(ui, "Auto Levels", Action::AutoLevels, true, actions);
                        item(ui, "Reset Adjustments", Action::ResetAdjust, true, actions);
                    });
                });
            });
            ui.menu_button("Draw", |ui| {
                let label = if self.show_toolbar { "Hide Drawing Toolbar" } else { "Show Drawing Toolbar" };
                item(ui, label, Action::ShowToolbar, has_image, actions);
                ui.separator();
                item(ui, "Sketch", Action::Pick(Pick::Draw(Shape::Sketch)), has_image, actions);
                ui.add_enabled_ui(has_image, |ui| {
                    ui.menu_button("Shapes", |ui| {
                        for (shape, name) in markup::SHAPES {
                            item(ui, name, Action::Pick(Pick::Draw(shape)), true, actions);
                        }
                    });
                });
                item(ui, "Text", Action::Pick(Pick::Draw(Shape::Text)), has_image, actions);
                item(ui, "Text Formatting…", Action::Pick(Pick::Draw(Shape::Text)), has_image, actions);
                item(ui, "Border Color…", Action::StyleControls, has_image, actions);
                item(ui, "Fill Color…", Action::StyleControls, has_image, actions);
                item(ui, "Highlight", Action::Pick(Pick::Draw(Shape::Highlight)), has_image, actions);
                item(ui, "Magnifier", Action::Pick(Pick::Draw(Shape::Magnifier)), has_image, actions);
                ui.separator();
                item(ui, "Image Description…", Action::Describe, has_image, actions);
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

    /// The inspector sidebar, shown while `show_info` is on.
    fn inspector(&mut self, ui: &mut egui::Ui) {
        let (Some(path), true) = (self.current().cloned(), self.show_info) else { return };
        let Some(l) = self.cache.get(&path) else { return };
        if self.inspector.as_ref().is_none_or(|i| i.path != path) {
            self.inspector = Some(Inspector::new(&path));
        }
        let doc = self.doc.clone();
        let ins = self.inspector.as_mut().unwrap();
        let save = egui::Panel::right("inspector").size_range(240.0..=320.0).default_size(320.0).resizable(true).show(ui, |ui| inspector::ui(ui, ins, l, doc.as_ref())).inner;
        let Some(d) = ins.exif.as_ref().ok().filter(|_| save) else { return };
        let stem = d.path.file_stem().unwrap_or_default().to_string_lossy();
        let mut file_dialog = rfd::FileDialog::new().set_file_name(format!("{stem}-edited.jpg")).add_filter("JPEG", &["jpg", "jpeg"]);
        if let Some(dir) = d.path.parent() {
            file_dialog = file_dialog.set_directory(dir);
        }
        let (source, original, values, remove_gps) = (d.path.clone(), d.original.clone(), d.values.clone(), d.remove_gps);
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            if let Some(dest) = file_dialog.save_file() {
                let result = exif_edit::write_copy(&source, &dest, &original, &values, remove_gps);
                notify.send(Msg::Saved(dest, result));
            }
        });
    }

    /// The markup toolbar, below the menu bar.
    fn toolbar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let Some(doc) = self.doc() else { return };
        let defaults = *self.style.get_or_insert_with(|| Style::for_image(doc.size_vec()));
        let selected = self.selected.and_then(|i| doc.elements.get(i));
        let (mut style, styled) = match (selected, self.tool) {
            (Some(e), _) => (e.style, Some(e.shape)),
            (None, Tool::Draw(shape)) => (defaults, Some(shape)),
            _ => (defaults, None),
        };
        let pick = self.pick();
        let out = egui::Panel::top("markup").show(ui, |ui| markup::toolbar_ui(ui, pick, &mut style, styled, !self.undo.is_empty(), !self.redo.is_empty())).inner;
        if out.style_changed {
            match self.selected {
                Some(i) => self.restyle(i, style),
                None => self.style = Some(style),
            }
        }
        actions.extend(out.pick.map(Action::Pick));
        actions.extend(out.undo.then_some(Action::Undo));
        actions.extend(out.redo.then_some(Action::Redo));
        actions.extend(out.describe.then_some(Action::Describe));
        actions.extend(out.done.then_some(Action::ShowToolbar));
    }

    /// Select and Draw tools: dragging and clicking on the image. `dest`/`vis` is the layout.
    fn markup_input(&mut self, ctx: &egui::Context, drag: &egui::Response, dest: Rect, vis: Rect, doc: &Doc) {
        if drag.hovered() && self.tool != Tool::Select {
            ctx.set_cursor_icon(CursorIcon::Crosshair);
        }
        let k = dest.width() / vis.width();
        let bounds = Rect::from_min_size(Pos2::ZERO, doc.size_vec());
        let at = |p: Pos2| bounds.clamp(view::to_image(dest, vis, p));
        let origin = ctx.input(|i| i.pointer.press_origin());
        let pointer = drag.interact_pointer_pos();
        let style = *self.style.get_or_insert_with(|| Style::for_image(doc.size_vec()));
        let hit = |p: Pos2| doc.elements.iter().rposition(|e| e.shape != Shape::Erase && e.bbox().expand(6.0 / k).contains(p));
        match self.tool {
            Tool::Draw(shape @ (Shape::Text | Shape::Magnifier)) => {
                if drag.clicked() && let Some(p) = pointer.map(at) {
                    self.finish_text();
                    let mut e = Element::new(shape, p, style);
                    if shape == Shape::Magnifier {
                        e.points[1] = p + Vec2::new(bounds.width().min(bounds.height()) / 8.0, 0.0);
                    }
                    let Some(d) = self.doc_mut() else { return };
                    d.elements.push(e);
                    let i = d.elements.len() - 1;
                    if shape == Shape::Text {
                        (self.editing_text, self.focus_text, self.change_base) = (Some(i), true, Some(doc.clone()));
                    } else {
                        self.commit(doc.clone());
                    }
                }
            }
            Tool::Draw(shape) => {
                if drag.drag_started() && let Some(o) = origin {
                    self.change_base = Some(doc.clone());
                    if let Some(d) = self.doc_mut() {
                        d.elements.push(Element::new(shape, at(o), style));
                    }
                }
                if drag.dragged() && self.change_base.is_some() && let Some(p) = pointer.map(at) {
                    let e = self.doc_mut().and_then(|d| d.elements.last_mut());
                    match e {
                        Some(e) if shape == Shape::Sketch => {
                            if e.points.last().is_none_or(|q| q.distance(p) * k >= 1.0) {
                                e.points.push(p);
                            }
                        }
                        Some(e) => e.points[1] = p,
                        None => {}
                    }
                }
                if drag.drag_stopped() && let Some(before) = self.change_base.take() {
                    if let Some(d) = self.doc_mut()
                        && d.elements.last().is_some_and(Element::is_empty)
                    {
                        d.elements.pop();
                    }
                    self.commit(before);
                }
            }
            _ => {
                // Select.
                if drag.clicked() {
                    self.finish_text();
                    self.selected = pointer.map(at).and_then(hit);
                }
                if drag.double_clicked() && let Some(i) = self.selected.filter(|&i| doc.elements[i].shape == Shape::Text) {
                    (self.editing_text, self.focus_text, self.change_base) = (Some(i), true, Some(doc.clone()));
                }
                if drag.drag_started() && let Some(o) = origin {
                    let handle = self.selected.and_then(|i| doc.elements.get(i)).is_some_and(|e| view::to_screen(dest, vis, e.bbox().max).distance(o) < 10.0);
                    self.grab = if handle {
                        Grab::Resize
                    } else if let Some(i) = hit(at(o)) {
                        self.selected = Some(i);
                        Grab::Move
                    } else {
                        Grab::None
                    };
                    if self.grab != Grab::None {
                        self.change_base = Some(doc.clone());
                    }
                }
                if drag.dragged() && self.grab != Grab::None && let Some(i) = self.selected {
                    let delta = drag.drag_delta() / k;
                    let grab = self.grab;
                    if let Some(e) = self.doc_mut().and_then(|d| d.elements.get_mut(i)) {
                        match (grab, e.shape) {
                            (Grab::Move, _) => e.translate(delta),
                            (_, Shape::Magnifier) => e.points[1] += delta,
                            _ => {
                                // Scale about the top-left corner so the handle follows the pointer.
                                let b = e.bbox();
                                let f = ((b.size() + delta) / b.size()).max(Vec2::splat(0.05));
                                if e.shape == Shape::Text {
                                    e.style.font_size = (e.style.font_size * f.y).max(4.0);
                                } else {
                                    e.map(|p| b.min + (p - b.min) * f, 1.0);
                                }
                            }
                        }
                    }
                }
                if drag.drag_stopped() && let Some(before) = self.change_base.take() {
                    self.grab = Grab::None;
                    self.commit(before);
                }
            }
        }
    }

    /// Draws the markup over the image, plus the selection and the text being typed.
    #[allow(clippy::too_many_arguments)]
    fn paint_markup(&mut self, ui: &egui::Ui, doc: &Doc, dest: Rect, crop: Rect, texture: egui::TextureId, uv: &dyn Fn(Pos2) -> Pos2) {
        let k = dest.width() / crop.width();
        let to_screen = |p: Pos2| view::to_screen(dest, crop, p);
        let painter = ui.painter().with_clip_rect(dest);
        let mut used = Vec::new();
        for (i, e) in doc.elements.iter().enumerate() {
            let own = e.texture_key().and_then(|key| {
                used.push(key);
                if let std::collections::hash_map::Entry::Vacant(slot) = self.element_textures.entry(key) {
                    let image = e.texture_image(ui.ctx().input(|i| i.max_texture_side) as f32)?;
                    slot.insert(ui.ctx().load_texture("markup", image, TextureOptions::LINEAR));
                }
                Some(self.element_textures[&key].id())
            });
            if Some(i) != self.editing_text {
                markup::paint(&painter, e, &to_screen, k, texture, uv, own);
            }
        }
        self.element_textures.retain(|key, _| used.contains(key));
        if self.tool == Tool::Select && let Some(e) = self.selected.and_then(|i| doc.elements.get(i)) {
            let r = Rect::from_two_pos(to_screen(e.bbox().min), to_screen(e.bbox().max)).expand(3.0);
            painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, Color32::from_rgb(66, 165, 245)), egui::StrokeKind::Outside);
            painter.rect_filled(Rect::from_center_size(r.max, Vec2::splat(9.0)), 1.0, Color32::from_rgb(66, 165, 245));
        }
        let Some(i) = self.editing_text else { return };
        let Some(e) = doc.elements.get(i) else { return };
        let mut text = e.text.clone();
        let font = egui::FontId::proportional(e.style.font_size * k);
        let edit = egui::TextEdit::multiline(&mut text).font(font).text_color(e.style.color).frame(egui::Frame::NONE).desired_width(f32::INFINITY.min(dest.right() - to_screen(e.points[0]).x).max(40.0)).desired_rows(1);
        let response = egui::Area::new(egui::Id::unique("markup-text")).fixed_pos(to_screen(e.points[0])).order(egui::Order::Foreground).show(ui.ctx(), |ui| ui.add(edit)).inner;
        if std::mem::take(&mut self.focus_text) {
            response.request_focus();
        }
        if text != e.text && let Some(e) = self.doc_mut().and_then(|d| d.elements.get_mut(i)) {
            e.text = text;
        }
        if response.lost_focus() {
            self.finish_text();
        }
    }

    /// The colour that erased pixels get: transparent if the image has alpha or its format can
    /// hold it (a PNG without an alpha channel too), else the background.
    fn erase_color(&self) -> Color32 {
        let format_alpha = self.current().and_then(|p| p.extension()).and_then(|e| e.to_str()).is_some_and(|e| {
            ["png", "webp", "gif", "tif", "tiff"].contains(&e.to_ascii_lowercase().as_str())
        });
        let alpha = format_alpha || self.current().and_then(|p| self.cache.get(p)).is_some_and(|l| l.frames[0].alpha);
        let [r, g, b] = self.settings.background;
        if alpha { Color32::TRANSPARENT } else { Color32::from_rgb(r, g, b) }
    }

    /// Copies the selection (or the whole image) with its edits to the clipboard.
    fn copy_selection(&mut self, ctx: &egui::Context) {
        let Some((l, doc)) = self.current().and_then(|p| self.cache.get(p)).cloned().zip(self.doc()) else { return };
        // shortcut: renders the full edited image on the UI thread; move it to a thread if copying big images stutters.
        let Ok(img) = edit::render(&l.frames[self.anim_frame.min(l.frames.len() - 1)], &doc) else { return };
        let img = img.into_rgba8();
        let bounds = Rect::from_min_size(Pos2::ZERO, doc.size_vec());
        let sel = self.selection.clone().unwrap_or(Selection::Rect(bounds));
        let b = Rect::from_min_max(sel.bbox().min.floor(), sel.bbox().max.ceil()).intersect(bounds);
        let (x, y, w, h) = (b.min.x as u32, b.min.y as u32, b.width() as u32, b.height() as u32);
        if w == 0 || h == 0 {
            return;
        }
        let mut part = image::imageops::crop_imm(&img, x, y, w, h).to_image();
        if !matches!(sel, Selection::Rect(_)) && let Some(mask) = markup::polygon_mask(&sel.outline(), None, b.min, 1.0, w, h) {
            for (px, m) in part.pixels_mut().zip(mask.data()) {
                px[3] = (px[3] as u32 * *m as u32 / 255) as u8;
            }
        }
        ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], part.as_raw()));
        self.status = Some((format!("Copied {w} × {h} px"), Instant::now()));
    }

    fn delete_selection(&mut self) {
        let (Some(sel), Some(doc)) = (self.selection.take(), self.doc()) else { return };
        let (bounds, color) = (Rect::from_min_size(Pos2::ZERO, doc.size_vec()), self.erase_color());
        self.edit(|d| d.elements.push(Element::erase(&sel, false, bounds, color)));
    }

    /// Crops to the selection; for an ellipse or lasso, the outside is erased too.
    fn image_from_selection(&mut self) {
        let (Some(sel), Some(doc)) = (self.selection.take(), self.doc()) else { return };
        let r = Rect::from_min_max(sel.bbox().min.round(), sel.bbox().max.round()).intersect(Rect::from_min_size(Pos2::ZERO, doc.size_vec()));
        let color = self.erase_color();
        self.edit(|d| {
            d.crop_to(r);
            if !matches!(sel, Selection::Rect(_)) {
                d.elements.push(Element::erase(&sel.translate(-r.min.to_vec2()), true, Rect::from_min_size(Pos2::ZERO, d.size_vec()), color));
            }
        });
    }

    /// Adds a clipboard image (straight RGBA) as a movable element, centred and fitted inside the image.
    fn paste(&mut self, w: u32, h: u32, pixels: Vec<u8>) {
        let (Some(img), Some(doc)) = (image::RgbaImage::from_raw(w, h, pixels), self.doc()) else { return };
        let size = Vec2::new(w as f32, h as f32);
        let fit = (doc.size_vec() / size).min_elem().min(1.0);
        let at = self.selection.as_ref().map_or(((doc.size_vec() - size * fit) / 2.0).to_pos2(), |s| s.bbox().min);
        let style = *self.style.get_or_insert_with(|| Style::for_image(doc.size_vec()));
        self.edit(|d| d.elements.push(Element::pasted(Arc::new(img), at, size * fit, style)));
        self.finish_text();
        (self.tool, self.show_toolbar, self.selection) = (Tool::Select, true, None);
        self.selected = self.doc.as_ref().map(|d| d.elements.len() - 1);
    }

    /// Runs background removal on the edited image on another thread, cropping to the subject when `crop`.
    fn cutout(&mut self, crop: bool) {
        let Some((l, doc, path)) = self.current().and_then(|p| Some((self.cache.get(p)?.clone(), self.doc()?, p.clone()))) else { return };
        let task = crate::ai::Task::Background;
        let Some(file) = crate::ai::choose(self.settings.ai.choice(task), task, &crate::ai::installed()) else { return };
        let preset = crate::ai::preset(&file, &self.settings.ai.overrides);
        self.status = Some((format!("Running {file}…"), Instant::now()));
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            let result = edit::render(&l.frames[0], &doc).and_then(|img| {
                let img = img.into_rgba8();
                let pre = match preset {
                    Some(p) => p,
                    None => crate::ai::Preprocess::auto(crate::ai::input_size(&file)?),
                };
                let mask = crate::ai::mask(&file, &pre, &img)?;
                let object = if crop { Some(crate::ai::bbox(&mask).ok_or("no subject found")?) } else { None };
                Ok((crate::ai::cutout(&img, &mask), object))
            });
            notify.send(Msg::Cutout(path, result));
        });
    }

    /// Denoise / Enhance on the part of the image on screen (at most 256 px square, so it takes
    /// seconds), for the before/after preview.
    fn preview_restore(&mut self, task: crate::ai::Task) {
        let (Some(vis), Some(doc)) = (self.visible, self.doc()) else { return };
        let size = vis.size().min(Vec2::splat(256.0));
        let region = Rect::from_center_size(vis.center(), size);
        let region = Rect::from_min_max(region.min.round(), region.max.round()).intersect(Rect::from_min_size(Pos2::ZERO, doc.size_vec()));
        self.compare = None;
        self.run_restore(task, region, false, None);
    }

    /// Runs Denoise / Enhance on `region` (output px) of the edited image on another thread.
    fn run_restore(&mut self, task: crate::ai::Task, region: Rect, whole: bool, selection: Option<Selection>) {
        let Some((l, doc, path)) = self.current().and_then(|p| Some((self.cache.get(p)?.clone(), self.doc()?, p.clone()))) else { return };
        let Some(file) = crate::ai::choose(self.settings.ai.choice(task), task, &crate::ai::installed()) else { return };
        if region.width() < 1.0 || region.height() < 1.0 || self.restoring.is_some() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.restoring = Some(Restoring { task, progress: 0.0, started: Instant::now(), cancel: cancel.clone(), selection });
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = edit::render(&l.frames[0], &doc).and_then(|img| {
                let img = img.into_rgba8();
                let part = image::imageops::crop_imm(&img, region.min.x as u32, region.min.y as u32, region.width() as u32, region.height() as u32).to_image();
                let progress = |p: f32| notify.send(Msg::Progress(p));
                let out = crate::ai::restore(&file, &part, &progress, &cancel)?;
                let mp = (out.width() * out.height()) as f64 / 1e6;
                Ok((out, started.elapsed().as_secs_f64() * 1000.0 / mp))
            });
            notify.send(Msg::Restored(path, region, whole, result));
        });
    }

    /// The bottom bar while Denoise / Enhance runs: progress, time left, Cancel.
    fn restore_progress(&mut self, ctx: &egui::Context, viewport: Rect) {
        let Some(r) = &self.restoring else { return };
        let left = (r.progress > 0.02).then(|| r.started.elapsed().as_secs_f64() * (1.0 - r.progress as f64) / r.progress as f64);
        let on = if crate::ai::on_gpu() { " on the GPU" } else { "" };
        let label = format!("{}{on}… {:.0} %{}", r.task.title(), r.progress * 100.0, left.map_or(String::new(), |s| format!(" · about {} left", format_duration(s))));
        let cancel = egui::Area::new(egui::Id::unique("restore-progress")).fixed_pos(Pos2::new(viewport.center().x - 200.0, viewport.bottom() - 64.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(400.0);
                ui.label(label);
                ui.horizontal(|ui| {
                    ui.add(egui::ProgressBar::new(r.progress).desired_width(320.0));
                    ui.button("Cancel").clicked()
                }).inner
            }).inner
        }).inner;
        if cancel {
            r.cancel.store(true, Ordering::Relaxed);
        }
        // The time left updates even between tiles.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }

    /// The bar over the before/after preview: Apply to Whole Image / Selection, Discard.
    fn compare_bar(&mut self, ctx: &egui::Context, viewport: Rect) {
        let Some(c) = &self.compare else { return };
        let Some(doc) = self.doc() else { return };
        let mp = doc.size_vec().x as f64 * doc.size_vec().y as f64 / 1e6;
        // shortcut: overlapping 512 px tiles do ~1.4× the image's pixels on a 12 MP photo; count the tiles if estimates are off.
        let estimate = format_duration(c.ms_per_mp * mp * 1.4 / 1000.0);
        let selection = self.selection.clone().filter(|s| !s.is_empty());
        let (whole, sel, discard) = egui::Area::new(egui::Id::unique("compare-bar")).fixed_pos(Pos2::new(viewport.center().x - 260.0, viewport.top() + 12.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(format!("{} preview of the visible area · drag to compare · Before | After", c.task.title()));
                ui.horizontal(|ui| {
                    let whole = ui.button(format!("Apply to Whole Image (about {estimate})")).clicked();
                    let sel = selection.is_some() && ui.button("Apply to Selection").clicked();
                    (whole, sel, ui.button("Discard").clicked())
                }).inner
            }).inner
        }).inner;
        let task = c.task;
        if whole || sel || discard {
            self.compare = None;
        }
        if whole {
            self.run_restore(task, Rect::from_min_size(Pos2::ZERO, doc.size_vec()), true, None);
        } else if sel && let Some(s) = selection {
            let b = Rect::from_min_max(s.bbox().min.floor(), s.bbox().max.ceil()).intersect(Rect::from_min_size(Pos2::ZERO, doc.size_vec()));
            self.run_restore(task, b, true, Some(s));
        }
    }

    /// Shown before the first Enhance Image.
    fn enhance_notice_dialog(&mut self, ctx: &egui::Context) {
        if !self.enhance_notice {
            return;
        }
        let modal = egui::Modal::new(egui::Id::unique("enhance-notice")).show(ctx, |ui| {
            ui.heading("Enhance Image");
            ui.label("Enhance Image uses generative AI. To make the image look better it may invent or alter fine details, such as textures, text or faces, that weren't in the original.");
            ui.label("For a faithful clean-up, use Denoise instead.");
            ui.add_space(6.0);
            ui.horizontal(|ui| (ui.button("Continue").clicked(), ui.button("Cancel").clicked())).inner
        });
        let (go, cancel) = modal.inner;
        if go {
            self.settings.ai.enhance_notice_seen = true;
            self.enhance_notice = false;
            self.preview_restore(crate::ai::Task::Enhance);
        } else if cancel || modal.should_close() {
            self.enhance_notice = false;
        }
    }

    /// Auto Levels for the visible part of the current image.
    fn auto_levels(&self) -> Option<[[u8; 2]; 3]> {
        let frame = &self.cache.get(self.current()?)?.frames[0];
        Some(edit::auto_levels(frame, self.doc()?.crop))
    }

    /// The Adjust Color panel: the sliders change the edits live, and Done makes them one undo step.
    fn adjust_panel(&mut self, ctx: &egui::Context) {
        let (Some(before), Some(mut doc)) = (self.adjusting.clone(), self.doc()) else { return };
        let levels = self.auto_levels();
        let mut open = true;
        let choice = egui::Window::new("Adjust Color").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            edit::adjust_ui(ui, &mut doc.adjust, || levels.unwrap_or([[0, 255]; 3]))
        }).and_then(|r| r.inner).flatten();
        let adjust = doc.adjust;
        self.doc = Some(doc);
        if choice.is_some() || !open {
            // Back to before the panel opened, then Done applies the result as one edit.
            self.adjusting = None;
            self.doc = Some(before);
            if choice == Some(true) {
                self.edit(|d| d.adjust = adjust);
            }
        }
    }

    fn ai_models_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_ai_models {
            return;
        }
        let (installed, runtime) = (crate::ai::installed(), crate::ai::runtime_installed());
        let modal = egui::Modal::new(egui::Id::unique("ai-models")).show(ctx, |ui| crate::ai::models_ui(ui, &mut self.settings.ai, &installed, &self.downloading, runtime));
        let close = modal.should_close();
        let out = modal.inner;
        if out.close || close {
            self.show_ai_models = false;
        }
        let notify = self.notify.clone();
        if let Some(i) = out.download {
            self.license_prompt = Some((Download::Model(i), 0));
        } else if out.download_gpu {
            self.license_prompt = Some((Download::Gpu, 0));
        } else if let Some(file) = out.detect {
            // Opening a big model takes a moment, so not on the UI thread.
            std::thread::spawn(move || notify.send(Msg::Detected(file.clone(), crate::ai::input_size(&file))));
        } else if out.add {
            std::thread::spawn(move || {
                if let Some(src) = rfd::FileDialog::new().add_filter("ONNX model", &["onnx"]).pick_file() {
                    notify.send(Msg::Status(match crate::ai::add_model(&src) {
                        Ok(name) => format!("Added {name}"),
                        Err(e) => format!("Couldn't add the model: {e}"),
                    }));
                }
            });
        }
    }

    /// Shows the download's licence; the download starts only once it's accepted.
    fn license_dialog(&mut self, ctx: &egui::Context) {
        let Some((download, step)) = self.license_prompt else { return };
        let (title, licenses, note) = match download {
            Download::Model(i) => (crate::ai::CATALOG[i].label, crate::ai::CATALOG[i].licenses, crate::ai::CATALOG[i].converted_by),
            Download::Runtime => ("AI runtime (ONNX Runtime)", std::slice::from_ref(&crate::ai::RUNTIME_LICENSE), None),
            Download::Gpu => ("GPU support", &crate::ai::GPU_LICENSES[..], Some("Downloaded from Microsoft (ONNX Runtime) and NVIDIA (CUDA libraries, via their official PyPI packages).")),
        };
        let modal = egui::Modal::new(egui::Id::unique("license")).show(ctx, |ui| crate::ai::license_ui(ui, title, &licenses[step], step, licenses.len(), note));
        let close = modal.should_close();
        match modal.inner {
            Some(true) if step + 1 < licenses.len() => self.license_prompt = Some((download, step + 1)),
            Some(true) => {
                self.license_prompt = None;
                let notify = self.notify.clone();
                match download {
                    Download::Model(i) => {
                        let known = &crate::ai::CATALOG[i];
                        self.downloading.insert(known.file.to_string());
                        std::thread::spawn(move || notify.send(Msg::Downloaded(known.file.to_string(), crate::ai::download(known))));
                    }
                    Download::Gpu => {
                        self.downloading.insert("gpu".into());
                        self.status = Some((format!("Downloading GPU support ({} MB)…", crate::ai::GPU_DOWNLOAD_MB), Instant::now()));
                        std::thread::spawn(move || notify.send(Msg::Downloaded("gpu".into(), crate::ai::download_gpu())));
                    }
                    Download::Runtime => {
                        self.status = Some(("Downloading the AI runtime…".into(), Instant::now()));
                        std::thread::spawn(move || {
                            notify.send(Msg::Status(match crate::ai::download_runtime() {
                                Ok(()) => "Installed the AI runtime".into(),
                                Err(e) => format!("Couldn't install the AI runtime: {e}"),
                            }));
                        });
                    }
                }
            }
            Some(false) => self.license_prompt = None,
            None if close => self.license_prompt = None,
            None => {}
        }
    }

    fn resize_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.resize_dialog else { return };
        let modal = egui::Modal::new(egui::Id::unique("resize")).show(ctx, |ui| edit::resize_ui(ui, dialog));
        match modal.inner {
            Some(true) => {
                let (size, dpi) = dialog.result();
                self.resize_dialog = None;
                self.edit(|d| {
                    d.resize(size[0], size[1]);
                    d.dpi = dpi.or(d.dpi);
                });
            }
            Some(false) => self.resize_dialog = None,
            None if modal.should_close() => self.resize_dialog = None,
            None => {}
        }
    }

    /// Asks before an action that would lose unsaved edits.
    fn discard_dialog(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            return;
        }
        let name = self.current().and_then(|p| p.file_name()).unwrap_or_default().to_string_lossy().into_owned();
        let modal = egui::Modal::new(egui::Id::unique("discard")).show(ctx, |ui| {
            ui.heading("Discard edits?");
            ui.label(format!("Your edits to {name} haven't been saved."));
            ui.add_space(6.0);
            ui.horizontal(|ui| (ui.button("Discard").clicked(), ui.button("Cancel").clicked())).inner
        });
        let (discard, cancel) = modal.inner;
        if discard && let Some(p) = self.pending.take() {
            self.doc = None;
            self.confirm(p);
        } else if cancel || modal.should_close() {
            self.pending = None;
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
        storage.set_string("toolbar", if self.show_toolbar { "shown" } else { "hidden" }.into());
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
        crate::ai::set_use_gpu(self.settings.ai.use_gpu);
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }

        // Plain keys (arrows, F11) and undo belong to a text field while one has focus.
        let typing = ctx.egui_wants_keyboard_input();
        let mut actions: Vec<Action> = ctx.input_mut(|i| {
            SHORTCUTS
                .iter()
                .filter(|(sc, a)| !(typing && (sc.modifiers.is_none() || matches!(a, Action::Undo | Action::Redo | Action::SelectAll | Action::Deselect))) && i.consume_shortcut(sc))
                .map(|(_, a)| a.clone())
                .collect()
        });
        let (esc, fullscreen) = ctx.input(|i| (i.key_pressed(Key::Escape), i.viewport().fullscreen.unwrap_or(false)));
        let mut pasted = None;
        if !typing {
            ctx.input(|i| {
                for event in &i.events {
                    match event {
                        egui::Event::Copy => actions.push(Action::Copy),
                        egui::Event::Cut => actions.push(Action::Cut),
                        egui::Event::PasteImage(img) => pasted = Some(img.clone()),
                        // Text on the clipboard: there may be an image too.
                        egui::Event::Paste(_) => actions.push(Action::Paste),
                        _ => {}
                    }
                }
            });
        }
        if !fullscreen {
            egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui, &mut actions));
            if self.show_toolbar {
                self.toolbar(ui, &mut actions);
            }
        }
        for action in &actions {
            match action {
                Action::Open => self.open_dialog(),
                Action::SaveAs => self.save_dialog(None),
                Action::Export(i) => self.save_dialog(Some(*i)),
                Action::OpenRecent(path) => self.confirm(Pending::Open(path.clone())),
                Action::Close => self.confirm(Pending::Close),
                Action::Undo => self.undo_redo(true),
                Action::Redo => self.undo_redo(false),
                Action::Crop if self.doc().is_some() => (self.tool, self.crop_rect) = (Tool::Crop, None),
                Action::Resize => self.resize_dialog = self.doc().map(|d| ResizeDialog::new(&d)),
                Action::Info => self.show_info = !self.show_info,
                Action::Settings => self.show_settings = true,
                Action::EditExif => self.show_info = true,
                Action::AdjustColor => {
                    if self.adjusting.is_none() {
                        self.adjusting = self.doc();
                    }
                }
                Action::AutoLevels => {
                    if let Some(levels) = self.auto_levels() {
                        self.edit(|d| d.adjust.levels = Some(levels));
                    }
                }
                Action::ResetAdjust => self.edit(|d| d.adjust = Adjust::default()),
                Action::Measure if self.tool == Tool::Measure => (self.tool, self.measure) = (Tool::Pan, None),
                Action::Measure if self.doc().is_some() => (self.tool, self.crop_rect) = (Tool::Measure, None),
                Action::ShowToolbar => {
                    self.show_toolbar = !self.show_toolbar;
                    if !self.show_toolbar {
                        self.finish_text();
                        (self.tool, self.selected) = (Tool::Pan, None);
                    }
                }
                Action::Pick(pick) if self.doc().is_some() => self.set_pick(*pick),
                Action::StyleControls if self.doc().is_some() => {
                    if !matches!(self.tool, Tool::Select | Tool::Draw(_)) {
                        self.set_pick(Pick::Draw(Shape::Rect));
                    }
                    self.show_toolbar = true;
                }
                Action::Describe => self.show_info = true,
                Action::SelectTool(kind) if self.doc().is_some() => {
                    self.finish_text();
                    (self.tool, self.crop_rect, self.selected) = (Tool::Selection(*kind), None, None);
                }
                Action::SelectAll => self.selection = self.doc().map(|d| Selection::Rect(Rect::from_min_size(Pos2::ZERO, d.size_vec()))),
                Action::Deselect => self.selection = None,
                Action::Copy => self.copy_selection(&ctx),
                Action::Cut => {
                    if self.selection.is_some() {
                        self.copy_selection(&ctx);
                        self.delete_selection();
                    }
                }
                Action::Paste => match arboard::Clipboard::new().and_then(|mut c| c.get_image()) {
                    Ok(img) => self.paste(img.width as u32, img.height as u32, img.bytes.into_owned()),
                    Err(e) => self.status = Some((format!("Nothing to paste: {e}"), Instant::now())),
                },
                Action::DeleteSelection => self.delete_selection(),
                Action::Cutout(crop) => self.cutout(*crop),
                Action::Restore(task) if *task == crate::ai::Task::Enhance && !self.settings.ai.enhance_notice_seen => self.enhance_notice = true,
                Action::Restore(task) => self.preview_restore(*task),
                Action::DownloadRuntime => self.license_prompt = Some((Download::Runtime, 0)),
                Action::AiModels => self.show_ai_models = true,
                Action::ImageFromSelection => self.image_from_selection(),
                Action::RotateLeft => self.edit(Doc::rotate_left),
                Action::RotateRight => self.edit(Doc::rotate_right),
                Action::FlipHorizontal => self.edit(Doc::flip_horizontal),
                Action::FlipVertical => self.edit(Doc::flip_vertical),
                Action::Fullscreen => ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen)),
                Action::PrevImage => self.confirm(Pending::Navigate(-1)),
                Action::NextImage => self.confirm(Pending::Navigate(1)),
                Action::Play => (self.anim_paused, self.anim_due) = (false, None),
                Action::Pause => self.anim_paused = true,
                _ => {} // zoom and frame-step actions are applied below, once the image is known
            }
        }
        if let Some(img) = pasted {
            let pixels = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            self.paste(img.size[0] as u32, img.size[1] as u32, pixels);
        }
        let enter = !typing && ctx.input(|i| i.key_pressed(Key::Enter));
        let delete = !typing && ctx.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace));
        if esc && !typing && let Some(before) = self.change_base.take() {
            // Cancel the drawing in progress.
            (self.doc, self.grab) = (Some(before), Grab::None);
        } else if esc && !typing && self.tool != Tool::Pan {
            if matches!(self.tool, Tool::Selection(_)) {
                self.selection = None;
            }
            (self.tool, self.crop_rect, self.measure, self.selected) = (Tool::Pan, None, None, None);
        } else if esc && !typing && fullscreen {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        if enter && self.tool == Tool::Crop && let Some(r) = self.crop_rect.take() {
            self.edit(|d| d.crop_to(r));
            self.tool = Tool::Pan;
        }
        if delete && self.tool == Tool::Select && let Some(i) = self.selected.take() {
            self.edit(|d| drop(d.elements.remove(i)));
        } else if delete {
            self.delete_selection();
        }

        self.inspector(ui);
        let viewport = ui.available_rect_before_wrap();
        let ppp = ctx.pixels_per_point();
        self.loader.set_target([(viewport.width() * ppp) as u32, (viewport.height() * ppp) as u32]);
        let doc = self.doc();
        let full = self.current().and_then(|p| self.cache.get(p)).cloned();
        let full = full.as_deref();

        // Zoom and pan only once the full image is in (the preview has different pixel sizes).
        if let (Some(l), Some(doc)) = (full, &doc) {
            let img = doc.size_vec();
            let drag = ui.interact(viewport, ui.scope_id().with("image"), Sense::click_and_drag());
            if let Some(c) = &mut self.compare {
                // Dragging moves the before/after line instead of using the tool.
                let (dest, vis) = view::layout(self.view, img, viewport, ppp);
                if drag.dragged() && let Some(p) = drag.interact_pointer_pos() {
                    let left = view::to_screen(dest, vis, c.region.min).x;
                    let right = view::to_screen(dest, vis, c.region.max).x;
                    c.split = ((p.x - left) / (right - left).max(1.0)).clamp(0.0, 1.0);
                }
            } else {
            match self.tool {
                Tool::Pan if drag.dragged() => self.view = view::pan(self.view, drag.drag_delta(), img, viewport, ppp),
                Tool::Pan => {}
                Tool::Select | Tool::Draw(_) => {
                    let (dest, vis) = view::layout(self.view, img, viewport, ppp);
                    self.markup_input(&ctx, &drag, dest, vis, doc);
                }
                Tool::Selection(kind) => {
                    if drag.hovered() {
                        ctx.set_cursor_icon(CursorIcon::Crosshair);
                    }
                    let (dest, vis) = view::layout(self.view, img, viewport, ppp);
                    let bounds = Rect::from_min_size(Pos2::ZERO, img);
                    let at = |p: Pos2| bounds.clamp(view::to_image(dest, vis, p));
                    if drag.clicked() {
                        self.selection = None;
                    }
                    if drag.drag_started() && let Some(o) = ctx.input(|i| i.pointer.press_origin()) {
                        self.drag_start = Some(at(o));
                        self.selection = (kind == SelKind::Lasso).then(|| Selection::Lasso(vec![at(o)]));
                    }
                    if drag.dragged() && let (Some(a), Some(p)) = (self.drag_start, drag.interact_pointer_pos().map(at)) {
                        let k = dest.width() / vis.width();
                        match (kind, &mut self.selection) {
                            (SelKind::Lasso, Some(Selection::Lasso(points))) => {
                                if points.last().is_none_or(|q| q.distance(p) * k >= 2.0) {
                                    points.push(p);
                                }
                            }
                            (SelKind::Ellipse, sel) => *sel = Some(Selection::Ellipse(Rect::from_two_pos(a, p))),
                            (_, sel) => *sel = Some(Selection::Rect(Rect::from_two_pos(a, p))),
                        }
                    }
                    if drag.drag_stopped() && self.selection.as_ref().is_some_and(Selection::is_empty) {
                        self.selection = None;
                    }
                }
                Tool::Crop | Tool::Measure => {
                    if drag.hovered() {
                        ctx.set_cursor_icon(CursorIcon::Crosshair);
                    }
                    let (dest, vis) = view::layout(self.view, img, viewport, ppp);
                    let bounds = Rect::from_min_size(Pos2::ZERO, img);
                    let at = |p: Pos2| bounds.clamp(view::to_image(dest, vis, p));
                    let origin = ctx.input(|i| i.pointer.press_origin());
                    if drag.drag_started() && let Some(o) = origin {
                        self.drag_start = Some(at(o));
                        // Measure: grab an endpoint near the press, or start a new line.
                        let near = self.measure.and_then(|m| (0..2).find(|&i| view::to_screen(dest, vis, m[i]).distance(o) < 10.0));
                        match near {
                            Some(i) => self.measure_drag = i,
                            None if self.tool == Tool::Measure => (self.measure, self.measure_drag) = (Some([at(o); 2]), 1),
                            None => {}
                        }
                    }
                    if drag.dragged() && let (Some(a), Some(p)) = (self.drag_start, drag.interact_pointer_pos()) {
                        match (self.tool, &mut self.measure) {
                            (Tool::Measure, Some(m)) => m[self.measure_drag] = at(p),
                            (Tool::Crop, _) => self.crop_rect = Some(Rect::from_two_pos(a, at(p))),
                            _ => {}
                        }
                    }
                }
            }
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
            for action in &actions {
                let step = match action {
                    Action::NextFrame => 1,
                    Action::PrevFrame => frames.len() - 1,
                    _ => continue,
                };
                self.anim_frame = (self.anim_frame + step) % frames.len();
                self.anim_paused = true;
            }
            if frames.len() > 1 && !self.anim_paused {
                let now = Instant::now();
                let due = *self.anim_due.get_or_insert(now + frames[self.anim_frame].delay);
                if now >= due {
                    self.anim_frame = (self.anim_frame + 1) % frames.len();
                    self.anim_due = Some(now + frames[self.anim_frame].delay);
                }
                ctx.request_repaint_after(self.anim_due.unwrap() - now);
            }
        }

        let (frame, view, doc) = match (full, doc) {
            (Some(l), Some(doc)) => (Some(&l.frames[self.anim_frame]), self.view, doc),
            _ => (self.preview.as_ref(), View::Fit, self.preview.as_ref().map(|f| Doc::new(f.w, f.h)).unwrap_or(Doc::new(1, 1))),
        };
        if let Some(frame) = frame {
            let frame_size = Vec2::new(frame.w as f32, frame.h as f32);
            let (dest, crop) = view::layout(view, doc.size_vec(), viewport, ppp);
            let interpolation = self.settings.interpolation();
            // Images bigger than the GPU's texture limit always take the CPU path.
            // shortcut: colour adjustments only run on the CPU path; add a shader if GPU users miss smooth zoom while adjusting.
            let gpu = self.settings.gpu_scaling && frame.w.max(frame.h) as usize <= ctx.input(|i| i.max_texture_side) && doc.adjust.is_identity();
            let key = if gpu {
                TexKey { generation: self.generation, anim_frame: self.anim_frame, gpu, interpolation, transform: Transform::default(), crop: [0.0; 4], size: [frame.w, frame.h], adjust: Adjust::default() }
            } else {
                let src = doc.to_source(crop);
                let size = [(dest.width() * ppp).round().max(1.0) as u32, (dest.height() * ppp).round().max(1.0) as u32];
                TexKey { generation: self.generation, anim_frame: self.anim_frame, gpu, interpolation, transform: doc.transform, crop: [src.min.x, src.min.y, src.width(), src.height()], size, adjust: doc.adjust }
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
                    let unadjusted = TexKey { adjust: Adjust::default(), ..key };
                    if self.resized.as_ref().map(|r| r.0) != Some(unadjusted) {
                        let [x, y, w, h] = key.crop;
                        self.resized = Some((unadjusted, resize(frame, Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)), key.size, doc.transform, interpolation)));
                    }
                    let pixels = &self.resized.as_ref().unwrap().1;
                    let size = [key.size[0] as usize, key.size[1] as usize];
                    let image = if doc.adjust.is_identity() {
                        to_color_image(frame.alpha, size, pixels)
                    } else {
                        // Only the adjustment re-runs while a slider moves.
                        let mut pixels = pixels.clone();
                        doc.adjust.apply(&mut pixels, if frame.alpha { 4 } else { 3 });
                        to_color_image(frame.alpha, size, &pixels)
                    };
                    (image, TextureOptions::LINEAR)
                };
                match &mut self.texture {
                    Some(t) => t.set(image, options),
                    None => self.texture = Some(ctx.load_texture("image", image, options)),
                }
                self.tex_key = Some(key);
            }
            let texture = self.texture.as_ref().unwrap().id();
            if frame.alpha {
                // Transparent parts show as a checkerboard, not as the background.
                let painter = ui.painter().with_clip_rect(dest.intersect(viewport));
                painter.add(markup::checker(dest.intersect(viewport), None));
            }
            if gpu {
                // Each screen corner samples the source point the transform maps it from.
                let mut mesh = egui::Mesh::with_texture(texture);
                let shown = [crop.left_top(), crop.right_top(), crop.right_bottom(), crop.left_bottom()];
                let screen = [dest.left_top(), dest.right_top(), dest.right_bottom(), dest.left_bottom()];
                for (pos, p) in screen.into_iter().zip(shown) {
                    let uv = (doc.source_point(p).to_vec2() / frame_size).to_pos2();
                    mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
                }
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                ui.painter().add(mesh);
            } else {
                ui.painter().image(texture, dest, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            let frame_wh = [frame.w, frame.h];
            self.visible = full.is_some().then_some(crop);
            if let Some(c) = &self.compare {
                // The result on the right of the line, the original on the left.
                let r = Rect::from_two_pos(view::to_screen(dest, crop, c.region.min), view::to_screen(dest, crop, c.region.max));
                let x = r.left() + r.width() * c.split;
                let after = Rect::from_min_max(Pos2::new(x, r.top()), r.max);
                ui.painter().with_clip_rect(after.intersect(dest)).image(c.texture.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                let painter = ui.painter().with_clip_rect(dest);
                painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, Color32::from_white_alpha(160)), egui::StrokeKind::Outside);
                painter.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], egui::Stroke::new(2.0, Color32::WHITE));
                painter.circle_filled(Pos2::new(x, r.center().y), 6.0, Color32::WHITE);
            }
            if !doc.elements.is_empty() || self.editing_text.is_some() || !self.element_textures.is_empty() {
                // The magnifier samples the image texture: the whole frame (GPU) or the visible part (CPU).
                let uv = |p: Pos2| if gpu { (doc.source_point(p).to_vec2() / frame_size).to_pos2() } else { ((p - crop.min) / crop.size()).to_pos2() };
                self.paint_markup(ui, &doc, dest, crop, texture, &uv);
            }
            if let Some(sel) = &self.selection {
                let mut outline: Vec<Pos2> = sel.outline().into_iter().map(|p| view::to_screen(dest, crop, p)).collect();
                outline.push(outline[0]);
                // Marching-ants style: white dashes over a dark line, visible on any image.
                let painter = ui.painter().with_clip_rect(dest.expand(1.0));
                painter.add(egui::Shape::line(outline.clone(), egui::Stroke::new(1.0, Color32::from_black_alpha(200))));
                painter.extend(egui::Shape::dashed_line(&outline, egui::Stroke::new(1.0, Color32::WHITE), 5.0, 4.0));
            }
            if self.tool == Tool::Crop {
                let painter = ui.painter().with_clip_rect(dest);
                let shade = Color32::from_black_alpha(140);
                match self.crop_rect.map(|r| Rect::from_two_pos(view::to_screen(dest, crop, r.min), view::to_screen(dest, crop, r.max))) {
                    Some(r) => {
                        // Darken everything outside the crop.
                        for side in [
                            Rect::from_x_y_ranges(dest.x_range(), dest.top()..=r.top()),
                            Rect::from_x_y_ranges(dest.x_range(), r.bottom()..=dest.bottom()),
                            Rect::from_x_y_ranges(dest.left()..=r.left(), r.y_range()),
                            Rect::from_x_y_ranges(r.right()..=dest.right(), r.y_range()),
                        ] {
                            painter.rect_filled(side, 0.0, shade);
                        }
                        painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
                    }
                    None => {
                        painter.rect_filled(dest, 0.0, shade);
                    }
                }
                bubble(ui.painter(), Pos2::new(viewport.center().x, viewport.top() + 24.0), "Drag to select · Enter to crop · Esc to cancel");
            }
            if self.tool == Tool::Measure {
                let hint = match self.measure {
                    Some(m) => {
                        let [a, b] = m.map(|p| view::to_screen(dest, crop, p));
                        let painter = ui.painter().with_clip_rect(viewport);
                        // A dark halo keeps the guide line visible on light and dark images.
                        painter.line_segment([a, b], egui::Stroke::new(3.0, Color32::from_black_alpha(160)));
                        painter.line_segment([a, b], egui::Stroke::new(1.5, Color32::YELLOW));
                        for p in [a, b] {
                            painter.circle(p, 5.0, Color32::from_black_alpha(120), egui::Stroke::new(1.5, Color32::YELLOW));
                        }
                        let [dx, dy, d] = doc.measure(m[0], m[1]);
                        format!("Distance: {d:.0} px · ΔX: {dx:.0} px · ΔY: {dy:.0} px")
                    }
                    None => "Drag to measure in image pixels · Esc to stop".into(),
                };
                bubble(ui.painter(), Pos2::new(viewport.center().x, viewport.top() + 24.0), &hint);
            }

            if self.timings {
                if full.is_none() && self.preview_reported != Some(frame_wh) {
                    eprintln!("preview     {:.1} ms  ({}×{})", ms(self.t0), frame_wh[0], frame_wh[1]);
                    self.preview_reported = Some(frame_wh);
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
                bubble(ui.painter(), Pos2::new(viewport.center().x, viewport.bottom() - 24.0), text);
                ctx.request_repaint_after(SHOWN - at.elapsed());
            } else {
                self.status = None;
            }
        }

        self.settings_dialog(&ctx);
        self.resize_dialog(&ctx);
        self.ai_models_dialog(&ctx);
        self.license_dialog(&ctx);
        self.enhance_notice_dialog(&ctx);
        self.restore_progress(&ctx, viewport);
        self.compare_bar(&ctx, viewport);
        self.adjust_panel(&ctx);
        self.discard_dialog(&ctx);

        let title = self.make_title();
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// Resizes the `crop` part of `frame` so that, after `transform`, it's `size` px.
/// Transforming the small resized output instead of the full image keeps rotating cheap.
fn resize(frame: &Frame, crop: Rect, size: [u32; 2], transform: Transform, interpolation: Interpolation) -> Vec<u8> {
    let [w, h] = if transform.rot % 2 == 1 { [size[1], size[0]] } else { size };
    let pixels = edit::resize(frame, crop, w, h, interpolation.cpu());
    if transform == Transform::default() {
        pixels
    } else {
        transform.apply(edit::to_dynamic(frame.alpha, w, h, pixels).unwrap()).into_bytes()
    }
}

/// A short message in a dark rounded box centred on `center`.
fn bubble(painter: &egui::Painter, center: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(14.0), Color32::WHITE);
    let rect = Rect::from_center_size(center, galley.size() + Vec2::new(16.0, 8.0));
    painter.rect_filled(rect, 4.0, Color32::from_black_alpha(190));
    painter.galley(rect.min + Vec2::new(8.0, 4.0), galley, Color32::WHITE);
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

/// "40 s", "3 min", "1 h 5 min".
fn format_duration(secs: f64) -> String {
    let secs = secs.max(1.0).round() as u64;
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs.div_ceil(60)),
        _ => format!("{} h {} min", secs / 3600, secs % 3600 / 60),
    }
}

pub fn format_size(bytes: u64) -> String {
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
        assert_eq!(out.len(), 2 * 3 * 3);
        // Rotated right, the top row reads bottom-left then top-left of the source: 3, 0.
        let red: Vec<u8> = out.chunks(3).map(|p| p[0]).collect();
        assert_eq!(red, [3, 0, 4, 1, 5, 2]);
    }

    /// Clipboard between two processes (needs a desktop session):
    /// `cargo test clipboard_copy -- --ignored & sleep 3; cargo test clipboard_paste -- --ignored`
    #[test]
    #[ignore]
    fn clipboard_copy() {
        let bytes: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
        let mut clipboard = arboard::Clipboard::new().unwrap();
        clipboard.set_image(arboard::ImageData { width: 4, height: 3, bytes: bytes.into() }).unwrap();
        // Like the app's window, keep owning the clipboard for a while.
        std::thread::sleep(std::time::Duration::from_secs(8));
    }

    #[test]
    #[ignore]
    fn clipboard_paste() {
        let img = arboard::Clipboard::new().unwrap().get_image().unwrap();
        assert_eq!((img.width, img.height), (4, 3));
        assert_eq!(img.bytes[..8], [0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(0.2), "1 s");
        assert_eq!(format_duration(42.4), "42 s");
        assert_eq!(format_duration(61.0), "2 min");
        assert_eq!(format_duration(3725.0), "1 h 2 min");
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
