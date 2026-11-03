use super::{App, Tool};
use crate::curves::curves_ui;
use crate::edit;
use crate::i18n::{tr, trf};
use crate::load::{self, Msg};
use crate::settings;
use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{self, Rect};

/// A download waiting for the user to accept its licence.
#[derive(Clone, Copy)]
pub(super) enum Download {
    /// An index into `ai::CATALOG`.
    Model(usize),
    Runtime,
    /// ONNX Runtime's GPU build and NVIDIA's CUDA libraries.
    Gpu,
}

/// The Rename dialog's state.
pub(super) struct Renaming {
    original: String,
    pub(super) name: String,
    /// The dialog just opened, or a name was refused: the name without its extension gets selected.
    fresh: bool,
    pub(super) error: Option<String>,
}

impl Renaming {
    pub(super) fn new(path: &std::path::Path) -> Self {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        Renaming { original: name.clone(), name, fresh: true, error: None }
    }
}

impl App {
    /// Whichever dialogs and floating bars are open.
    pub(super) fn dialogs(&mut self, ctx: &egui::Context, viewport: Rect) {
        self.settings_dialog(ctx);
        self.shortcuts_dialog(ctx);
        self.default_dialog(ctx);
        self.resize_dialog(ctx);
        self.ai_models_dialog(ctx);
        self.license_dialog(ctx);
        self.enhance_notice_dialog(ctx);
        self.text_dialog(ctx);
        self.restore_progress(ctx, viewport);
        self.compare_bar(ctx, viewport);
        self.adjust_panel(ctx);
        self.straighten_panel(ctx);
        self.discard_dialog(ctx);
        self.delete_dialog(ctx);
        self.rename_dialog(ctx);
        self.changed_dialog(ctx);
        self.perspective_dialog(ctx);
    }

