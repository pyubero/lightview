use super::menu::SHORTCUTS;
use super::{Action, App, Download, Grab, Pending, SelKind, Tool};
use crate::edit::{Adjust, Doc, ResizeDialog};
use crate::i18n::trf;
use crate::load::Loaded;
use crate::markup::{self, Element, Pick, Selection, Shape, Style};
use crate::view::{self, View};
use eframe::egui::{self, ColorImage, CursorIcon, Key, Pos2, Rect, Sense, Vec2, ViewportCommand};
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
    pub(super) fn read_shortcuts(ctx: &egui::Context, typing: bool) -> Vec<Action> {
        ctx.input_mut(|i| {
            SHORTCUTS
                .iter()
                .filter(|(sc, a)| !(typing && (sc.modifiers.is_none() || matches!(a, Action::Undo | Action::Redo | Action::SelectAll | Action::Deselect))) && i.consume_shortcut(sc))
                .map(|(_, a)| a.clone())
                .collect()
        })
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
                Action::OpenRecent(path) => self.confirm(Pending::Open(path.clone())),
                Action::Close => self.confirm(Pending::Close),
                Action::Undo => self.undo_redo(true),
                Action::Redo => self.undo_redo(false),
                Action::Crop if self.selection.is_some() => self.image_from_selection(),
                Action::Crop if self.doc().is_some() => (self.tool, self.crop_rect) = (Tool::Crop, None),
                Action::Resize => self.resize_dialog = self.doc().map(|d| ResizeDialog::new(&d)),
                Action::Info => self.show_info = !self.show_info,
                Action::Settings => self.show_settings = true,
                Action::AdjustColor => {
                    if self.adjusting.is_none() {
                        self.adjusting = self.doc();
                    }
                }
                Action::AutoLevels => {
                    if let Some([exposure, contrast]) = self.auto_levels() {
                        self.edit(|d| (d.adjust.exposure, d.adjust.contrast) = (exposure, contrast));
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
                Action::Restore(task) if *task == crate::ai::Task::Enhance && let Some(file) = self.unseen_notice() => self.enhance_notice = Some(file),
                Action::Restore(task) => self.preview_restore(*task),
                Action::DownloadRuntime => self.license_prompt = Some((Download::Runtime, 0)),
                Action::AiModels => self.show_ai_models = true,
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
            Tool::Pan if drag.dragged() => self.view = view::pan(self.view, drag.drag_delta(), img, viewport, ppp),
            Tool::Pan => {}
            Tool::Select | Tool::Draw(_) => self.markup_input(ctx, &drag, dest, vis, doc),
            Tool::Selection(kind) => self.selection_input(ctx, &drag, dest, vis, img, kind),
            Tool::Crop | Tool::Measure => self.crop_measure_input(ctx, &drag, dest, vis, img),
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
                (Tool::Crop, _) => self.crop_rect = Some(Rect::from_two_pos(a, at(p))),
                _ => {}
            }
        }
    }

    /// The scroll wheel, and the zoom actions, which wait until the image is known.
    pub(super) fn zoom(&mut self, ctx: &egui::Context, drag: &egui::Response, actions: &[Action], img: Vec2, viewport: Rect, ppp: f32) {
        let (factor, hover) = ctx.input(|i| (i.zoom_delta() * (i.smooth_scroll_delta.y / 200.0).exp(), i.pointer.hover_pos()));
        if factor != 1.0 && drag.hovered() {
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
}
