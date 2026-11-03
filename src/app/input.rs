use super::dialogs::Renaming;
use super::{Action, App, Download, Grab, Pending, SelKind, Tool};
use crate::edit::{self, Adjust, Doc, ResizeDialog};
use crate::i18n::{tr, trf};
use crate::load::Loaded;
use crate::markup::{self, Element, Pick, Selection, Shape, Style};
use crate::view::{self, View};
use eframe::egui::{self, ColorImage, CursorIcon, Key, KeyboardShortcut, Modifiers, PointerButton, Pos2, Rect, Sense, Vec2, ViewportCommand};
use std::sync::Arc;
use std::time::Instant;

impl App {
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

    pub(super) fn set_pick(&mut self, pick: Pick) {
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
    pub(super) fn finish_text(&mut self) {
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

    /// The markup toolbar, below the menu bar.
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let Some(doc) = self.doc() else { return };
        let defaults = *self.style.get_or_insert_with(|| Style::for_image(doc.size_vec()));
        let selected = self.selected.and_then(|i| doc.elements.get(i));
        let (mut style, styled) = match (selected, self.tool) {
            (Some(e), _) => (e.style, Some(e.shape)),
            (None, Tool::Draw(shape)) => (defaults, Some(shape)),
            _ => (defaults, None),
        };
        let pick = self.pick();
        let out = egui::Panel::top("markup").show(ui, |ui| markup::toolbar_ui(ui, pick, &mut style, styled, !(self.undo.is_empty() && self.deleted.is_empty()), !self.redo.is_empty())).inner;
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
    pub(super) fn markup_input(&mut self, ctx: &egui::Context, drag: &egui::Response, dest: Rect, vis: Rect, doc: &Doc) {
        if drag.hovered() && self.tool != Tool::Select {
            ctx.set_cursor_icon(CursorIcon::Crosshair);
        }
        let k = dest.width() / vis.width();
        let bounds = doc.bounds();
        let at = |p: Pos2| bounds.clamp(view::to_image(dest, vis, p));
        let origin = ctx.input(|i| i.pointer.press_origin());
        let pointer = drag.interact_pointer_pos();
        let style = *self.style.get_or_insert_with(|| Style::for_image(doc.size_vec()));
        let hit = |p: Pos2| doc.elements.iter().rposition(|e| e.shape != Shape::Erase && e.bbox().expand(6.0 / k).contains(p));
        match self.tool {
            Tool::Draw(shape @ (Shape::Text | Shape::Magnifier)) => {
                if drag.clicked() && let Some(p) = pointer.map(at) {
                    self.finish_text();
                    let e = if shape == Shape::Magnifier { Element::lens(p, bounds.size(), style) } else { Element::new(shape, p, style) };
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

    /// The shortcuts pressed this frame. Plain keys (arrows, F11) and undo belong to a text field while one has focus.
    pub(super) fn read_shortcuts(ctx: &egui::Context, typing: bool, keys: &[(KeyboardShortcut, Action)]) -> Vec<Action> {
        let modal = ctx.memory(|m| m.top_modal_layer().is_some());
        ctx.input_mut(|i| {
            let mut actions: Vec<Action> = keys
                .iter()
                .filter(|(sc, a)| !(typing && (sc.modifiers.is_none() || matches!(a, Action::Undo | Action::Redo | Action::SelectAll | Action::Deselect))) && i.consume_shortcut(sc))
                .map(|(_, a)| a.clone())
                .collect();
            // The mouse's back and forward buttons.
            for (button, action) in [(PointerButton::Extra1, Action::PrevImage), (PointerButton::Extra2, Action::NextImage)] {
                if !typing && !modal && i.pointer.button_pressed(button) {
                    actions.push(action);
                }
            }
            actions
        })
    }

    /// Settings' Change: the next key press, with its modifiers, becomes `recording`'s shortcut
    /// unless another action has it; Esc cancels. Nothing else sees the key, so Esc doesn't also
    /// close Settings and Ctrl+R doesn't also rotate.
    pub(super) fn record_shortcut(&mut self, ctx: &egui::Context) {
        let Some(action) = self.recording.clone() else { return };
        let pressed = ctx.input_mut(|i| {
            let pressed = i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some(KeyboardShortcut::new(*modifiers, *key)),
                _ => None,
            });
            i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) | egui::Event::Text(_)));
            pressed
        });
        let Some(sc) = pressed else { return };
        self.recording = None;
        if sc == KeyboardShortcut::new(Modifiers::NONE, Key::Escape) {
            return;
        }
        match super::menu::taken(&super::menu::shortcuts(&self.settings.shortcuts), sc, &action) {
            Some(other) => self.shortcut_error = Some(trf("{keys} is already used by {action}", &[("keys", &ctx.format_shortcut(&sc)), ("action", &other)])),
            None => self.set_shortcut(&action, Some(sc)),
        }
    }

    /// Gives `action` the shortcut `sc`, or none.
    pub(super) fn set_shortcut(&mut self, action: &Action, sc: Option<KeyboardShortcut>) {
        let name = format!("{action:?}");
        self.settings.shortcuts.retain(|(n, _)| *n != name);
        self.settings.shortcuts.push((name, sc));
        self.shortcut_error = None;
    }

    /// Adds this frame's clipboard events to `actions`; returns a pasted image.
    pub(super) fn read_clipboard(ctx: &egui::Context, actions: &mut Vec<Action>) -> Option<Arc<ColorImage>> {
        let mut pasted = None;
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
        pasted
    }

    pub(super) fn run_actions(&mut self, actions: &[Action], ctx: &egui::Context, fullscreen: bool) {
        for action in actions {
            match action {
                Action::Open => self.open_dialog(),
                Action::SaveAs => self.save_dialog(),
                Action::SaveCopy => self.save_copy(),
                Action::CopyPath => {
                    if let Some(path) = self.current() {
                        ctx.copy_text(path.display().to_string());
                        self.set_status(tr("Copied the path"));
                    }
                }
                Action::CopyFile => self.copy_file(),
                Action::ShowInFolder => {
                    if let Some(path) = self.current() {
                        crate::platform::show_in_folder(path);
                    }
                }
                Action::Rename if self.restoring.is_none() => self.renaming = self.current().map(|p| Renaming::new(p)),
                Action::OpenRecent(path) => self.confirm(Pending::Open(path.clone())),
                Action::Close => self.confirm(Pending::Close),
                Action::Undo => self.undo_redo(true),
                Action::Redo => self.undo_redo(false),
                Action::Crop if self.selection.is_some() => self.image_from_selection(),
                Action::Crop if self.doc().is_some() => (self.tool, self.crop_rect) = (Tool::Crop, None),
                Action::Perspective => {
                    if let Some(doc) = self.doc() {
                        self.finish_text();
                        let r = doc.bounds().shrink2(doc.size_vec() * 0.1);
                        self.perspective = Some([r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]);
                        (self.tool, self.crop_rect, self.selected) = (Tool::Perspective, None, None);
                    }
                }
                Action::Resize => self.resize_dialog = self.doc().map(|d| ResizeDialog::new(&d)),
                Action::Info => self.show_info = !self.show_info,
                Action::Settings => self.show_settings = true,
                Action::AdjustColor => {
                    if self.adjusting.is_none() {
                        self.adjusting = self.doc();
                    }
                    self.curve_panel = None;
                }
                Action::AdjustCurves => {
                    if self.adjusting.is_none() {
                        self.adjusting = self.doc();
                    }
                    self.curve_panel.get_or_insert_default();
                }
                Action::AutoLevels => {
                    if let Some([exposure, contrast]) = self.auto_levels() {
                        self.edit(|d| (d.adjust.exposure, d.adjust.contrast) = (exposure, contrast));
                    }
                }
                Action::ResetAdjust => self.edit(|d| d.adjust = Adjust::default()),
                Action::Greyscale => self.edit(|d| d.adjust.saturation = if d.adjust.saturation == -1.0 { 0.0 } else { -1.0 }),
                Action::Invert => self.edit(|d| d.adjust.invert = !d.adjust.invert),
                Action::RedEye => self.patch_selection(crate::retouch::remove_red_eye, tr("Red eye removed")),
                Action::Pixelate => self.patch_selection(crate::retouch::pixelate, tr("Pixelated")),
                Action::Straighten => {
                    if self.doc().is_some() {
                        self.finish_text();
                        (self.tool, self.straighten, self.crop_rect, self.selected) = (Tool::Straighten, 0.0, None, None);
                    }
                }
                Action::Measure if self.tool == Tool::Measure => (self.tool, self.measure) = (Tool::Pan, None),
                Action::Measure if self.doc().is_some() => (self.tool, self.crop_rect) = (Tool::Measure, None),
                Action::CopyColor if self.tool == Tool::CopyColor => self.tool = Tool::Pan,
                Action::CopyColor if self.doc().is_some() => {
                    self.finish_text();
                    (self.tool, self.crop_rect, self.selected) = (Tool::CopyColor, None, None);
                }
                Action::ShowToolbar => {
                    self.show_toolbar = !self.show_toolbar;
                    if !self.show_toolbar {
                        self.finish_text();
                        (self.tool, self.selected) = (Tool::Pan, None);
                    }
                }
                Action::Pick(pick) if self.doc().is_some() => self.set_pick(*pick),
                Action::Describe => self.show_info = true,
                Action::SelectTool(kind) if self.doc().is_some() => {
                    self.finish_text();
                    (self.tool, self.crop_rect, self.selected) = (Tool::Selection(*kind), None, None);
                }
                Action::SelectAll => self.selection = self.doc().map(|d| Selection::Rect(d.bounds())),
                Action::Deselect => self.selection = None,
                Action::Copy => self.copy_selection(ctx),
                Action::Cut => {
                    if self.selection.is_some() {
                        self.copy_selection(ctx);
                        self.delete_selection();
                    }
                }
                Action::Paste => match arboard::Clipboard::new().and_then(|mut c| c.get_image()) {
                    Ok(img) => self.paste(img.width as u32, img.height as u32, img.bytes.into_owned()),
                    Err(e) => self.set_status(trf("Nothing to paste: {e}", &[("e", &e)])),
                },
                Action::DeleteSelection => self.delete_selection(),
                Action::DeleteImage => self.delete_prompt = true,
                Action::Cutout(crop) => self.cutout(*crop),
                Action::ReadText(qr) => self.read_text(*qr),
                Action::Restore(task) if *task == crate::ai::Task::Enhance && let Some(file) = self.unseen_notice() => self.enhance_notice = Some(file),
                Action::Restore(task) => self.preview_restore(*task),
                Action::Inpaint => self.inpaint(),
                Action::DownloadRuntime => self.license_prompt = Some((Download::Runtime, 0)),
                Action::AiModels => self.show_ai_models = true,
                Action::RotateLeft => self.edit(Doc::rotate_left),
                Action::RotateRight => self.edit(Doc::rotate_right),
                Action::FlipHorizontal => self.edit(Doc::flip_horizontal),
                Action::FlipVertical => self.edit(Doc::flip_vertical),
                Action::Fullscreen => ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen)),
                Action::PrevImage => self.confirm(Pending::Navigate(-1)),
                Action::NextImage => self.confirm(Pending::Navigate(1)),
                Action::FirstImage => self.go_to(0),
                Action::LastImage => self.go_to(self.files.len().saturating_sub(1)),
                Action::Play => (self.anim_paused, self.anim_due) = (false, None),
                Action::Pause => self.anim_paused = true,
                _ => {} // zoom and frame-step actions are applied below, once the image is known
            }
        }
    }

    pub(super) fn paste_event(&mut self, img: &ColorImage) {
        let pixels = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        self.paste(img.size[0] as u32, img.size[1] as u32, pixels);
    }

    /// Esc, Enter and Delete, which mean different things depending on the tool.
    pub(super) fn handle_keys(&mut self, ctx: &egui::Context, typing: bool, esc: bool, fullscreen: bool) {
        let enter = !typing && ctx.input(|i| i.key_pressed(Key::Enter));
        let delete_key = !typing && ctx.input(|i| i.key_pressed(Key::Delete));
        let delete = delete_key || !typing && ctx.input(|i| i.key_pressed(Key::Backspace));
        if esc && !typing && let Some(before) = self.change_base.take() {
            // Cancel the drawing in progress.
            (self.doc, self.grab) = (Some(before), Grab::None);
        } else if esc && !typing && self.tool != Tool::Pan && !self.perspective_prompt {
            if matches!(self.tool, Tool::Selection(_)) {
                self.selection = None;
            }
            (self.tool, self.crop_rect, self.measure, self.selected, self.perspective) = (Tool::Pan, None, None, None, None);
        } else if esc && !typing && fullscreen {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        if enter && self.tool == Tool::Crop && let Some(r) = self.crop_rect.take() {
            self.edit(|d| d.crop_to(r));
            self.tool = Tool::Pan;
        }
        if enter && self.tool == Tool::Straighten && !self.perspective_prompt {
            self.apply_straighten();
        }
        if enter && self.tool == Tool::Perspective && !self.perspective_prompt && self.adjusting.is_none() {
            if self.doc.as_ref().is_some_and(|d| !d.elements.is_empty()) {
                self.perspective_prompt = true;
            } else {
                self.apply_perspective();
            }
        }
        if delete && self.tool == Tool::Select && let Some(i) = self.selected.take() {
            self.edit(|d| drop(d.elements.remove(i)));
        } else if delete && self.selection.is_some() {
            self.delete_selection();
        } else if delete_key && self.current().is_some() {
            self.delete_prompt = true;
        }
    }

    /// Dragging and clicking on the image, for the compare line or the current tool.
    pub(super) fn image_input(&mut self, ui: &egui::Ui, doc: &Doc, viewport: Rect, ppp: f32) -> egui::Response {
        let ctx = ui.ctx();
        let img = doc.size_vec();
        let drag = ui.interact(viewport, ui.scope_id().with("image"), Sense::click_and_drag());
        let (dest, vis) = view::layout(self.view, img, viewport, ppp);
        // A drag started with the right button pans, whatever the tool; the tools never see it.
        if drag.drag_started_by(egui::PointerButton::Secondary) {
            self.right_pan = true;
        }
        if self.right_pan {
            if drag.dragged() {
                self.view = view::pan(self.view, drag.drag_delta(), img, viewport, ppp);
            }
            self.right_pan = !drag.drag_stopped();
            return drag;
        }
        if let Some(c) = &mut self.compare {
            // Dragging moves the before/after line instead of using the tool.
            if drag.dragged() && let Some(p) = drag.interact_pointer_pos() {
                let left = view::to_screen(dest, vis, c.region.min).x;
                let right = view::to_screen(dest, vis, c.region.max).x;
                c.split = ((p.x - left) / (right - left).max(1.0)).clamp(0.0, 1.0);
            }
            return drag;
        }
        match self.tool {
            Tool::Pan | Tool::Straighten if drag.dragged() => self.view = view::pan(self.view, drag.drag_delta(), img, viewport, ppp),
            Tool::Pan | Tool::Straighten => {}
            Tool::Select | Tool::Draw(_) => self.markup_input(ctx, &drag, dest, vis, doc),
            Tool::Selection(kind) => self.selection_input(ctx, &drag, dest, vis, img, kind),
            Tool::Crop | Tool::Measure => self.crop_measure_input(ctx, &drag, dest, vis, img),
            Tool::Perspective => self.perspective_input(ctx, &drag, dest, vis, img),
            Tool::CopyColor => self.copy_color_input(ctx, &drag, dest, vis, doc),
        }
        drag
    }

    /// Selection tools: dragging out a rectangle, ellipse or lasso.
    fn selection_input(&mut self, ctx: &egui::Context, drag: &egui::Response, dest: Rect, vis: Rect, img: Vec2, kind: SelKind) {
        if drag.hovered() {
            ctx.set_cursor_icon(CursorIcon::Crosshair);
        }
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

    /// Crop and Measure tools: dragging out the crop rect or a measuring line.
    fn crop_measure_input(&mut self, ctx: &egui::Context, drag: &egui::Response, dest: Rect, vis: Rect, img: Vec2) {
        if drag.hovered() {
            ctx.set_cursor_icon(CursorIcon::Crosshair);
        }
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
                (Tool::Crop, _) => self.crop_rect = Some(Rect::from_two_pos(a, view::snap_ratio(a, at(p), bounds))),
                _ => {}
            }
        }
    }

    /// Copy Color tool: a click copies the pixel's colour as hex, as it's seen with the edits.
    fn copy_color_input(&mut self, ctx: &egui::Context, drag: &egui::Response, dest: Rect, vis: Rect, doc: &Doc) {
        if drag.hovered() {
            ctx.set_cursor_icon(CursorIcon::Crosshair);
        }
        let Some(p) = drag.interact_pointer_pos().filter(|_| drag.clicked()) else { return };
        let Some(l) = self.current().and_then(|c| self.cache.get(c)).cloned() else { return };
        // shortcut: renders the full edited image on the UI thread, like Copy; fine for one click.
        let Ok(img) = edit::render(&l.frames[self.anim_frame.min(l.frames.len() - 1)], doc) else { return };
        let img = img.into_rgba8();
        let p = view::to_image(dest, vis, p);
        let (x, y) = ((p.x as u32).min(img.width() - 1), (p.y as u32).min(img.height() - 1));
        let color = hex_color(img.get_pixel(x, y).0);
        ctx.copy_text(color.clone());
        self.set_status(trf("Copied {color}", &[("color", &color)]));
    }

    /// Perspective tool: dragging the corner nearest the press.
    fn perspective_input(&mut self, ctx: &egui::Context, drag: &egui::Response, dest: Rect, vis: Rect, img: Vec2) {
        if drag.hovered() {
            ctx.set_cursor_icon(CursorIcon::Crosshair);
        }
        let Some(corners) = &mut self.perspective else { return };
        if drag.drag_started() && let Some(o) = ctx.input(|i| i.pointer.press_origin()) {
            let off = |i: usize| view::to_screen(dest, vis, corners[i]).distance(o);
            self.perspective_drag = (0..4).min_by(|&a, &b| off(a).total_cmp(&off(b))).filter(|&i| off(i) < 14.0);
        }
        if drag.dragged() && let (Some(i), Some(p)) = (self.perspective_drag, drag.interact_pointer_pos()) {
            corners[i] = Rect::from_min_size(Pos2::ZERO, img).clamp(view::to_image(dest, vis, p));
        }
        if drag.drag_stopped() {
            self.perspective_drag = None;
        }
    }

    /// The scroll wheel, and the zoom actions, which wait until the image is known.
    pub(super) fn zoom(&mut self, ctx: &egui::Context, drag: &egui::Response, actions: &[Action], img: Vec2, viewport: Rect, ppp: f32) {
        let (factor, hover) = ctx.input(|i| (i.zoom_delta() * (i.smooth_scroll_delta.y / 200.0).exp(), i.pointer.hover_pos()));
        let modal_open = ctx.memory(|m| m.top_modal_layer().is_some());
        if factor != 1.0 && drag.hovered() && !modal_open {
            self.view = view::zoom_at(self.view, factor, hover.unwrap_or(viewport.center()), img, viewport, ppp);
        }
        for action in actions {
            self.view = match action {
                Action::ZoomIn => view::zoom_at(self.view, 1.25, viewport.center(), img, viewport, ppp),
                Action::ZoomOut => view::zoom_at(self.view, 0.8, viewport.center(), img, viewport, ppp),
                Action::Fit => View::Fit,
                Action::ActualSize => view::actual_size(img),
                _ => self.view,
            };
        }
    }

    /// Puts the current file on the clipboard as a file, for pasting into chats and file managers.
    // shortcut: GNOME Files may want x-special/gnome-copied-files, which arboard doesn't offer; add it if that's reported.
    fn copy_file(&mut self) {
        let Some(path) = self.current().cloned() else { return };
        let clipboard = match self.file_clipboard.take() {
            Some(c) => Ok(c),
            None => arboard::Clipboard::new(),
        };
        let result = clipboard.and_then(|mut c| {
            let copied = c.set().file_list(&[&path]);
            self.file_clipboard = Some(c);
            copied
        });
        match result {
            Ok(()) => self.set_status(tr("Copied the file")),
            Err(e) => self.set_status(trf("Couldn't copy the file: {e}", &[("e", &e)])),
        }
    }

    /// Steps the animation: by frame-step actions and, unless paused, by time.
    pub(super) fn animate(&mut self, ctx: &egui::Context, l: &Loaded, actions: &[Action]) {
        let frames = &l.frames;
        for action in actions {
            let step = match action {
                Action::NextFrame => 1,
                Action::PrevFrame => frames.len() - 1,
                _ => continue,
            };
            self.anim_frame = (self.anim_frame + step) % frames.len();
            self.anim_paused = true;
        }
        if frames.len() > 1 && !self.anim_paused && !l.paged() {
            let now = Instant::now();
            let due = *self.anim_due.get_or_insert(now + frames[self.anim_frame].delay);
            if now >= due {
                self.anim_frame = (self.anim_frame + 1) % frames.len();
                self.anim_due = Some(now + frames[self.anim_frame].delay);
            }
            ctx.request_repaint_after(self.anim_due.unwrap() - now);
        }
    }
}

/// `#RRGGBB`, with `AA` added when the colour isn't opaque.
fn hex_color([r, g, b, a]: [u8; 4]) -> String {
    if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex_colors() {
        assert_eq!(super::hex_color([255, 16, 0, 255]), "#FF1000");
        assert_eq!(super::hex_color([0, 0, 255, 128]), "#0000FF80");
    }
}
