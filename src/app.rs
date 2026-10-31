use crate::i18n::{tr, trf};
use crate::edit::{self, Doc, ResizeDialog};
use crate::exif_edit;
use crate::inspector::{self, Inspector};
use crate::load::{self, Frame, Loaded, Loader, Msg, Notify};
use crate::markup::{self, Element, Selection, Shape, Style};
use crate::save;
use crate::settings::{self, Settings};
use crate::view::View;
use eframe::egui::{self, Color32, Key, Pos2, Rect, TextureHandle, TextureOptions, Vec2, ViewportCommand};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::time::Instant;

mod ai_tools;
mod canvas;
mod dialogs;
mod input;
mod menu;

use ai_tools::{Compare, Restoring};
use canvas::{BakeKey, TexKey};
use dialogs::Download;
use menu::Action;

const RECENT_MAX: usize = 10;

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

/// What to do once the user agrees to discard unsaved edits.
enum Pending {
    Navigate(isize),
    Open(PathBuf),
    Close,
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
    /// The first-launch question while it's open: the checkbox's state.
    default_prompt: Option<bool>,
    default_result: Option<Result<(), String>>,
    inspector: Option<Inspector>,
    files: Vec<PathBuf>,
    index: usize,
    /// Decoded images for current ±1 only.
    // Arc so a frame can hold the current image while calling `&mut self` methods.
    cache: HashMap<PathBuf, Arc<Loaded>>,
    preview: Option<Arc<Frame>>,
    error: Option<String>,
    view: View,
    texture: Option<TextureHandle>,
    tex_key: Option<TexKey>,
    /// The CPU path's resized pixels before colour adjustments, and what they hold.
    resized: Option<(TexKey, Vec<u8>)>,
    /// The current frame at its output size, while the edits shrink it.
    baked: Option<(BakeKey, Arc<Frame>)>,
    /// Bumped whenever what's shown changes source (navigation, preview → full).
    generation: u64,
    anim_frame: usize,
    anim_due: Option<Instant>,
    anim_paused: bool,
    /// Edits to the current image (None: none yet), with undo/redo history.
    doc: Option<Doc>,
    undo: Vec<Doc>,
    redo: Vec<Doc>,
    /// Images moved to the Trash this session, newest last; Ctrl+Z brings them back.
    deleted: Vec<PathBuf>,
    delete_prompt: bool,
    /// The edits last saved to a copy.
    saved: Option<Doc>,
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
    /// How many Denoise / Enhance runs were started: the last run's id.
    restore_runs: u64,
    compare: Option<Compare>,
    /// Enhance Image waiting for the user to read its model's notice: the model file.
    enhance_notice: Option<String>,
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
    pub fn new(cc: &eframe::CreationContext, t0: Instant, timings: bool, bench: bool, rx: mpsc::Receiver<Msg>, loader: Loader, notify: Notify, mut settings: Settings, start: Option<PathBuf>) -> Self {
        // Otherwise egui's own Ctrl +/− scales the whole UI.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        cc.egui_ctx.set_fonts(markup::font_definitions());
        let mut recent: Vec<PathBuf> = cc.storage.and_then(|s| s.get_string("recent")).unwrap_or_default().lines().map(PathBuf::from).collect();
        let show_toolbar = cc.storage.and_then(|s| s.get_string("toolbar")).is_some_and(|v| v == "shown");
        if let Some(path) = &start {
            push_recent(&mut recent, path);
        }
        // Spawns xdg-mime, so only until the answer is known: this stays off normal startups.
        let mut default_prompt = None;
        if !settings.default_asked && settings::desktop_installed() {
            settings.default_asked = settings::is_default();
            default_prompt = (!settings.default_asked).then_some(true);
        }
        App {
            t0, timings, bench, rx, loader, notify, recent, settings, show_info: false, show_settings: false, default_prompt, default_result: None, inspector: None,
            files: start.into_iter().collect(), index: 0,
            cache: HashMap::new(), preview: None, error: None, view: View::Fit,
            texture: None, tex_key: None, resized: None, baked: None, generation: 0, anim_frame: 0, anim_due: None, anim_paused: false,
            doc: None, undo: Vec::new(), redo: Vec::new(), deleted: Vec::new(), delete_prompt: false, saved: None, tool: Tool::Pan, drag_start: None, crop_rect: None, measure: None, measure_drag: 1,
            show_toolbar, style: None, selected: None, grab: Grab::None, editing_text: None, focus_text: false, change_base: None, last_restyle: None, selection: None, element_textures: HashMap::new(),
            resize_dialog: None, adjusting: None, pending: None, show_ai_models: false, license_prompt: None, restoring: None, restore_runs: 0, compare: None, enhance_notice: None, visible: None, downloading: HashSet::new(),
            title: String::new(),
            window_reported: false, preview_reported: None, full_reported: false, nav_start: None, close_next: false, status: None,
        }
    }

