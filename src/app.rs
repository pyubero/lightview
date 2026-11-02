use crate::curves::CurvePanel;
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
use std::sync::{Arc, Mutex, mpsc};
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
    Perspective,
    Measure,
    CopyColor,
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
    /// An SVG drawn sharper than its 1× frame: how many times bigger, and the pixels.
    sharp: Option<(u32, Arc<Frame>)>,
    /// The scale the zoom last asked for; 1 is the frame itself.
    sharp_wanted: u32,
    /// The image with its markup: for which generation and doc, and the pixels (`flattened`).
    flat: Option<(u64, Doc, Arc<Frame>)>,
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
    renaming: Option<dialogs::Renaming>,
    /// The file the watcher thread keeps an eye on.
    watched: Arc<Mutex<Option<PathBuf>>>,
    /// The state lightview itself last left the current file in, which isn't a change to react to.
    own_stamp: Option<load::Stamp>,
    /// Asking what to do: the file changed on disk, and there are unsaved edits.
    changed_prompt: bool,
    /// The navigator's thumbnail, and what it holds.
    navigator: Option<(TexKey, TextureHandle)>,
    /// The edits last saved to a copy.
    saved: Option<Doc>,
    tool: Tool,
    /// Crop tool: where the drag started and the rect so far, in image px.
    drag_start: Option<Pos2>,
    /// A drag started with the right button is under way: it pans.
    right_pan: bool,
    crop_rect: Option<Rect>,
    /// Perspective tool: the corners (output px, clockwise from the top left) and which one is being dragged.
    perspective: Option<[Pos2; 4]>,
    perspective_drag: Option<usize>,
    /// Asking before flattening the markup into the corrected image.
    perspective_prompt: bool,
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
    /// While Adjust Color or Adjust Curves is open: the edits from before it opened.
    adjusting: Option<Doc>,
    /// Some while the open panel is Adjust Curves rather than Adjust Color.
    curve_panel: Option<CurvePanel>,
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
        settings::apply_font_size(&cc.egui_ctx, settings.font_size);
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
        let watched = load::watch(notify.clone(), std::time::Duration::from_secs(1));
        App {
            t0, timings, bench, rx, loader, notify, recent, settings, show_info: false, show_settings: false, default_prompt, default_result: None, inspector: None,
            files: start.into_iter().collect(), index: 0,
            cache: HashMap::new(), preview: None, error: None, view: View::Fit,
            texture: None, tex_key: None, resized: None, baked: None, sharp: None, sharp_wanted: 1, flat: None, generation: 0, anim_frame: 0, anim_due: None, anim_paused: false,
            doc: None, undo: Vec::new(), redo: Vec::new(), deleted: Vec::new(), delete_prompt: false, renaming: None, watched, own_stamp: None, changed_prompt: false, navigator: None, saved: None, tool: Tool::Pan, drag_start: None, right_pan: false, crop_rect: None, perspective: None, perspective_drag: None, perspective_prompt: false, measure: None, measure_drag: 1,
            show_toolbar, style: None, selected: None, grab: Grab::None, editing_text: None, focus_text: false, change_base: None, last_restyle: None, selection: None, element_textures: HashMap::new(),
            resize_dialog: None, adjusting: None, curve_panel: None, pending: None, show_ai_models: false, license_prompt: None, restoring: None, restore_runs: 0, compare: None, enhance_notice: None, visible: None, downloading: HashSet::new(),
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

    /// What to decode: the window, without what's cached. Native decoders only get the image on
    /// screen: see `heif`.
    fn to_decode(&self) -> Vec<PathBuf> {
        let current = self.current();
        self.window().into_iter().filter(|p| !self.cache.contains_key(p) && (Some(p) == current || !crate::heif::is_native(p))).collect()
    }

    fn request_decodes(&mut self) {
        let window = self.window();
        self.cache.retain(|p, _| window.contains(p));
        self.loader.want(self.to_decode());
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
                        // The filled area should be seen, not outlined.
                        if run.task == crate::ai::Task::Inpaint {
                            self.selection = None;
                        }
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
            Msg::Changed(path, stamp) => {
                if self.current() != Some(&path) || self.own_stamp == Some(stamp) {
                    return;
                }
                if self.dirty() {
                    self.changed_prompt = true;
                } else {
                    self.reload();
                }
            }
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
            Msg::Sharp(path, k, frame) => {
                if self.current() == Some(&path) && self.sharp_wanted == k {
                    self.sharp = Some((k, Arc::new(frame)));
                    self.generation += 1;
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
                    self.own_stamp = load::stamp(&path);
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
            if doc.base != before.base {
                self.generation += 1;
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
        if current.base != doc.base {
            self.generation += 1;
        }
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

    /// Opens what was dragged onto the window: the first item, or a folder's first image.
    // shortcut: winit 0.30 delivers no drops on Wayland, so this only runs on X11 and Windows; a newer winit would bring it.
    fn open_dropped(&mut self, paths: Vec<PathBuf>) {
        let Some(path) = paths.into_iter().next() else { return };
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if path.is_dir() {
            let (sort, notify) = (self.settings.sort, self.notify.clone());
            std::thread::spawn(move || notify.send(match load::first_image(&path, sort) {
                Some(image) => Msg::Open(image),
                None => Msg::Status(trf("No images in {name}", &[("name", &name)])),
            }));
        } else if load::is_image(&path) {
            self.confirm(Pending::Open(path));
        } else {
            self.set_status(trf("Can't open {name}: not a supported image", &[("name", &name)]));
        }
    }

    /// Goes to the image at `index`. Already there is no move, so it can't touch any edits.
    fn go_to(&mut self, index: usize) {
        let delta = index as isize - self.index as isize;
        if delta != 0 {
            self.confirm(Pending::Navigate(delta));
        }
    }

    /// Moves by `delta` in the folder, stopping at the ends.
    fn navigate(&mut self, delta: isize) {
        let Some(index) = self.index.checked_add_signed(delta).filter(|&i| i < self.files.len()) else { return };
        self.index = index;
        self.reset_shown();
        self.nav_start = Some(Instant::now());
        self.request_decodes();
    }

    /// Shows the current file's new contents. The old pixels stay up until they're decoded, at the
    /// same zoom, so a file that changes often doesn't flicker.
    fn reload(&mut self) {
        let (Some(path), view) = (self.current().cloned(), self.view) else { return };
        self.reset_shown();
        self.view = view;
        self.inspector = None;
        self.loader.want(vec![path]);
    }

    /// Clears what's shown, before switching to another image.
    fn reset_shown(&mut self) {
        self.view = View::Fit;
        self.preview = None;
        self.error = None;
        (self.sharp, self.sharp_wanted) = (None, 1);
        self.generation += 1;
        self.anim_frame = 0;
        self.anim_due = None;
        self.anim_paused = false;
        (self.doc, self.saved) = (None, None);
        self.undo.clear();
        self.redo.clear();
        self.tool = Tool::Pan;
        self.crop_rect = None;
        (self.perspective, self.perspective_drag, self.perspective_prompt) = (None, None, false);
        self.measure = None;
        self.adjusting = None;
        (self.style, self.selected, self.grab, self.editing_text, self.change_base, self.last_restyle) = (None, None, Grab::None, None, None, None);
        self.selection = None;
        self.element_textures.clear();
        if let Some(r) = self.restoring.take() {
            r.cancel.store(true, Ordering::Relaxed);
        }
        self.compare = None;
        self.changed_prompt = false;
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

    /// What saving needs: the current still image's frame, its edits and its path.
    fn to_save(&self) -> Option<(Frame, Doc, PathBuf)> {
        let path = self.current()?;
        let l = self.cache.get(path)?;
        Some((l.frames[0].clone(), self.doc().filter(|_| l.frames.len() == 1)?, path.clone()))
    }

    /// Save Copy has something to write: with no edits it would only re-encode the file.
    fn can_save_copy(&self) -> bool {
        self.dirty() && self.current().and_then(|p| self.cache.get(p)).is_some_and(|l| l.frames.len() == 1)
    }

    /// Saves the current image (with its edits) next to it under a new name, with no questions.
    fn save_copy(&mut self) {
        if !self.can_save_copy() {
            return;
        }
        let Some((frame, doc, source)) = self.to_save() else { return };
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            let (dest, result) = match save::reserve_copy(&source) {
                Ok(dest) => {
                    let result = save::save(&frame, &doc, &source, &dest);
                    if result.is_err() {
                        let _ = std::fs::remove_file(&dest);
                    }
                    (dest, result)
                }
                Err(e) => (source.clone(), Err(e)),
            };
            notify.send(Msg::Saved(dest, result, Some((source, doc))));
        });
    }

    /// Asks where to save the current image (with its edits), then saves on that thread.
    fn save_dialog(&mut self) {
        let Some((frame, doc, source)) = self.to_save() else { return };
        let mut dialog = save::formats_for(&source).iter().fold(rfd::FileDialog::new().set_file_name(save::suggested_name(&source)), |d, (name, exts)| d.add_filter(*name, exts));
        if let Some(dir) = source.parent() {
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

    /// Renames the current image's file. It stays open, with its edits.
    fn rename_current(&mut self, name: &str) -> Result<(), String> {
        let Some(old) = self.current().cloned() else { return Ok(()) };
        let new = load::rename(&old, name)?;
        self.files[self.index] = new.clone();
        if let Some(l) = self.cache.remove(&old) {
            self.cache.insert(new.clone(), l);
        }
        if let Some(entry) = self.recent.iter_mut().find(|p| **p == old) {
            *entry = new.clone();
        }
        load::relist(&new, self.settings.sort, &self.notify);
        self.request_decodes();
        Ok(())
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

    /// Straightens what the perspective tool's corners enclose, as one undo step.
    fn apply_perspective(&mut self) {
        let (Some(corners), Some(l)) = (self.perspective, self.current().and_then(|p| self.cache.get(p)).cloned()) else { return };
        let mut result = Ok(());
        self.edit(|d| result = d.perspective(&l.frames[0], corners));
        match result {
            Ok(()) => (self.tool, self.perspective) = (Tool::Pan, None),
            Err(e) => self.set_status(e),
        }
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

    fn make_title(&self, zoom: Option<f32>) -> String {
        let Some(path) = self.current() else { return "lightview".into() };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let zoom = zoom.map_or(String::new(), |z| format!(" — {z:.2}×"));
        match self.cache.get(path) {
            Some(l) => {
                let page = if l.paged() { format!(" — {}", trf("page {n} of {count}", &[("n", &(self.anim_frame + 1)), ("count", &l.frames.len())])) } else { String::new() };
                format!("{name}{page} — {}×{} — {}{zoom}", l.frames[0].w, l.frames[0].h, format_size(l.file_size))
            }
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
        if let Ok(mut watched) = self.watched.lock() && watched.as_ref() != self.files.get(self.index) {
            *watched = self.files.get(self.index).cloned();
        }
        crate::ai::set_use_gpu(self.settings.ai.use_gpu);
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        self.open_dropped(dropped);

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

        let (mut hovered_at, mut zoom) = (None, None);
        // Zoom and pan only once the full image is in (the preview has different pixel sizes).
        if let (Some(l), Some(doc)) = (full, &doc) {
            let drag = self.image_input(ui, doc, viewport, ppp);
            hovered_at = drag.hover_pos().filter(|_| drag.hovered());
            self.zoom(&ctx, &drag, &actions, doc.size_vec(), viewport, ppp);
            zoom = Some(crate::view::scale(self.view, doc.size_vec(), viewport, ppp));
            self.animate(&ctx, l, &actions);
        }
        self.draw(ui, full, doc, hovered_at, viewport);
        self.show_status(ui, viewport);
        self.dialogs(&ctx, viewport);

        let title = self.make_title(zoom);
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
    use crate::view;

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
        app.cache.insert(path.clone(), Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None }));
        app.files = vec![path.clone()];
        (app, path)
    }

    /// `app_with_image`, with a 2×1 SVG as the image.
    fn app_with_svg() -> (App, PathBuf, Arc<Loaded>) {
        let (mut app, path) = app_with_image();
        let tree = crate::svg::load(br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"/>"#).unwrap();
        let frame = crate::svg::render(&tree, 1).unwrap();
        let loaded = Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: Some(tree) });
        app.cache.insert(path.clone(), loaded.clone());
        (app, path, loaded)
    }

    #[test]
    fn svg_sharpens_with_zoom_and_returns_to_1x() {
        let (mut app, _, l) = app_with_svg();
        let doc = Doc::new(2, 1);
        assert!(app.sharp_frame(Some(&l), &doc, 0.5).is_none());
        assert!(app.sharp_frame(Some(&l), &doc, 3.0).is_none(), "nothing until the render arrives");
        let msg = app.rx.recv().unwrap();
        app.handle(msg);
        let (k, frame) = app.sharp_frame(Some(&l), &doc, 3.0).unwrap();
        assert_eq!((k, frame.w, frame.h), (4, 8, 4));
        assert!(app.rx.try_recv().is_err(), "the same tier isn't rendered twice");
        // Still shown while the next tier renders, then back to 1× when zoomed out.
        assert_eq!(app.sharp_frame(Some(&l), &doc, 5.0).unwrap().0, 4);
        assert!(app.sharp_frame(Some(&l), &doc, 1.0).is_none());
        assert!(app.sharp.is_none());
        // The 16× render arrives too late.
        let msg = app.rx.recv().unwrap();
        app.handle(msg);
        assert!(app.sharp.is_none());
    }

    #[test]
    fn a_sharp_svg_draws_at_any_zoom_and_rotation_on_both_paths() {
        let (mut app, path) = app_with_image();
        let tree = crate::svg::load(br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"/>"#).unwrap();
        let l = Arc::new(Loaded { frames: vec![crate::svg::render(&tree, 1).unwrap()], file_size: 6, svg: Some(tree.clone()) });
        app.cache.insert(path.clone(), l.clone());
        let ctx = egui::Context::default();
        ctx.set_fonts(markup::font_definitions());
        for gpu in [true, false] {
            app.settings.gpu_scaling = gpu;
            app.reset_shown();
            app.sharp_wanted = 4;
            app.handle(Msg::Sharp(path.clone(), 4, crate::svg::render(&tree, 4).unwrap()));
            for quarter_turn in [false, true] {
                if quarter_turn {
                    app.edit(Doc::rotate_right);
                }
                for view in [View::Fit, View::Manual { scale: 3.3, center: Pos2::new(10.0, 10.0) }, View::Manual { scale: 2.0, center: Pos2::new(99.0, 49.0) }] {
                    app.view = view;
                    let mut input = egui::RawInput::default();
                    input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
                    let mut output = ctx.run_ui(input, |ui| {
                        let doc = app.doc();
                        app.draw(ui, Some(&l), doc, None, ui.available_rect_before_wrap());
                    });
                    output.textures_delta.clear();
                    assert_eq!(app.sharp_wanted, 4);
                    if gpu {
                        assert_eq!(app.texture.as_ref().unwrap().size(), [400, 200], "the whole sharp frame is uploaded");
                    }
                }
            }
        }
    }

    #[test]
    fn svg_with_markup_stays_at_1x_and_navigation_clears_it() {
        let (mut app, path, l) = app_with_svg();
        let mut doc = Doc::new(2, 1);
        doc.elements.push(Element::erase(&Selection::Rect(doc.bounds()), false, doc.bounds(), Color32::TRANSPARENT));
        assert!(app.sharp_frame(Some(&l), &doc, 8.0).is_none());
        assert_eq!(app.sharp_wanted, 1);
        app.sharp_wanted = 4;
        app.handle(Msg::Sharp(path, 4, l.frames[0].clone()));
        assert!(app.sharp.is_some());
        app.reset_shown();
        assert!(app.sharp.is_none() && app.sharp_wanted == 1);
    }

    #[test]
    fn native_decoders_only_get_the_image_on_screen() {
        let (mut app, _) = app_with_image();
        app.cache.clear();
        app.files = ["/a.heic", "/b.avif", "/c.png"].map(PathBuf::from).to_vec();
        app.index = 1;
        assert_eq!(app.to_decode(), ["/b.avif", "/c.png"].map(PathBuf::from));
        app.index = 2;
        assert_eq!(app.to_decode(), ["/c.png"].map(PathBuf::from));
    }

    #[test]
    fn pages_step_by_hand_and_never_play() {
        let (mut app, path) = app_with_image();
        let page = |shade: u8| Frame { w: 2, h: 1, pixels: vec![shade; 6], alpha: false, delay: std::time::Duration::ZERO };
        let loaded = Arc::new(Loaded { frames: vec![page(0), page(1), page(2)], file_size: 6, svg: None });
        app.cache.insert(path, loaded.clone());
        let ctx = egui::Context::default();
        for _ in 0..3 {
            app.animate(&ctx, &loaded, &[]);
        }
        assert_eq!(app.anim_frame, 0);
        assert!(app.make_title(None).contains(trf("page {n} of {count}", &[("n", &1), ("count", &3)]).as_str()));
        app.animate(&ctx, &loaded, &[crate::app::menu::Action::NextFrame]);
        assert_eq!(app.anim_frame, 1);
        app.animate(&ctx, &loaded, &[crate::app::menu::Action::PrevFrame, crate::app::menu::Action::PrevFrame]);
        assert_eq!(app.anim_frame, 2);
    }

    #[test]
    fn title_shows_the_zoom() {
        let (app, _) = app_with_image();
        assert_eq!(app.make_title(Some(0.25)), "a.png — 2×1 — 1 KB — 0.25×");
        assert_eq!(app.make_title(None), "a.png — 2×1 — 1 KB");
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

    #[test]
    fn removing_an_object_pastes_the_fill_once_and_clears_the_selection() {
        let (mut app, path) = app_with_image();
        let region = Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, 1.0));
        let sel = Selection::Rect(region);
        let run = |id, sel| Restoring { id, task: crate::ai::Task::Inpaint, progress: 0.0, started: Instant::now(), cancel: Arc::default(), selection: Some(sel) };
        // A failure keeps the selection for another try.
        app.selection = Some(sel.clone());
        app.restoring = Some(run(1, sel.clone()));
        app.handle(Msg::Restored(1, path.clone(), region, true, Err("boom".into())));
        assert!(app.selection.is_some() && app.doc().is_none_or(|d| d.elements.is_empty()));
        app.restoring = Some(run(2, sel));
        let fill = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
        app.handle(Msg::Restored(2, path, region, true, Ok((fill, 0.0))));
        assert!(app.selection.is_none() && app.restoring.is_none());
        assert_eq!(app.doc().map(|d| d.elements.len()), Some(1));
    }

    /// A big image at 1:1, panned by nothing yet.
    fn app_zoomed_in() -> (App, View) {
        let (mut app, path) = app_with_image();
        let frame = Frame { w: 1000, h: 1000, pixels: vec![0; 3_000_000], alpha: false, delay: std::time::Duration::ZERO };
        app.cache.insert(path, Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None }));
        app.view = crate::view::actual_size(Vec2::splat(1000.0));
        let view = app.view;
        (app, view)
    }

    fn press(button: egui::PointerButton, pressed: bool, at: f32) -> egui::Event {
        egui::Event::PointerButton { pos: Pos2::new(at, at), button, pressed, modifiers: egui::Modifiers::NONE }
    }

    fn drag_events(button: egui::PointerButton) -> Vec<egui::Event> {
        let to = |at| egui::Event::PointerMoved(Pos2::new(at, at));
        vec![to(100.0), press(button, true, 100.0), to(130.0), to(160.0), to(190.0), press(button, false, 190.0), to(190.0)]
    }

    /// Runs `image_input` for one frame per event, in a 400 px viewport.
    fn feed(app: &mut App, events: Vec<egui::Event>) {
        let ctx = egui::Context::default();
        ctx.set_fonts(markup::font_definitions());
        for event in events {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
            input.events.push(event);
            let mut output = ctx.run_ui(input, |ui| {
                let doc = app.doc().unwrap();
                app.image_input(ui, &doc, ui.available_rect_before_wrap(), 1.0);
            });
            output.textures_delta.clear();
        }
    }

    const LEFT: egui::PointerButton = egui::PointerButton::Primary;
    const RIGHT: egui::PointerButton = egui::PointerButton::Secondary;

    #[test]
    fn right_drag_pans_with_any_tool() {
        for tool in [Tool::Draw(Shape::Rect), Tool::Draw(Shape::Magnifier), Tool::Select, Tool::Selection(SelKind::Rect), Tool::Crop, Tool::Measure, Tool::Pan] {
            let (mut app, start) = app_zoomed_in();
            app.tool = tool;
            feed(&mut app, drag_events(RIGHT));
            assert_ne!(app.view, start);
            assert!(!app.right_pan);
            // The tool saw nothing.
            assert!(app.doc().unwrap().elements.is_empty() && app.change_base.is_none());
            assert!(app.selection.is_none() && app.crop_rect.is_none() && app.measure.is_none());
        }
    }

    #[test]
    fn left_drag_still_uses_the_tool_or_pans() {
        let (mut app, start) = app_zoomed_in();
        app.tool = Tool::Draw(Shape::Rect);
        feed(&mut app, drag_events(LEFT));
        assert_eq!(app.view, start);
        assert_eq!(app.doc().unwrap().elements.len(), 1);
        let (mut app, start) = app_zoomed_in();
        feed(&mut app, drag_events(LEFT));
        assert_ne!(app.view, start, "with no tool, left-drag pans");
    }

    #[test]
    fn a_right_press_during_a_left_drawing_leaves_it_alone() {
        let (mut app, start) = app_zoomed_in();
        app.tool = Tool::Draw(Shape::Rect);
        let mut events = drag_events(LEFT);
        // Tapped once the drawing is under way.
        events.splice(4..4, [press(RIGHT, true, 160.0), press(RIGHT, false, 160.0)]);
        feed(&mut app, events);
        assert_eq!(app.view, start);
        assert!(app.change_base.is_none() && !app.right_pan);
        assert_eq!(app.doc().unwrap().elements.len(), 1);
        assert_eq!(app.undo.len(), 1);
    }

    #[test]
    fn right_drag_pans_the_before_after_preview() {
        let (mut app, start) = app_zoomed_in();
        let texture = egui::Context::default().load_texture("t", egui::ColorImage::example(), TextureOptions::LINEAR);
        app.compare = Some(Compare { task: crate::ai::Task::Denoise, region: Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)), texture, split: 0.5, ms_per_mp: 0.0, scale: 1 });
        feed(&mut app, drag_events(RIGHT));
        assert_ne!(app.view, start);
        assert_eq!(app.compare.as_ref().map(|c| c.split), Some(0.5));
    }

    fn with_line(app: &mut App) -> Arc<Loaded> {
        let mut line = Element::new(Shape::Line, Pos2::new(1.0, 1.0), Style::for_image(Vec2::splat(1000.0)));
        line.points[1] = Pos2::new(50.0, 50.0);
        app.doc_mut().unwrap().elements.push(line);
        app.current().and_then(|p| app.cache.get(p)).unwrap().clone()
    }

    #[test]
    fn markup_is_shown_as_it_will_be_saved() {
        let (mut app, _) = app_zoomed_in();
        let l = with_line(&mut app);
        for bar in [true, false] {
            app.show_toolbar = bar;
            let doc = app.doc().unwrap();
            let flat = app.flattened(Some(&l), &doc).unwrap();
            assert_eq!(flat.pixels, edit::flat(&l.frames[0], &doc).unwrap().pixels);
            // Nothing changed: no new render, so nothing else is invalidated.
            let generation = app.generation;
            assert!(Arc::ptr_eq(&flat, &app.flattened(Some(&l), &doc).unwrap()));
            assert_eq!(app.generation, generation);
        }
        // A change renders again.
        let before = app.flattened(Some(&l), &app.doc().unwrap()).unwrap();
        app.doc_mut().unwrap().elements[0].points[1] = Pos2::new(90.0, 20.0);
        assert!(!Arc::ptr_eq(&before, &app.flattened(Some(&l), &app.doc().unwrap()).unwrap()));
    }

    #[test]
    fn flattening_stops_where_it_cant_apply() {
        let (mut app, _) = app_zoomed_in();
        let l = with_line(&mut app);
        let doc = app.doc().unwrap();
        assert!(app.flattened(Some(&l), &doc).is_some());
        // Still decoding.
        assert!(app.flattened(None, &doc).is_none() && app.flat.is_none());
        // No markup.
        assert!(app.flattened(Some(&l), &Doc::new(1000, 1000)).is_none());
        // Animated.
        let frame = || Frame { w: 2, h: 2, pixels: vec![0; 12], alpha: false, delay: std::time::Duration::ZERO };
        let animated = Loaded { frames: vec![frame(), frame()], file_size: 6, svg: None };
        assert!(app.flattened(Some(&animated), &doc).is_none());
        // Past the limit.
        let sized = |w, h| { let mut d = doc.clone(); d.resize(w, h); d };
        assert!(app.flattened(Some(&l), &sized(4000, 3000)).is_some(), "12 MP");
        assert!(app.flattened(Some(&l), &sized(4000, 4000)).is_some(), "16 MP");
        assert!(app.flattened(Some(&l), &sized(6000, 4000)).is_none(), "24 MP");
    }

    #[test]
    fn markup_stays_at_screen_resolution_until_a_change_ends() {
        let (mut app, _) = app_zoomed_in();
        let l = with_line(&mut app);
        let doc = app.doc().unwrap();
        // A small image too: rendering every frame of a drag lags at 1600×900.
        app.change_base = Some(doc.clone());
        assert!(app.flattened(Some(&l), &doc).is_none(), "tracing");
        app.change_base = None;
        assert!(app.flattened(Some(&l), &doc).is_some(), "released");
        app.adjusting = Some(doc.clone());
        assert!(app.flattened(Some(&l), &doc).is_none(), "Adjust Color open");
        app.adjusting = None;
        // Typing, even once something else has taken `change_base`.
        app.editing_text = Some(0);
        assert!(app.flattened(Some(&l), &doc).is_none(), "typing");
        app.editing_text = None;
        assert!(app.flattened(Some(&l), &doc).is_some(), "text finished");
    }

    #[test]
    fn the_texture_follows_flattening_with_the_elements_still_editable() {
        let (mut app, _) = app_zoomed_in();
        let l = with_line(&mut app);
        let ctx = egui::Context::default();
        ctx.set_fonts(markup::font_definitions());
        let draw = |app: &mut App| {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
            let mut output = ctx.run_ui(input, |ui| {
                let doc = app.doc();
                app.draw(ui, Some(&l), doc, None, ui.available_rect_before_wrap());
            });
            output.textures_delta.clear();
            app.tex_key.unwrap()
        };
        assert!(draw(&mut app).flat);
        assert_eq!(app.doc().unwrap().elements.len(), 1, "flattening only changes the view");
        // Without flattening it's drawn on top again, from a new texture.
        app.adjusting = app.doc();
        assert!(!draw(&mut app).flat);
    }

    #[test]
    fn curves_panel_runs_on_a_real_app() {
        let (mut app, _) = app_with_image();
        app.adjusting = app.doc();
        app.curve_panel = Some(CurvePanel::default());
        let ctx = egui::Context::default();
        ctx.set_fonts(markup::font_definitions());
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.adjust_panel(ui.ctx()));
            output.textures_delta.clear();
        }
        assert!(app.curve_panel.is_some() && app.adjusting.is_some(), "still open");
        assert_eq!(app.doc().unwrap().adjust, edit::Adjust::default(), "just opening it changes nothing");
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

    /// One frame of `handle_keys` with `key` pressed.
    fn press_key(app: &mut App, key: Key) {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
        let mut output = ctx.run_ui(input, |ui| app.handle_keys(ui.ctx(), false, ui.input(|i| i.key_pressed(Key::Escape)), false));
        output.textures_delta.clear();
    }

    /// A 1000 px image, black on the left half and white on the right, with the corners on the white part.
    fn app_with_corners() -> (App, Arc<Loaded>) {
        let (mut app, path) = app_with_image();
        let pixels = (0..1000 * 1000 * 3).map(|i| if (i / 3) % 1000 < 500 { 0 } else { 255 }).collect();
        let frame = Frame { w: 1000, h: 1000, pixels, alpha: false, delay: std::time::Duration::ZERO };
        let l = Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None });
        app.cache.insert(path, l.clone());
        app.settings.gpu_scaling = false;
        app.run_actions(&[Action::Perspective], &egui::Context::default(), false);
        assert!(app.tool == Tool::Perspective);
        app.perspective = Some([Pos2::new(600.0, 100.0), Pos2::new(900.0, 100.0), Pos2::new(900.0, 900.0), Pos2::new(600.0, 900.0)]);
        (app, l)
    }

    /// Draws one frame and gives the darkest and brightest pixels the texture was made from.
    fn drawn_range(app: &mut App, l: &Loaded) -> (u8, u8) {
        let ctx = egui::Context::default();
        ctx.set_fonts(markup::font_definitions());
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
        let mut output = ctx.run_ui(input, |ui| {
            let doc = app.doc();
            app.draw(ui, Some(l), doc, None, ui.available_rect_before_wrap());
        });
        output.textures_delta.clear();
        let pixels = &app.resized.as_ref().unwrap().1;
        (*pixels.iter().min().unwrap(), *pixels.iter().max().unwrap())
    }

    #[test]
    fn perspective_shows_undoes_and_redoes() {
        let (mut app, l) = app_with_corners();
        assert_eq!(drawn_range(&mut app, &l), (0, 255));
        press_key(&mut app, Key::Enter);
        assert!(app.tool == Tool::Pan && app.perspective.is_none());
        assert_eq!(app.doc().unwrap().size, [300, 800]);
        assert!(app.dirty());
        assert_eq!(drawn_range(&mut app, &l), (255, 255), "only the white part is left");
        app.undo_redo(true);
        assert_eq!(app.doc().unwrap().size, [1000, 1000]);
        assert_eq!(drawn_range(&mut app, &l), (0, 255));
        assert!(!app.dirty());
        app.undo_redo(false);
        assert_eq!(drawn_range(&mut app, &l), (255, 255));
    }

    #[test]
    fn perspective_of_the_same_size_redraws() {
        // Nothing in the texture key but the generation can tell these pixels apart.
        let (mut app, l) = app_with_corners();
        app.perspective = Some([Pos2::new(0.0, 1000.0), Pos2::new(1000.0, 1000.0), Pos2::new(1000.0, 0.0), Pos2::ZERO]);
        drawn_range(&mut app, &l);
        let (key, resized) = (app.tex_key, app.resized.as_ref().unwrap().1.clone());
        press_key(&mut app, Key::Enter);
        drawn_range(&mut app, &l);
        assert!(app.tex_key != key);
        assert_eq!(app.doc().unwrap().size, [1000, 1000]);
        assert_eq!(app.resized.as_ref().unwrap().1, resized, "flipped top to bottom, the columns are the same");
    }

    #[test]
    fn perspective_asks_before_flattening_markup() {
        let (mut app, _) = app_with_corners();
        with_line(&mut app);
        press_key(&mut app, Key::Enter);
        assert!(app.perspective_prompt);
        assert_eq!(app.doc().unwrap().elements.len(), 1, "nothing is flattened yet");
        // Enter and Esc belong to the dialog while it's open.
        press_key(&mut app, Key::Enter);
        press_key(&mut app, Key::Escape);
        assert!(app.tool == Tool::Perspective && app.perspective.is_some());
        app.apply_perspective();
        assert_eq!(app.doc().unwrap().size, [300, 800]);
        assert!(app.doc().unwrap().elements.is_empty());
    }

    #[test]
    fn perspective_refuses_crossed_corners_and_keeps_the_tool() {
        let (mut app, _) = app_with_corners();
        app.perspective = app.perspective.map(|[a, b, c, d]| [a, c, b, d]);
        press_key(&mut app, Key::Enter);
        assert!(app.tool == Tool::Perspective && app.doc.is_none() && app.status.is_some());
    }

    #[test]
    fn perspective_corners_follow_the_drag() {
        let (mut app, _) = app_zoomed_in();
        app.tool = Tool::Perspective;
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0));
        let (dest, vis) = crate::view::layout(app.view, Vec2::splat(1000.0), viewport, 1.0);
        let at = |p: f32| crate::view::to_image(dest, vis, Pos2::new(p, p));
        let far = Pos2::new(999.0, 999.0);
        app.perspective = Some([at(103.0), far, far, far]);
        feed(&mut app, drag_events(LEFT));
        let moved = app.perspective.unwrap();
        assert!(moved[0].distance(at(190.0)) < 1.0, "{:?}", moved[0]);
        assert_eq!(moved[1], far);
        assert!(app.perspective_drag.is_none());
        // A press away from every corner moves none.
        app.perspective = Some([at(150.0), far, far, far]);
        feed(&mut app, drag_events(LEFT));
        assert_eq!(app.perspective.unwrap()[0], at(150.0));
    }

    /// A temporary folder holding `files`, empty ones. Removed by hand at the end of a test.
    fn temp_folder(name: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lightview-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for file in files {
            std::fs::write(dir.join(file), b"").unwrap();
        }
        std::fs::canonicalize(dir).unwrap()
    }

    /// One frame of `ui` with `input`, with the new events it took.
    fn frame(app: &mut App, ctx: &egui::Context, input: egui::RawInput) -> egui::FullOutput {
        use eframe::App as _;
        let mut eframe = eframe::Frame::_new_kittest();
        let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut eframe));
        output.textures_delta.clear();
        output
    }

    #[test]
    fn dropped_image_opens_and_edits_ask_first() {
        let dir = temp_folder("drop-image", &["b.png", "notes.txt"]);
        let (mut app, _) = app_with_image();
        app.open_dropped(vec![dir.join("notes.txt")]);
        assert_eq!(app.files, [PathBuf::from("/a.png")], "not an image: stays where it is");
        assert!(app.status.is_some());
        app.edit(Doc::rotate_left);
        app.open_dropped(vec![dir.join("b.png")]);
        assert!(matches!(app.pending, Some(Pending::Open(_))), "unsaved edits ask first");
        app.pending = None;
        app.doc = None;
        app.open_dropped(vec![dir.join("b.png"), dir.join("notes.txt")]);
        assert_eq!(app.files, [dir.join("b.png")]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dropped_folder_opens_its_first_image() {
        let dir = temp_folder("drop-folder", &["notes.txt", "img10.png", "img2.png"]);
        let (mut app, _) = app_with_image();
        app.open_dropped(vec![dir.clone()]);
        assert!(matches!(app.rx.recv().unwrap(), Msg::Open(p) if p == dir.join("img2.png")));
        let empty = temp_folder("drop-empty", &["notes.txt"]);
        app.open_dropped(vec![empty.clone()]);
        assert!(matches!(app.rx.recv().unwrap(), Msg::Status(_)));
        assert_eq!(app.files, [PathBuf::from("/a.png")]);
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(empty).unwrap();
    }

    #[test]
    fn a_change_on_disk_reloads_or_asks() {
        let stamp = (std::time::SystemTime::UNIX_EPOCH, 9);
        let changed = |path: &PathBuf| Msg::Changed(path.clone(), stamp);
        let shown = |app: &App, path: &PathBuf| app.cache.get(path).unwrap().file_size;
        let (mut app, path) = app_with_image();
        let view = View::Manual { scale: 2.0, center: Pos2::new(1.0, 1.0) };
        app.view = view;
        app.handle(changed(&"/other.png".into()));
        app.handle(Msg::Changed(path.clone(), stamp));
        assert_eq!(app.view, view, "the zoom stays");
        assert_eq!(shown(&app, &path), 6, "the old pixels stay up until the new ones arrive");
        // Lightview's own write.
        app.own_stamp = Some(stamp);
        app.edit(Doc::rotate_left);
        app.handle(changed(&path));
        assert!(!app.changed_prompt && app.dirty());
        app.own_stamp = None;
        // Someone else's write, with edits: they stay until the user answers.
        app.handle(changed(&path));
        assert!(app.changed_prompt && app.dirty());
        let ctx = egui::Context::default();
        ctx.set_fonts(markup::font_definitions());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.changed_dialog(ui.ctx()));
        output.textures_delta.clear();
        assert!(app.changed_prompt && app.dirty(), "it waits");
        app.answer_changed(false);
        assert!(!app.changed_prompt && app.dirty(), "Keep Edits");
        app.handle(changed(&path));
        app.answer_changed(true);
        assert!(!app.changed_prompt && !app.dirty() && app.doc.is_none(), "Reload");
    }

    /// A real file changing under a real watcher and decoder, with no edits and then with some.
    #[test]
    fn an_outside_write_reaches_the_screen() {
        let dir = temp_folder("reload", &[]);
        let path = dir.join("a.png");
        image::RgbImage::new(4, 2).save(&path).unwrap();
        let (tx, rx) = mpsc::channel();
        let notify = Notify { tx, ctx: Arc::default() };
        let loader = Loader::new(notify.clone(), 1, [100, 100]);
        let settings = Settings { default_asked: true, ..Settings::default() };
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = App::new(&cc, Instant::now(), false, false, rx, loader, notify, settings, None);
        app.open(path.clone());
        *app.watched.lock().unwrap() = Some(path.clone());
        let wait_for = |app: &mut App, done: &dyn Fn(&App) -> bool| {
            while !done(app) {
                let msg = app.rx.recv_timeout(std::time::Duration::from_secs(10)).expect("nothing arrived");
                app.handle(msg);
            }
        };
        let width = |app: &App| app.cache.get(&path).map(|l| l.frames[0].w);
        wait_for(&mut app, &|app| width(app) == Some(4));
        std::thread::sleep(std::time::Duration::from_millis(1100));
        image::RgbImage::new(6, 2).save(&path).unwrap();
        wait_for(&mut app, &|app| width(app) == Some(6));
        app.edit(Doc::rotate_left);
        image::RgbImage::new(8, 2).save(&path).unwrap();
        wait_for(&mut app, &|app| app.changed_prompt);
        assert_eq!(width(&app), Some(6), "the edits' image stays until the user answers");
        app.answer_changed(true);
        wait_for(&mut app, &|app| width(app) == Some(8));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn navigating_away_drops_the_question() {
        let mut app = app_in_folder();
        app.changed_prompt = true;
        app.navigate(1);
        assert!(!app.changed_prompt);
    }

    #[test]
    fn save_copy_needs_unsaved_edits_and_writes_a_new_file() {
        let dir = temp_folder("save-copy", &[]);
        let path = dir.join("a.png");
        std::fs::write(&path, b"original").unwrap();
        let (mut app, old) = app_with_image();
        let l = app.cache.remove(&old).unwrap();
        app.cache.insert(path.clone(), l);
        app.files = vec![path.clone()];
        app.run_actions(&[Action::SaveCopy], &egui::Context::default(), false);
        assert!(app.rx.try_recv().is_err(), "no edits, nothing to write");
        app.edit(Doc::rotate_left);
        app.run_actions(&[Action::SaveCopy], &egui::Context::default(), false);
        let msg = app.rx.recv().unwrap();
        app.handle(msg);
        assert!(!app.dirty());
        assert!(image::open(dir.join("a-edited.png")).is_ok());
        assert_eq!(std::fs::read(path).unwrap(), b"original");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `app_with_image` on a folder of three images, showing the middle one.
    fn app_in_folder() -> App {
        let (mut app, _) = app_with_image();
        app.files = ["/a.png", "/b.png", "/c.png"].map(PathBuf::from).to_vec();
        app.index = 1;
        let frame = Frame { w: 2, h: 1, pixels: vec![0; 6], alpha: false, delay: std::time::Duration::ZERO };
        app.cache.insert("/b.png".into(), Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None }));
        app
    }

    #[test]
    fn home_and_end_go_to_the_ends_and_stay_put_there() {
        let ctx = egui::Context::default();
        let mut app = app_in_folder();
        app.run_actions(&[Action::LastImage], &ctx, false);
        assert_eq!(app.index, 2);
        app.run_actions(&[Action::FirstImage], &ctx, false);
        assert_eq!(app.index, 0);
        app.index = 1;
        app.edit(Doc::rotate_left);
        app.run_actions(&[Action::FirstImage], &ctx, false);
        assert!(matches!(app.pending, Some(Pending::Navigate(-1))), "unsaved edits ask first");
        app.pending = None;
        // Already there: edits survive, with no question.
        app.files.truncate(2);
        app.run_actions(&[Action::LastImage], &ctx, false);
        assert!(app.pending.is_none() && app.doc.is_some() && app.index == 1);
    }

    #[test]
    fn mouse_side_buttons_navigate_unless_a_dialog_is_open() {
        fn actions(button: egui::PointerButton, modal: bool) -> Vec<Action> {
            let ctx = egui::Context::default();
            ctx.set_fonts(markup::font_definitions());
            let mut found = Vec::new();
            for event in [egui::Event::PointerMoved(Pos2::new(9.0, 9.0)), press(button, true, 9.0)] {
                let mut input = egui::RawInput::default();
                input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
                input.events.push(event);
                let mut output = ctx.run_ui(input, |ui| {
                    found = App::read_shortcuts(ui.ctx(), false);
                    if modal {
                        egui::Modal::new(egui::Id::unique("dialog")).show(ui.ctx(), |ui| ui.label("dialog"));
                    }
                });
                output.textures_delta.clear();
            }
            found
        }
        assert!(actions(egui::PointerButton::Extra1, false) == [Action::PrevImage]);
        assert!(actions(egui::PointerButton::Extra2, false) == [Action::NextImage]);
        assert!(actions(egui::PointerButton::Extra1, true).is_empty());
        assert!(actions(LEFT, false).is_empty());
    }

    #[test]
    fn copy_path_puts_the_path_on_the_clipboard() {
        let (mut app, _) = app_with_image();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.run_actions(&[Action::CopyPath], ui.ctx(), false));
        output.textures_delta.clear();
        assert!(output.platform_output.commands.contains(&egui::OutputCommand::CopyText("/a.png".into())));
    }

    #[test]
    fn renaming_follows_the_file_and_keeps_the_edits() {
        let dir = temp_folder("rename-app", &["a.png", "taken.png"]);
        let path = dir.join("a.png");
        let (mut app, old) = app_with_image();
        let l = app.cache.remove(&old).unwrap();
        app.cache.insert(path.clone(), l);
        app.files = vec![path.clone()];
        app.recent = vec![path.clone()];
        app.edit(Doc::rotate_left);
        assert!(app.rename_current("taken.png").is_err());
        assert_eq!(app.files, [path.clone()]);
        app.rename_current("b.png").unwrap();
        let new = dir.join("b.png");
        assert_eq!((app.current(), app.recent.as_slice()), (Some(&new), std::slice::from_ref(&new)));
        assert!(app.cache.contains_key(&new) && !app.cache.contains_key(&path));
        assert!(app.dirty(), "edits stay");
        assert!(matches!(app.rx.recv().unwrap(), Msg::Folder(files) if files.contains(&new)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_rename_dialog_replaces_the_name_keeps_the_extension_and_shows_refusals() {
        let dir = temp_folder("rename-dialog", &["a.b.png", "taken.png"]);
        let (mut app, old) = app_with_image();
        let l = app.cache.remove(&old).unwrap();
        app.cache.insert(dir.join("a.b.png"), l);
        app.files = vec![dir.join("a.b.png")];
        let ctx = test_ctx();
        let run = |app: &mut App, events: Vec<egui::Event>| {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
            input.events = events;
            let mut output = ctx.run_ui(input, |ui| app.rename_dialog(ui.ctx()));
            output.textures_delta.clear();
        };
        let enter = || egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        app.run_actions(&[Action::Rename], &ctx, false);
        run(&mut app, vec![]);
        run(&mut app, vec![]);
        run(&mut app, vec![egui::Event::Text("taken".into())]);
        assert_eq!(app.renaming.as_ref().unwrap().name, "taken.png", "the extension wasn't selected");
        run(&mut app, vec![enter()]);
        assert!(app.renaming.as_ref().is_some_and(|r| r.error.is_some()), "refused, and still open");
        assert!(dir.join("a.b.png").exists());
        run(&mut app, vec![]);
        run(&mut app, vec![egui::Event::Text("free".into())]);
        run(&mut app, vec![enter()]);
        assert!(app.renaming.is_none());
        assert_eq!(app.current(), Some(&dir.join("free.png")));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn f2_is_refused_while_an_ai_job_needs_the_path() {
        let (mut app, _) = app_with_image();
        app.restoring = Some(Restoring { id: 1, task: crate::ai::Task::Denoise, progress: 0.0, started: Instant::now(), cancel: Arc::default(), selection: None });
        app.run_actions(&[Action::Rename], &egui::Context::default(), false);
        assert!(app.renaming.is_none());
        app.restoring = None;
        app.run_actions(&[Action::Rename], &egui::Context::default(), false);
        assert!(app.renaming.is_some());
    }

    /// One frame of input, drawing and the navigator, in a 400 px viewport.
    fn view_frame(app: &mut App, ctx: &egui::Context, l: &Loaded, events: Vec<egui::Event>) -> egui::FullOutput {
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
        input.events = events;
        let mut output = ctx.run_ui(input, |ui| {
            let doc = app.doc();
            let viewport = ui.available_rect_before_wrap();
            app.image_input(ui, doc.as_ref().unwrap(), viewport, 1.0);
            app.draw(ui, Some(l), doc, None, viewport);
        });
        output.textures_delta.clear();
        output
    }

    fn click_at(app: &mut App, ctx: &egui::Context, l: &Loaded, p: Pos2) -> Vec<egui::OutputCommand> {
        let button = |pressed| egui::Event::PointerButton { pos: p, button: LEFT, pressed, modifiers: egui::Modifiers::NONE };
        let mut commands = Vec::new();
        for events in [vec![egui::Event::PointerMoved(p)], vec![], vec![button(true)], vec![button(false)], vec![]] {
            commands.extend(view_frame(app, ctx, l, events).platform_output.commands);
        }
        commands
    }

    fn test_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_fonts(markup::font_definitions());
        ctx
    }

    #[test]
    fn the_navigator_shows_only_when_zoomed_in_and_isnt_rebuilt_by_panning() {
        let (mut app, _) = app_zoomed_in();
        let (ctx, l) = (test_ctx(), app.cache.values().next().unwrap().clone());
        app.settings.gpu_scaling = false;
        app.view = View::Fit;
        view_frame(&mut app, &ctx, &l, vec![]);
        assert!(app.navigator.is_none());
        app.view = crate::view::actual_size(Vec2::splat(1000.0));
        view_frame(&mut app, &ctx, &l, vec![]);
        let first = app.navigator.as_ref().unwrap().1.id();
        app.view = view::pan(app.view, Vec2::splat(50.0), Vec2::splat(1000.0), Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)), 1.0);
        view_frame(&mut app, &ctx, &l, vec![]);
        assert_eq!(app.navigator.as_ref().unwrap().1.id(), first);
        app.edit(Doc::rotate_right);
        app.view = crate::view::actual_size(Vec2::splat(1000.0));
        view_frame(&mut app, &ctx, &l, vec![]);
        assert_ne!(app.navigator.as_ref().unwrap().1.id(), first, "it follows the edits");
    }

    #[test]
    fn clicking_the_navigator_moves_the_view_and_not_the_tool() {
        let (mut app, _) = app_zoomed_in();
        let (ctx, l) = (test_ctx(), app.cache.values().next().unwrap().clone());
        app.tool = Tool::Draw(Shape::Rect);
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0));
        let nav = view::navigator_rect(Vec2::splat(1000.0), viewport);
        click_at(&mut app, &ctx, &l, nav.right_bottom() - Vec2::splat(10.0));
        let (_, crop) = view::layout(app.view, Vec2::splat(1000.0), viewport, 1.0);
        assert!(crop.max.x > 999.0 && crop.max.y > 999.0, "{crop:?}");
        assert!(app.doc().unwrap().elements.is_empty());
    }

    #[test]
    fn the_navigator_follows_a_rotated_image() {
        let (mut app, path) = app_with_image();
        let frame = Frame { w: 1000, h: 500, pixels: vec![0; 1_500_000], alpha: false, delay: std::time::Duration::ZERO };
        let l = Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None });
        app.cache.insert(path, l.clone());
        app.edit(Doc::rotate_right);
        let (size, viewport) = (Vec2::new(500.0, 1000.0), Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
        app.view = crate::view::actual_size(size);
        let ctx = test_ctx();
        let nav = view::navigator_rect(size, viewport);
        assert!(nav.height() > nav.width());
        click_at(&mut app, &ctx, &l, nav.right_bottom() - Vec2::splat(2.0));
        let (_, crop) = view::layout(app.view, size, viewport, 1.0);
        assert!(crop.max.x > 499.0 && crop.max.y > 999.0, "{crop:?}");
    }

    /// `app_with_image` with a transparent pixel on the left and a red one on the right, picking.
    fn app_picking() -> (App, Arc<Loaded>) {
        let (mut app, path) = app_with_image();
        let frame = Frame { w: 2, h: 1, pixels: vec![0, 0, 255, 128, 255, 16, 0, 255], alpha: true, delay: std::time::Duration::ZERO };
        let l = Arc::new(Loaded { frames: vec![frame], file_size: 8, svg: None });
        app.cache.insert(path, l.clone());
        app.run_actions(&[Action::CopyColor], &egui::Context::default(), false);
        assert!(app.tool == Tool::CopyColor);
        (app, l)
    }

    #[test]
    fn copy_color_copies_the_pixel_clicked() {
        let (mut app, l) = app_picking();
        let ctx = test_ctx();
        let copied = |text: &str| egui::OutputCommand::CopyText(text.into());
        assert!(click_at(&mut app, &ctx, &l, Pos2::new(300.0, 200.0)).contains(&copied("#FF1000")));
        assert!(click_at(&mut app, &ctx, &l, Pos2::new(100.0, 200.0)).contains(&copied("#0000FF80")), "not opaque");
        // What's seen, not the file: here flipped.
        app.edit(Doc::flip_horizontal);
        assert!(click_at(&mut app, &ctx, &l, Pos2::new(100.0, 200.0)).contains(&copied("#FF1000")));
        press_key(&mut app, Key::Escape);
        assert!(app.tool == Tool::Pan);
    }

    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("unused".into())
        }
    }

    #[test]
    fn a_dropped_file_reaches_the_app_through_egui() {
        let dir = temp_folder("drop-egui", &["b.png"]);
        let (mut app, _) = app_with_image();
        let ctx = egui::Context::default();
        ctx.set_fonts(markup::font_definitions());
        let mut input = egui::RawInput::default();
        input.dropped_files.push(Arc::new(Dropped(dir.join("b.png"))));
        frame(&mut app, &ctx, input);
        assert_eq!(app.files, [dir.join("b.png")]);
        std::fs::remove_dir_all(dir).unwrap();
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
