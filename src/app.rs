use crate::curves::CurvePanel;
use crate::i18n::{tr, trf};
use crate::edit::{self, Doc, ResizeDialog};
use crate::exif_edit;
use crate::inspector::{self, Inspector};
use crate::files;
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum SelKind {
    Rect,
    Ellipse,
    Lasso,
}

/// What dragging on the image does.
#[derive(Clone, Copy, Default, PartialEq)]
enum Tool {
    #[default]
    Pan,
    /// The crop rect so far, in image px.
    Crop(Option<Rect>),
    /// The corners (output px, clockwise from the top left) and which one is being dragged.
    Perspective { corners: [Pos2; 4], drag: Option<usize> },
    /// Turning the image by this many degrees, clockwise.
    Straighten(f32),
    /// The two endpoints (image px) and which one is being dragged.
    Measure { ends: Option<[Pos2; 2]>, drag: usize },
    CopyColor,
    Select,
    Draw(Shape),
    Selection(SelKind),
}

/// What dragging with the Select tool does.
#[derive(Clone, Copy, Default, PartialEq)]
enum Grab {
    #[default]
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

/// What belongs to the image on screen: all of it starts afresh when another image is shown.
#[derive(Default)]
struct Shown {
    preview: Option<Arc<Frame>>,
    anim_frame: usize,
    anim_due: Option<Instant>,
    anim_paused: bool,
    /// Edits to the current image (None: none yet), with undo/redo history.
    doc: Option<Doc>,
    undo: Vec<Doc>,
    redo: Vec<Doc>,
    /// Asking what to do: the file changed on disk, and there are unsaved edits.
    changed_prompt: bool,
    /// The edits last saved to a copy.
    saved: Option<Doc>,
    tool: Tool,
    /// Asking before flattening the markup into the corrected image.
    perspective_prompt: bool,
    /// The style for new markup (None until the toolbar is first used on this image).
    style: Option<Style>,
    selected: Option<usize>,
    grab: Grab,
    /// The text element being typed into.
    editing_text: Option<usize>,
    /// The edits before the markup change in progress (a drag or typing), for one undo step.
    change_base: Option<Doc>,
    /// The element whose style the last undo step changed, so style tweaks merge into one step.
    last_restyle: Option<usize>,
    /// The selection, in output px.
    selection: Option<Selection>,
    /// Textures for Erase and Image elements, by `Element::texture_key`.
    element_textures: HashMap<u64, TextureHandle>,
    /// While Adjust Color or Adjust Curves is open: the edits from before it opened.
    adjusting: Option<Doc>,
    /// Some while the open panel is Adjust Curves rather than Adjust Color.
    curve_panel: Option<CurvePanel>,
    compare: Option<Compare>,
}

pub struct App {
    /// The image on screen: its edits, tool, animation and dialogs.
    shown: Shown,
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
    /// Images moved to the Trash this session, newest last; Ctrl+Z brings them back.
    deleted: Vec<PathBuf>,
    delete_prompt: bool,
    renaming: Option<dialogs::Renaming>,
    /// The file the watcher thread keeps an eye on.
    watched: Arc<Mutex<Option<PathBuf>>>,
    /// The state lightview itself last left the current file in, which isn't a change to react to.
    own_stamp: Option<files::Stamp>,
    /// The navigator's thumbnail, and what it holds.
    navigator: Option<(TexKey, TextureHandle)>,
    /// Crop, Measure and selection tools: where the drag started, in image px.
    drag_start: Option<Pos2>,
    /// A drag started with the right button is under way: it pans.
    right_pan: bool,
    show_toolbar: bool,
    focus_text: bool,
    resize_dialog: Option<ResizeDialog>,
    pending: Option<Pending>,
    show_ai_models: bool,
    /// The download whose licences are being shown, and which one (they come one at a time).
    license_prompt: Option<(Download, usize)>,
    restoring: Option<Restoring>,
    /// How many Denoise / Enhance runs were started: the last run's id.
    restore_runs: u64,
    /// Enhance Image waiting for the user to read its model's notice: the model file.
    enhance_notice: Option<String>,
    /// Blur Background's dialog is open, with this strength (% of the long side).
    blur_strength: Option<f32>,
    /// Recognize Text or Read QR Code is running.
    reading: bool,
    /// What they read, and the dialog's title.
    found_text: Option<(&'static str, String)>,
    /// The Keyboard Shortcuts dialog (from Settings) is open.
    show_shortcuts: bool,
    /// That dialog is waiting for the key combination for this action.
    recording: Option<Action>,
    /// Why the last key combination wasn't taken.
    shortcut_error: Option<String>,
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
    /// Holds what Copy File put on the clipboard: on Linux it's gone once this is dropped.
    file_clipboard: Option<arboard::Clipboard>,
    /// The theme's colours for outlines and guides drawn over the image.
    canvas: crate::theme::Canvas,
    /// Why the chosen theme didn't load, shown in Settings.
    theme_error: Option<String>,
}

impl App {
    /// `start` is already being decoded (see `load::start` in main).
    #[allow(clippy::too_many_arguments)]
    pub fn new(cc: &eframe::CreationContext, t0: Instant, timings: bool, bench: bool, rx: mpsc::Receiver<Msg>, loader: Loader, notify: Notify, mut settings: Settings, start: Option<PathBuf>) -> Self {
        // Otherwise egui's own Ctrl +/− scales the whole UI.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        cc.egui_ctx.set_fonts(markup::font_definitions());
        settings::apply_font_size(&cc.egui_ctx, settings.font_size);
        crate::theme::seed_folder();
        let (canvas, theme_error) = crate::theme::apply(&cc.egui_ctx, &settings.theme);
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
        let watched = files::watch(notify.clone(), std::time::Duration::from_secs(1));
        App {
            shown: Shown::default(),
            t0, timings, bench, rx, loader, notify, recent, settings, show_info: false, show_settings: false, default_prompt, default_result: None, inspector: None,
            files: start.into_iter().collect(), index: 0,
            cache: HashMap::new(), error: None, view: View::Fit,
            texture: None, tex_key: None, resized: None, baked: None, sharp: None, sharp_wanted: 1, flat: None, generation: 0,
            deleted: Vec::new(), delete_prompt: false, renaming: None, watched, own_stamp: None, navigator: None, drag_start: None, right_pan: false,
            show_toolbar, focus_text: false,
            resize_dialog: None, pending: None, show_ai_models: false, license_prompt: None, restoring: None, restore_runs: 0, enhance_notice: None, blur_strength: None, reading: false, found_text: None, show_shortcuts: false, recording: None, shortcut_error: None, visible: None, downloading: HashSet::new(),
            title: String::new(),
            window_reported: false, preview_reported: None, full_reported: false, nav_start: None, close_next: false, status: None, file_clipboard: None, canvas, theme_error,
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        (self.canvas, self.theme_error) = crate::theme::apply(ctx, &self.settings.theme);
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
                        self.shown.compare = texture.map(|texture| Compare { task: run.task, region, texture, split: 0.5, ms_per_mp, scale });
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
                            self.shown.selection = None;
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
            Msg::TextFound(qr, result) => {
                self.reading = false;
                match result {
                    Ok(text) if text.trim().is_empty() => self.set_status(if qr { tr("No QR code found") } else { tr("No text found") }),
                    Ok(text) => {
                        self.status = None;
                        self.found_text = Some((if qr { tr("QR Code") } else { tr("Text in Image") }, text));
                    }
                    Err(e) => self.set_status(trf("Couldn't read the text: {e}", &[("e", &e)])),
                }
            }
            Msg::Cutout(path, result) => match result {
                Ok((img, object, backdrop)) if self.current() == Some(&path) => {
                    let Some(doc) = self.doc() else { return };
                    let bounds = doc.bounds();
                    let style = *self.shown.style.get_or_insert_with(|| Style::for_image(doc.size_vec()));
                    let [r, g, b] = self.settings.background;
                    // Cover everything, then put the cut-out back on top: one undo step.
                    self.edit(|d| {
                        match backdrop {
                            crate::ai::Backdrop::Clear => d.elements.push(Element::erase(&Selection::Rect(bounds), false, bounds, Color32::TRANSPARENT)),
                            crate::ai::Backdrop::Colour => d.elements.push(Element::erase(&Selection::Rect(bounds), false, bounds, Color32::from_rgb(r, g, b))),
                            crate::ai::Backdrop::Blurred(blurred) => d.elements.push(Element::pasted(Arc::new(blurred), Pos2::ZERO, bounds.size(), style)),
                        }
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
                    self.shown.changed_prompt = true;
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
                    self.shown.preview = Some(Arc::new(frame));
                    self.generation += 1;
                }
            }
            Msg::Full(path, loaded) => {
                if self.window().contains(&path) {
                    if self.current() == Some(&path) {
                        self.shown.preview = None;
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
                    self.shown.saved = Some(doc);
                }
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                self.set_status(match &result {
                    Ok(()) => trf("Saved {name}", &[("name", &name)]),
                    Err(e) => trf("Couldn't save {name}: {e}", &[("name", &name), ("e", &e)]),
                });
                let (Ok(()), Ok(path), Some(current)) = (result, std::fs::canonicalize(&path), self.current().cloned()) else { return };
                if path == current {
                    // Overwrote the image on screen: show the saved version.
                    self.own_stamp = files::stamp(&path);
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
        self.shown.doc.clone().or_else(|| self.base_doc())
    }

    /// Whether the current edits haven't been saved to a copy.
    fn dirty(&self) -> bool {
        self.shown.doc.is_some() && self.shown.doc != self.shown.saved.clone().or_else(|| self.base_doc())
    }

    /// Applies an edit to the current image as one undo step.
    fn edit(&mut self, f: impl FnOnce(&mut Doc)) {
        let Some(mut doc) = self.doc() else { return };
        let before = doc.clone();
        f(&mut doc);
        if doc != before {
            self.replace_doc(doc);
            self.shown.undo.push(before);
            self.shown.redo.clear();
            self.shown.last_restyle = None;
        }
    }

    /// Makes `doc` the current edits. Points in the old image's coordinates go when the geometry
    /// changes, and new `Doc.base` pixels need new textures, which `TexKey` can't see.
    fn replace_doc(&mut self, doc: Doc) {
        let current = self.doc();
        if current.as_ref().is_none_or(|d| d.geometry() != doc.geometry()) {
            (self.view, self.shown.selection) = (View::Fit, None);
            if let Tool::Measure { ends, .. } = &mut self.shown.tool {
                *ends = None;
            }
        }
        if current.is_some_and(|d| d.base != doc.base) {
            self.generation += 1;
        }
        self.shown.doc = Some(doc);
    }

    /// The current edits, to change in place; pair with `commit`.
    fn doc_mut(&mut self) -> Option<&mut Doc> {
        if self.shown.doc.is_none() {
            self.shown.doc = self.base_doc();
        }
        self.shown.doc.as_mut()
    }

    /// Ends an in-place change that started from `before`, as one undo step.
    fn commit(&mut self, before: Doc) {
        if self.shown.doc.as_ref() != Some(&before) {
            self.shown.undo.push(before);
            self.shown.redo.clear();
        }
        self.shown.last_restyle = None;
    }

    fn undo_redo(&mut self, undo: bool) {
        if undo && self.shown.undo.is_empty() && let Some(path) = self.deleted.pop() {
            match files::restore_trashed(&path) {
                Ok(()) => self.open(path),
                Err(e) => self.set_status(trf("Couldn't restore the image: {e}", &[("e", &e)])),
            }
            return;
        }
        let Some(current) = self.doc() else { return };
        let (from, to) = if undo { (&mut self.shown.undo, &mut self.shown.redo) } else { (&mut self.shown.redo, &mut self.shown.undo) };
        let Some(doc) = from.pop() else { return };
        (self.shown.selected, self.shown.editing_text, self.shown.change_base, self.shown.last_restyle) = (None, None, None, None);
        to.push(current);
        self.replace_doc(doc);
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
            std::thread::spawn(move || notify.send(match files::first_image(&path, sort) {
                Some(image) => Msg::Open(image),
                None => Msg::Status(trf("No images in {name}", &[("name", &name)])),
            }));
        } else if files::is_image(&path) {
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
        self.error = None;
        (self.sharp, self.sharp_wanted) = (None, 1);
        self.generation += 1;
        if let Some(r) = self.restoring.take() {
            r.cancel.store(true, Ordering::Relaxed);
        }
        self.shown = Shown::default();
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
        let (notify, keep_icc) = (self.notify.clone(), !self.settings.color_managed);
        std::thread::spawn(move || {
            let (dest, result) = match save::reserve_copy(&source) {
                Ok(dest) => {
                    let result = save::save(&frame, &doc, &source, &dest, keep_icc);
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
        let (notify, keep_icc) = (self.notify.clone(), !self.settings.color_managed);
        // The system dialog asks before replacing an existing file.
        std::thread::spawn(move || {
            if let Some(dest) = dialog.save_file() {
                let result = save::save(&frame, &doc, &source, &dest, keep_icc);
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
        let doc = self.shown.doc.clone();
        let ins = self.inspector.as_mut().unwrap();
        let save = egui::Panel::right("inspector").size_range(240.0..=320.0).default_size(320.0).resizable(true).show(ui, |ui| inspector::ui(ui, ins, l, doc.as_ref())).inner;
        let Some(d) = ins.exif.as_ref().ok().filter(|_| save) else { return };
        let stem = d.path.file_stem().unwrap_or_default().to_string_lossy();
        // The copy keeps the original's format: only its EXIF changes.
        let ext = d.path.extension().unwrap_or_default().to_string_lossy().to_lowercase();
        let mut file_dialog = rfd::FileDialog::new().set_file_name(format!("{stem}-edited.{ext}")).add_filter(ext.to_uppercase(), &[&ext]);
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
        let Ok(img) = edit::render(&l.frames[self.shown.anim_frame.min(l.frames.len() - 1)], &doc) else { return };
        let img = img.into_rgba8();
        let bounds = doc.bounds();
        let sel = self.shown.selection.clone().unwrap_or(Selection::Rect(bounds));
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
        let (Some(sel), Some(doc)) = (self.shown.selection.take(), self.doc()) else { return };
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
        let new = files::rename(&old, name)?;
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
        let (Some(sel), Some(doc)) = (self.shown.selection.take(), self.doc()) else { return };
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
        let corners = match self.shown.tool {
            Tool::Perspective { corners, .. } => Some(corners),
            Tool::Straighten(degrees) => self.doc().map(|doc| edit::straighten_corners(doc.size_vec(), degrees)),
            _ => None,
        };
        let (Some(corners), Some(l)) = (corners, self.current().and_then(|p| self.cache.get(p)).cloned()) else { return };
        let mut result = Ok(());
        // Straighten's corners lie outside the image: the new corners get the background colour.
        let fill = matches!(self.shown.tool, Tool::Straighten(_)).then_some(self.settings.background);
        self.edit(|d| result = d.perspective(&l.frames[0], corners, fill));
        match result {
            Ok(()) => self.shown.tool = Tool::Pan,
            Err(e) => self.set_status(e),
        }
    }

    /// Straighten: the turned image's bounding box goes through Correct Perspective, which asks
    /// before merging the markup and keeps colour live.
    fn apply_straighten(&mut self) {
        let Some(doc) = self.doc() else { return };
        if self.shown.tool == Tool::Straighten(0.0) {
            self.shown.tool = Tool::Pan;
            return;
        }
        if doc.elements.is_empty() {
            self.apply_perspective();
        } else {
            self.shown.perspective_prompt = true;
        }
    }

    /// Changes the selected pixels, as shown, with `f`, as a patch over them that Ctrl+Z removes.
    fn patch_selection(&mut self, f: fn(&mut image::RgbaImage), done: &str) {
        let Some((l, doc)) = self.current().and_then(|p| Some((self.cache.get(p)?.clone(), self.doc()?))) else { return };
        let Some(sel) = self.shown.selection.clone().filter(|s| !s.is_empty()) else { return };
        let region = Rect::from_min_max(sel.bbox().min.floor(), sel.bbox().max.ceil()).intersect(doc.bounds());
        if region.width() < 1.0 || region.height() < 1.0 {
            return;
        }
        let frame = &l.frames[self.shown.anim_frame.min(l.frames.len() - 1)];
        let mut patch = match edit::render(frame, &doc) {
            Ok(img) => image::imageops::crop_imm(&img.into_rgba8(), region.min.x as u32, region.min.y as u32, region.width() as u32, region.height() as u32).to_image(),
            Err(e) => return self.set_status(e),
        };
        f(&mut patch);
        clip_to(&mut patch, &sel, region.min);
        let style = Style::for_image(region.size());
        self.edit(|d| d.elements.push(Element::pasted(Arc::new(patch), region.min, region.size(), style)));
        self.shown.selection = None;
        self.set_status(done);
    }

    /// Adds a clipboard image (straight RGBA) as a movable element, centred and fitted inside the image.
    fn paste(&mut self, w: u32, h: u32, pixels: Vec<u8>) {
        let (Some(img), Some(doc)) = (image::RgbaImage::from_raw(w, h, pixels), self.doc()) else { return };
        let size = Vec2::new(w as f32, h as f32);
        let fit = (doc.size_vec() / size).min_elem().min(1.0);
        let at = self.shown.selection.as_ref().map_or(((doc.size_vec() - size * fit) / 2.0).to_pos2(), |s| s.bbox().min);
        let style = *self.shown.style.get_or_insert_with(|| Style::for_image(doc.size_vec()));
        self.edit(|d| d.elements.push(Element::pasted(Arc::new(img), at, size * fit, style)));
        self.finish_text();
        (self.shown.tool, self.show_toolbar, self.shown.selection) = (Tool::Select, true, None);
        self.shown.selected = self.shown.doc.as_ref().map(|d| d.elements.len() - 1);
    }

    fn make_title(&self, zoom: Option<f32>) -> String {
        let Some(path) = self.current() else { return "lightview".into() };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let zoom = zoom.map_or(String::new(), |z| format!(" — {z:.2}×"));
        match self.cache.get(path) {
            Some(l) => {
                let page = if l.paged() { format!(" — {}", trf("page {n} of {count}", &[("n", &(self.shown.anim_frame + 1)), ("count", &l.frames.len())])) } else { String::new() };
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
        let mut actions = if self.recording.is_some() {
            self.record_shortcut(&ctx);
            Vec::new()
        } else {
            App::read_shortcuts(&ctx, typing, &menu::shortcuts(&self.settings.shortcuts))
        };
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
mod tests;