    /// Shows a short message at the bottom.
    fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
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
            Msg::Status(text) => self.set_status(text),
            Msg::Progress(id, p) => {
                if let Some(r) = self.restoring.as_mut().filter(|r| r.id == id) {
                    r.progress = p;
                }
            }
            Msg::Restored(id, path, region, whole, result) => {
                let Some(run) = self.restoring.take_if(|r| r.id == id) else { return };
                match result {
                    Ok(_) if self.current() != Some(&path) => {}
                    Ok((img, ms_per_mp)) if !whole => {
                        let image = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                        let texture = self.notify.ctx.get().map(|ctx| ctx.load_texture("restored", image, TextureOptions::LINEAR));
                        let scale = (img.width() as f32 / region.width()).round().max(1.0) as u32;
                        self.compare = texture.map(|texture| Compare { task: run.task, region, texture, split: 0.5, ms_per_mp, scale });
                    }
                    Ok((mut img, _)) => {
                        if let Some(sel) = &run.selection {
                            clip_to(&mut img, sel, region.min);
                        }
                        let style = Style::for_image(region.size());
                        let scale = (img.width() as f32 / region.width()).round().max(1.0);
                        self.edit(|d| {
                            if scale > 1.0 {
                                // An upscaler ran on the whole image: the image becomes that many times bigger.
                                let size = d.size_vec() * scale;
                                d.resize(size.x as u32, size.y as u32);
                                d.elements.push(Element::pasted(Arc::new(img), Pos2::ZERO, size, style));
                            } else {
                                d.elements.push(Element::pasted(Arc::new(img), region.min, region.size(), style));
                            }
                        });
                        self.set_status(trf("{task} done", &[("task", &run.task.title())]));
                    }
                    Err(e) if e == "cancelled" => self.set_status(tr("Cancelled")),
                    Err(e) => self.set_status(trf("{task} failed: {e}", &[("task", &run.task.title()), ("e", &e)])),
                }
            }
            Msg::Downloaded(file, result) => {
                self.downloading.remove(&file);
                self.set_status(match result {
                    Ok(()) if file == "gpu" => tr("GPU support installed: restart lightview to use it").into(),
                    Ok(()) => trf("Downloaded {file}", &[("file", &file)]),
                    Err(e) => trf("Couldn't download {file}: {e}", &[("file", &file), ("e", &e)]),
                });
            }
            Msg::Detected(file, result) => match result {
                Ok(size) => {
                    let base = crate::ai::preset(&file, &self.settings.ai.overrides).unwrap_or(crate::ai::Preprocess::auto(size));
                    self.settings.ai.overrides.insert(file, crate::ai::Preprocess { size, ..base });
                }
                Err(e) => self.set_status(trf("Couldn't read {file}: {e}", &[("file", &file), ("e", &e)])),
            },
            Msg::Cutout(path, result) => match result {
                Ok((img, object)) if self.current() == Some(&path) => {
                    let Some(doc) = self.doc() else { return };
                    let bounds = doc.bounds();
                    let style = *self.style.get_or_insert_with(|| Style::for_image(doc.size_vec()));
                    // Erase everything, then put the cut-out back on top: one undo step.
                    self.edit(|d| {
                        d.elements.push(Element::erase(&Selection::Rect(bounds), false, bounds, Color32::TRANSPARENT));
                        d.elements.push(Element::pasted(Arc::new(img), Pos2::ZERO, bounds.size(), style));
                        if let Some(r) = object {
                            d.crop_to(r);
                        }
                    });
                    self.set_status(tr("Done"));
                }
                Ok(_) => {}
                Err(e) => self.set_status(trf("Couldn't process the image: {e}", &[("e", &e)])),
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
                    self.preview = Some(Arc::new(frame));
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
            Msg::Saved(path, result, written) => {
                if let (Ok(()), Some((source, doc))) = (&result, written)
                    && self.current() == Some(&source)
                {
                    self.saved = Some(doc);
                }
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                self.set_status(match &result {
                    Ok(()) => trf("Saved {name}", &[("name", &name)]),
                    Err(e) => trf("Couldn't save {name}: {e}", &[("name", &name), ("e", &e)]),
                });
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

    fn undo_redo(&mut self, undo: bool) {
        if undo && self.undo.is_empty() && let Some(path) = self.deleted.pop() {
            match load::restore_trashed(&path) {
                Ok(()) => self.open(path),
                Err(e) => self.set_status(trf("Couldn't restore the image: {e}", &[("e", &e)])),
            }
            return;
        }
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
        (self.doc, self.saved) = (None, None);
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
        let mut dialog = rfd::FileDialog::new().add_filter(tr("Images"), &load::EXTENSIONS);
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
    fn save_dialog(&mut self) {
        let Some((path, l)) = self.current().and_then(|p| Some((p, self.cache.get(p)?))) else { return };
        let Some(doc) = self.doc().filter(|_| l.frames.len() == 1) else { return };
        let (frame, source) = (l.frames[0].clone(), path.clone());
        let mut dialog = save::formats_for(path).iter().fold(rfd::FileDialog::new().set_file_name(path.file_name().unwrap_or_default().to_string_lossy()), |d, (name, exts)| d.add_filter(*name, exts));
        if let Some(dir) = path.parent() {
            dialog = dialog.set_directory(dir);
        }
        let notify = self.notify.clone();
        // The system dialog asks before replacing an existing file.
        std::thread::spawn(move || {
            if let Some(dest) = dialog.save_file() {
                let result = save::save(&frame, &doc, &source, &dest);
                notify.send(Msg::Saved(dest, result, Some((source, doc))));
            }
        });
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
                notify.send(Msg::Saved(dest, result, None));
            }
        });
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
        let bounds = doc.bounds();
        let sel = self.selection.clone().unwrap_or(Selection::Rect(bounds));
        let b = Rect::from_min_max(sel.bbox().min.floor(), sel.bbox().max.ceil()).intersect(bounds);
        let (x, y, w, h) = (b.min.x as u32, b.min.y as u32, b.width() as u32, b.height() as u32);
        if w == 0 || h == 0 {
            return;
        }
        let mut part = image::imageops::crop_imm(&img, x, y, w, h).to_image();
        if !matches!(sel, Selection::Rect(_)) {
            clip_to(&mut part, &sel, b.min);
        }
        ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], part.as_raw()));
        self.set_status(trf("Copied {w} × {h} px", &[("w", &w), ("h", &h)]));
    }

    fn delete_selection(&mut self) {
        let (Some(sel), Some(doc)) = (self.selection.take(), self.doc()) else { return };
        let (bounds, color) = (doc.bounds(), self.erase_color());
        self.edit(|d| d.elements.push(Element::erase(&sel, false, bounds, color)));
    }

    /// Moves the current image to the Trash. If that fails the file stays where it is.
    fn delete_image(&mut self) {
        let Some(path) = self.current().cloned() else { return };
        if let Err(e) = trash::delete(&path) {
            self.set_status(trf("Couldn't delete the image: {e}", &[("e", &e)]));
            return;
        }
        self.files.remove(self.index);
        self.index = self.index.min(self.files.len().saturating_sub(1));
        self.cache.remove(&path);
        self.recent.retain(|p| *p != path);
        self.deleted.push(path);
        self.reset_shown();
        self.request_decodes();
    }

    /// Crops to the selection; for an ellipse or lasso, the outside is erased too.
    fn image_from_selection(&mut self) {
        let (Some(sel), Some(doc)) = (self.selection.take(), self.doc()) else { return };
        let r = Rect::from_min_max(sel.bbox().min.round(), sel.bbox().max.round()).intersect(doc.bounds());
        let color = self.erase_color();
        self.edit(|d| {
            d.crop_to(r);
            if !matches!(sel, Selection::Rect(_)) {
                d.elements.push(Element::erase(&sel.translate(-r.min.to_vec2()), true, d.bounds(), color));
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

        let typing = ctx.egui_wants_keyboard_input();
        let mut actions = App::read_shortcuts(&ctx, typing);
        let (esc, fullscreen) = ctx.input(|i| (i.key_pressed(Key::Escape), i.viewport().fullscreen.unwrap_or(false)));
        let pasted = if typing { None } else { App::read_clipboard(&ctx, &mut actions) };
        if !fullscreen {
            egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui, &mut actions));
            if self.show_toolbar {
                self.toolbar(ui, &mut actions);
            }
        }
        self.run_actions(&actions, &ctx, fullscreen);
        if let Some(img) = pasted {
            self.paste_event(&img);
        }
        self.handle_keys(&ctx, typing, esc, fullscreen);

        self.inspector(ui);
        let viewport = ui.available_rect_before_wrap();
        let ppp = ctx.pixels_per_point();
        self.loader.set_target([(viewport.width() * ppp) as u32, (viewport.height() * ppp) as u32]);
        let doc = self.doc();
        let full = self.current().and_then(|p| self.cache.get(p)).cloned();
        let full = full.as_deref();

        let mut hovered_at = None;
        // Zoom and pan only once the full image is in (the preview has different pixel sizes).
        if let (Some(l), Some(doc)) = (full, &doc) {
            let drag = self.image_input(ui, doc, viewport, ppp);
            hovered_at = drag.hover_pos().filter(|_| drag.hovered());
            self.zoom(&ctx, &drag, &actions, doc.size_vec(), viewport, ppp);
            self.animate(&ctx, l, &actions);
        }
        self.draw(ui, full, doc, hovered_at, viewport);
        self.show_status(ui, viewport);
        self.dialogs(&ctx, viewport);

        let title = self.make_title();
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// Makes `img`, whose top-left corner is at `origin`, transparent outside `sel`.
fn clip_to(img: &mut image::RgbaImage, sel: &Selection, origin: Pos2) {
    let Some(mask) = markup::polygon_mask(&sel.outline(), None, origin, 1.0, img.width(), img.height()) else { return };
    for (px, m) in img.pixels_mut().zip(mask.data()) {
        px[3] = (px[3] as u32 * *m as u32 / 255) as u8;
    }
}

/// Moves `path` to the front of `recent`, keeping at most `RECENT_MAX`.
fn push_recent(recent: &mut Vec<PathBuf>, path: &std::path::Path) {
    recent.retain(|p| p != path);
    recent.insert(0, path.to_path_buf());
    recent.truncate(RECENT_MAX);
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

    /// An app showing a decoded 2×1 image.
    fn app_with_image() -> (App, PathBuf) {
        let (tx, rx) = mpsc::channel();
        let notify = Notify { tx, ctx: Arc::default() };
        let loader = Loader::new(notify.clone(), 0, [100, 100]);
        let settings = Settings { default_asked: true, ..Settings::default() };
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = App::new(&cc, Instant::now(), false, false, rx, loader, notify, settings, None);
        let path = PathBuf::from("/a.png");
        let frame = Frame { w: 2, h: 1, pixels: vec![0; 6], alpha: false, delay: std::time::Duration::ZERO };
        app.cache.insert(path.clone(), Arc::new(Loaded { frames: vec![frame], file_size: 6 }));
        app.files = vec![path.clone()];
        (app, path)
    }

    #[test]
    fn only_the_edits_written_count_as_saved() {
        let (mut app, path) = app_with_image();
        app.edit(Doc::rotate_left);
        let doc = app.doc.clone().unwrap();
        let dest = PathBuf::from("/nonexistent/copy.jpg");
        // An EXIF copy, a failed save, and a Save As of the image shown before navigating.
        app.handle(Msg::Saved(dest.clone(), Ok(()), None));
        assert!(app.dirty());
        app.handle(Msg::Saved(dest.clone(), Err("disk full".into()), Some((path.clone(), doc.clone()))));
        assert!(app.dirty());
        app.handle(Msg::Saved(dest.clone(), Ok(()), Some(("/b.png".into(), doc.clone()))));
        assert!(app.dirty());
        app.handle(Msg::Saved(dest, Ok(()), Some((path, doc))));
        assert!(!app.dirty());
    }

    #[test]
    fn a_cancelled_restore_leaves_the_next_run_alone() {
        let (mut app, path) = app_with_image();
        app.restoring = Some(Restoring { id: 2, task: crate::ai::Task::Denoise, progress: 0.0, started: Instant::now(), cancel: Arc::default(), selection: None });
        app.handle(Msg::Progress(1, 0.5));
        app.handle(Msg::Restored(1, path.clone(), Rect::ZERO, false, Err("cancelled".into())));
        assert_eq!(app.restoring.as_ref().map(|r| r.progress), Some(0.0));
        assert!(app.status.is_none());
        app.handle(Msg::Progress(2, 0.5));
        assert_eq!(app.restoring.as_ref().map(|r| r.progress), Some(0.5));
        app.handle(Msg::Restored(2, path, Rect::ZERO, false, Err("cancelled".into())));
        assert!(app.restoring.is_none());
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
