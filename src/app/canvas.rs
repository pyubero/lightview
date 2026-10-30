use super::{App, Tool};
use crate::edit::{self, Doc};
use crate::load::Frame;
use crate::markup;
use crate::settings::Interpolation;
use crate::view::{self, Transform};
use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, TextureOptions, Vec2};

impl App {
    /// Draws the markup over the image, plus the selection and the text being typed.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_markup(&mut self, ui: &egui::Ui, doc: &Doc, dest: Rect, crop: Rect, texture: egui::TextureId, uv: &dyn Fn(Pos2) -> Pos2) {
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
}

/// Resizes the `crop` part of `frame` so that, after `transform`, it's `size` px.
/// Transforming the small resized output instead of the full image keeps rotating cheap.
pub(super) fn resize(frame: &Frame, crop: Rect, size: [u32; 2], transform: Transform, interpolation: Interpolation) -> Vec<u8> {
    let [w, h] = if transform.rot % 2 == 1 { [size[1], size[0]] } else { size };
    let pixels = edit::resize(frame, crop, w, h, interpolation.cpu());
    if transform == Transform::default() {
        pixels
    } else {
        transform.apply(edit::to_dynamic(frame.alpha, w, h, pixels).unwrap()).into_bytes()
    }
}

/// A short message in a dark rounded box centred on `center`.
pub(super) fn bubble(painter: &egui::Painter, center: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(14.0), Color32::WHITE);
    let rect = Rect::from_center_size(center, galley.size() + Vec2::new(16.0, 8.0));
    painter.rect_filled(rect, 4.0, Color32::from_black_alpha(190));
    painter.galley(rect.min + Vec2::new(8.0, 4.0), galley, Color32::WHITE);
}

pub(super) fn to_color_image(alpha: bool, size: [usize; 2], pixels: &[u8]) -> ColorImage {
    if alpha { ColorImage::from_rgba_unmultiplied(size, pixels) } else { ColorImage::from_rgb(size, pixels) }
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
}
