use super::{Action, App, Grab, Tool};
use crate::edit::Doc;
use crate::markup::{self, Element, Pick, Shape, Style};
use crate::view;
use eframe::egui::{self, CursorIcon, Pos2, Rect, Vec2};

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
}