    pub(super) fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let (sort, color_managed) = (self.settings.sort, self.settings.color_managed);
        let modal = egui::Modal::new(egui::Id::unique("settings")).show(ctx, |ui| {
            ui.heading(tr("Settings"));
            settings::ui(ui, &mut self.settings);
            ui.add_space(8.0);
            if ui.button(tr("Change Keyboard Shortcuts…")).clicked() {
                self.show_shortcuts = true;
            }
            ui.add_space(8.0);
            if settings::desktop_installed() {
                if ui.button(tr("Make lightview the default image viewer")).clicked() {
                    self.default_result = Some(settings::make_default());
                }
                if let Some(result) = &self.default_result {
                    ui.weak(match result {
                        Ok(()) => tr("lightview is now the default image viewer"),
                        Err(e) => e,
                    });
                }
                ui.add_space(8.0);
            }
            ui.weak(format!("lightview {}", env!("CARGO_PKG_VERSION")));
            if ui.button(tr("Check for updates")).clicked() {
                crate::platform::open("https://github.com/pyubero/lightview/releases");
            }
            ui.add_space(8.0);
            ui.button(tr("Close")).clicked()
        });
        if modal.inner || modal.should_close() {
            self.show_settings = false;
        }
        if self.settings.sort != sort && let Some(path) = self.current() {
            load::relist(path, self.settings.sort, &self.notify);
        }
        if self.settings.color_managed != color_managed {
            self.apply_color_setting();
        }
    }

    /// Keyboard Shortcuts, over Settings: every menu action by menu, its keys, Change and Remove.
    pub(super) fn shortcuts_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let height = ctx.content_rect().height() * 0.6;
        let modal = egui::Modal::new(egui::Id::unique("shortcuts")).show(ctx, |ui| {
            ui.heading(tr("Keyboard Shortcuts"));
            if let Some(e) = &self.shortcut_error {
                ui.colored_label(ui.visuals().warn_fg_color, e);
            }
            let keys = super::menu::shortcuts(&self.settings.shortcuts);
            egui::ScrollArea::vertical().max_height(height).show(ui, |ui| {
                for (menu, items) in super::menu::bindable_menus() {
                    ui.add_space(6.0);
                    ui.strong(tr(menu));
                    egui::Grid::new(("shortcuts", menu)).num_columns(3).striped(true).min_col_width(120.0).show(ui, |ui| {
                        for (action, name) in items {
                            ui.label(tr(name));
                            let current: Vec<String> = keys.iter().filter(|(_, a)| *a == action).map(|(sc, _)| ui.ctx().format_shortcut(sc)).collect();
                            if self.recording.as_ref() == Some(&action) {
                                ui.strong(tr("Press keys… (Esc to cancel)"));
                            } else {
                                ui.label(current.join(", "));
                            }
                            ui.horizontal(|ui| {
                                if ui.small_button(tr("Change")).clicked() {
                                    (self.recording, self.shortcut_error) = (Some(action.clone()), None);
                                }
                                if !current.is_empty() && ui.small_button(tr("Remove")).clicked() {
                                    self.set_shortcut(&action, None);
                                }
                            });
                            ui.end_row();
                        }
                    });
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Reset All Shortcuts")).clicked() {
                    (self.settings.shortcuts, self.recording, self.shortcut_error) = (Vec::new(), None, None);
                }
                ui.button(tr("Close")).clicked()
            })
            .inner
        });
        if modal.inner || modal.should_close() {
            (self.show_shortcuts, self.recording, self.shortcut_error) = (false, None, None);
        }
    }

    /// Decodes what's held again, with the new setting. Edits stay.
    // shortcut: pixels from Correct Perspective (`Doc.base`) keep the old colours until it's redone.
    pub(super) fn apply_color_setting(&mut self) {
        self.loader.set_color_managed(self.settings.color_managed);
        self.cache.clear();
        self.generation += 1;
        self.request_decodes();
    }

    /// Asked once, here and not by the package: a package runs as root, defaults are per user.
    pub(super) fn default_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut tick) = self.default_prompt else { return };
        let modal = egui::Modal::new(egui::Id::unique("default-viewer")).show(ctx, |ui| {
            ui.heading(tr("Welcome to lightview"));
            ui.add_space(8.0);
            ui.checkbox(&mut tick, tr("Make lightview my default image viewer"));
            ui.add_space(8.0);
            ui.button(tr("OK")).clicked()
        });
        self.default_prompt = Some(tick);
        if modal.inner || modal.should_close() {
            self.default_prompt = None;
            self.settings.default_asked = true;
            if modal.inner && tick {
                let result = settings::make_default();
                // Settings is where the outcome is shown.
                self.show_settings = result.is_err();
                self.default_result = Some(result);
            }
        }
    }

    /// Auto Levels for the visible part of the current image.
    pub(super) fn auto_levels(&self) -> Option<[f32; 2]> {
        let doc = self.doc()?;
        Some(crate::retouch::auto_levels(doc.source(&self.cache.get(self.current()?)?.frames[0]), doc.crop))
    }

    /// The Adjust Color or Adjust Curves panel: it changes the edits live, and Done makes them one undo step.
    pub(super) fn adjust_panel(&mut self, ctx: &egui::Context) {
        let (Some(before), Some(mut doc)) = (self.shown.adjusting.clone(), self.doc()) else { return };
        let mut open = true;
        let mut curves = self.shown.curve_panel.take();
        let base = doc.base.clone();
        let frame = base.as_ref().map(|b| &*b.0).or(self.current().and_then(|c| self.cache.get(c)).map(|l| &l.frames[0]));
        let title = if curves.is_some() { tr("Adjust Curves") } else { tr("Adjust Color") };
        let choice = egui::Window::new(title).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| match &mut curves {
            Some(panel) => curves_ui(ui, &mut doc.adjust, panel, frame, doc.crop),
            None => edit::adjust_ui(ui, &mut doc.adjust, || self.auto_levels()),
        }).and_then(|r| r.inner).flatten();
        let mut adjust = doc.adjust;
        self.shown.doc = Some(doc);
        self.shown.curve_panel = curves;
        if choice.is_some() || !open {
            // Back to before the panel opened, then Done applies the result as one edit.
            self.shown.adjusting = None;
            self.shown.curve_panel = None;
            self.shown.doc = Some(before);
            adjust.curves.keep_active();
            if choice == Some(true) {
                self.edit(|d| d.adjust = adjust);
            }
        }
    }

    /// Straighten's angle. The canvas shows the image turned and its new bounds; Done or Enter applies it.
    pub(super) fn straighten_panel(&mut self, ctx: &egui::Context) {
        if self.shown.perspective_prompt {
            return;
        }
        let Tool::Straighten(degrees) = &mut self.shown.tool else { return };
        let mut open = true;
        let choice = egui::Window::new(tr("Straighten")).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.add(egui::Slider::new(degrees, -180.0..=180.0).step_by(0.1).fixed_decimals(1).suffix("°"));
            ui.add_space(6.0);
            edit::done_cancel(ui)
        }).and_then(|r| r.inner).flatten();
        match choice {
            Some(true) => self.apply_straighten(),
            Some(false) => self.shown.tool = Tool::Pan,
            None if !open => self.shown.tool = Tool::Pan,
            None => {}
        }
    }

    /// What Recognize Text or Read QR Code read. Editable, so part of it can be selected and
    /// copied; a QR code's link is only shown, never opened.
    fn text_dialog(&mut self, ctx: &egui::Context) {
        let Some((title, text)) = &mut self.found_text else { return };
        let modal = egui::Modal::new(egui::Id::unique("found-text")).show(ctx, |ui| {
            ui.heading(*title);
            egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| ui.add(egui::TextEdit::multiline(text).desired_width(480.0).desired_rows(6)));
            ui.add_space(6.0);
            ui.horizontal(|ui| (ui.button(tr("Copy")).clicked(), ui.button(tr("Close")).clicked())).inner
        });
        let (copy, close) = modal.inner;
        if copy {
            ctx.copy_text(text.clone());
        }
        if copy || close || modal.should_close() {
            self.found_text = None;
        }
    }

    pub(super) fn ai_models_dialog(&mut self, ctx: &egui::Context) {
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
                if let Some(src) = rfd::FileDialog::new().add_filter(tr("ONNX model"), &["onnx"]).pick_file() {
                    notify.send(Msg::Status(match crate::ai::add_model(&src) {
                        Ok(name) => trf("Added {name}", &[("name", &name)]),
                        Err(e) => trf("Couldn't add the model: {e}", &[("e", &e)]),
                    }));
                }
            });
        }
    }

    /// Shows the download's licence; the download starts only once it's accepted.
    pub(super) fn license_dialog(&mut self, ctx: &egui::Context) {
        let Some((download, step)) = self.license_prompt else { return };
        let (title, licenses, note) = match download {
            Download::Model(i) => (tr(crate::ai::CATALOG[i].label), crate::ai::CATALOG[i].licenses, crate::ai::CATALOG[i].converted_by.map(tr)),
            Download::Runtime => (tr("AI runtime (ONNX Runtime)"), std::slice::from_ref(&crate::ai::RUNTIME_LICENSE), None),
            Download::Gpu => (tr("GPU support"), &crate::ai::GPU_LICENSES[..], Some(tr("Downloaded from Microsoft (ONNX Runtime) and NVIDIA (CUDA libraries, via their official PyPI packages)."))),
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
                        self.set_status(trf("Downloading GPU support ({mb} MB)…", &[("mb", &crate::ai::GPU_DOWNLOAD_MB)]));
                        std::thread::spawn(move || notify.send(Msg::Downloaded("gpu".into(), crate::ai::download_gpu())));
                    }
                    Download::Runtime => {
                        self.set_status(tr("Downloading the AI runtime…"));
                        std::thread::spawn(move || {
                            notify.send(Msg::Status(match crate::ai::download_runtime() {
                                Ok(()) => tr("Installed the AI runtime").into(),
                                Err(e) => trf("Couldn't install the AI runtime: {e}", &[("e", &e)]),
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

    pub(super) fn resize_dialog(&mut self, ctx: &egui::Context) {
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
    pub(super) fn discard_dialog(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            return;
        }
        let name = self.current().and_then(|p| p.file_name()).unwrap_or_default().to_string_lossy().into_owned();
        let modal = egui::Modal::new(egui::Id::unique("discard")).show(ctx, |ui| {
            ui.heading(tr("Discard edits?"));
            ui.label(trf("Your edits to {name} haven't been saved.", &[("name", &name)]));
            ui.add_space(6.0);
            ui.horizontal(|ui| (ui.button(tr("Discard")).clicked(), ui.button(tr("Cancel")).clicked())).inner
        });
        let (discard, cancel) = modal.inner;
        if discard && let Some(p) = self.pending.take() {
            self.shown.doc = None;
            self.confirm(p);
        } else if cancel || modal.should_close() {
            self.pending = None;
        }
    }

    /// Always asks before moving the current image to the Trash.
    /// Asks before the markup is flattened into the corrected image.
    pub(super) fn perspective_dialog(&mut self, ctx: &egui::Context) {
        if !self.shown.perspective_prompt {
            return;
        }
        let modal = egui::Modal::new(egui::Id::unique("perspective")).show(ctx, |ui| {
            ui.heading(tr("Merge drawings into the image?"));
            ui.label(tr("Drawings, text and pasted images will be merged into the image and can't be edited afterwards. Ctrl+Z undoes it."));
            ui.add_space(6.0);
            confirm_buttons(ui)
        });
        let (accept, cancel) = modal.inner;
        if accept {
            self.shown.perspective_prompt = false;
            self.apply_perspective();
        } else if cancel || modal.should_close() {
            self.shown.perspective_prompt = false;
        }
    }

    /// The file changed on disk under unsaved edits: only the user can say which one wins.
    pub(super) fn changed_dialog(&mut self, ctx: &egui::Context) {
        if !self.shown.changed_prompt {
            return;
        }
        let name = self.current().and_then(|p| p.file_name()).unwrap_or_default().to_string_lossy().into_owned();
        let modal = egui::Modal::new(egui::Id::unique("changed")).show(ctx, |ui| {
            ui.heading(tr("File changed on disk"));
            ui.label(trf("{name} changed on disk. Reloading it discards your edits.", &[("name", &name)]));
            ui.add_space(6.0);
            ui.horizontal(|ui| (ui.button(tr("Reload")).clicked(), ui.button(tr("Keep Edits")).clicked())).inner
        });
        let (reload, keep) = modal.inner;
        if reload || keep || modal.should_close() {
            self.answer_changed(reload);
        }
    }

    pub(super) fn answer_changed(&mut self, reload: bool) {
        self.shown.changed_prompt = false;
        if reload {
            self.shown.doc = None;
            self.reload();
        }
    }

    /// Asks for the current image's new file name. Enter accepts; a refused name stays to be fixed.
    pub(super) fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut r) = self.renaming.take() else { return };
        let modal = egui::Modal::new(egui::Id::unique("rename")).show(ctx, |ui| {
            ui.heading(tr("Rename Image"));
            ui.add_space(6.0);
            let mut edit = egui::TextEdit::singleline(&mut r.name).desired_width(320.0).show(ui);
            if std::mem::take(&mut r.fresh) {
                edit.response.request_focus();
                let stem = r.name.rsplit_once('.').map_or(&r.name[..], |(stem, _)| stem).chars().count();
                edit.state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(stem))));
                edit.state.store(ui.ctx(), edit.response.id);
            }
            let enter = edit.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(e) = &r.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.add_space(6.0);
            let (rename, cancel) = ui.horizontal(|ui| (ui.button(tr("Rename")).clicked(), ui.button(tr("Cancel")).clicked())).inner;
            (enter || rename, cancel)
        });
        let (accept, cancel) = modal.inner;
        if cancel || modal.should_close() {
            return;
        }
        if !accept {
            self.renaming = Some(r);
        } else if r.name != r.original && let Err(e) = self.rename_current(&r.name) {
            self.renaming = Some(Renaming { error: Some(e), fresh: true, ..r });
        }
    }

    pub(super) fn delete_dialog(&mut self, ctx: &egui::Context) {
        if !self.delete_prompt {
            return;
        }
        let name = self.current().and_then(|p| p.file_name()).unwrap_or_default().to_string_lossy().into_owned();
        let dirty = self.dirty();
        let modal = egui::Modal::new(egui::Id::unique("delete")).show(ctx, |ui| {
            ui.heading(tr("Delete image?"));
            ui.label(trf("{name} will be moved to the Trash.", &[("name", &name)]));
            if dirty {
                ui.label(trf("Your edits to {name} haven't been saved.", &[("name", &name)]));
            }
            ui.add_space(6.0);
            confirm_buttons(ui)
        });
        let (delete, cancel) = modal.inner;
        if delete {
            self.delete_prompt = false;
            self.delete_image();
        } else if cancel || modal.should_close() {
            self.delete_prompt = false;
        }
    }
}

/// Continue and Cancel, with Continue focused so that Enter accepts.
fn confirm_buttons(ui: &mut egui::Ui) -> (bool, bool) {
    ui.horizontal(|ui| {
        let (accept, cancel) = (ui.button(tr("Continue")), ui.button(tr("Cancel")));
        if !cancel.has_focus() {
            accept.request_focus();
        }
        (accept.clicked(), cancel.clicked())
    }).inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markup;
    use eframe::egui::{Key, Modifiers, Pos2, Rect, Sense, Vec2};

    // Scroll zoom relies on this: a modal must stop the image being hovered, and be seen by the next frame.
    #[test]
    fn modal_blocks_image_hover() {
        fn hovered(modal: bool) -> (bool, bool) {
            let ctx = egui::Context::default();
            ctx.set_fonts(markup::font_definitions());
            let mut hovered = false;
            for _ in 0..4 {
                let mut input = egui::RawInput::default();
                input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
                input.events.push(egui::Event::PointerMoved(Pos2::new(20.0, 20.0)));
                let mut output = ctx.run_ui(input, |ui| {
                    let viewport = ui.available_rect_before_wrap();
                    hovered = ui.interact(viewport, ui.scope_id().with("image"), Sense::click_and_drag()).hovered();
                    if modal {
                        egui::Modal::new(egui::Id::unique("dialog")).show(ui.ctx(), |ui| ui.label("dialog"));
                    }
                });
                output.textures_delta.clear();
            }
            (hovered, ctx.memory(|m| m.top_modal_layer().is_some()))
        }
        assert_eq!(hovered(false), (true, false));
        assert_eq!(hovered(true), (false, true));
    }

    #[test]
    fn enter_accepts_the_focused_button() {
        fn press(tab_first: bool) -> (bool, bool) {
            let ctx = egui::Context::default();
            ctx.set_fonts(markup::font_definitions());
            let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
            let mut result = (false, false);
            let keys: &[Option<Key>] = if tab_first { &[None, None, Some(Key::Tab), None, Some(Key::Enter)] } else { &[None, None, Some(Key::Enter)] };
            for k in keys {
                let mut input = egui::RawInput::default();
                input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
                input.events.extend(k.map(key));
                let mut output = ctx.run_ui(input, |ui| result = confirm_buttons(ui));
                output.textures_delta.clear();
            }
            result
        }
        assert_eq!(press(false), (true, false));
        assert_eq!(press(true), (false, true));
    }
}
