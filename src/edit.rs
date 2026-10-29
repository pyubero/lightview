use crate::load::Frame;
use crate::view::Transform;
use eframe::egui::{self, Pos2, Rect, Vec2};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::{Image, ImageRef}};
use image::DynamicImage;

/// The edits made to the current image, as one state; undo keeps a stack of these.
/// The output is `crop` of the decoded image, then `transform`, then scaled to `size`.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    /// In decoded-image pixels.
    pub crop: Rect,
    pub transform: Transform,
    /// Output size in pixels, after the transform.
    pub size: [u32; 2],
    /// Written into saved JPEGs; None keeps the default.
    pub dpi: Option<u16>,
}

impl Doc {
    /// No edits on a `w`×`h` image.
    pub fn new(w: u32, h: u32) -> Self {
        Doc { crop: Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32)), transform: Transform::default(), size: [w, h], dpi: None }
    }

    pub fn size_vec(&self) -> Vec2 {
        Vec2::new(self.size[0] as f32, self.size[1] as f32)
    }

    /// The crop and the transform: what decides the view's layout.
    pub fn geometry(&self) -> (Rect, Transform, [u32; 2]) {
        (self.crop, self.transform, self.size)
    }

    /// Output px per transformed-crop px, on each axis.
    fn scale(&self) -> Vec2 {
        self.size_vec() / self.transform.size(self.crop.size())
    }

    /// The decoded-image point that ends up at output point `p`.
    pub fn source_point(&self, p: Pos2) -> Pos2 {
        self.crop.min + self.transform.source_point((p.to_vec2() / self.scale()).to_pos2(), self.crop.size()).to_vec2()
    }

    /// The decoded-image rect that ends up at output rect `r`.
    pub fn to_source(&self, r: Rect) -> Rect {
        let scaled = Rect::from_min_max((r.min.to_vec2() / self.scale()).to_pos2(), (r.max.to_vec2() / self.scale()).to_pos2());
        self.transform.to_source(scaled, self.crop.size()).translate(self.crop.min.to_vec2())
    }

    pub fn rotate_right(&mut self) {
        self.transform = self.transform.rotate_right();
        self.size = [self.size[1], self.size[0]];
    }

    pub fn rotate_left(&mut self) {
        self.transform = self.transform.rotate_left();
        self.size = [self.size[1], self.size[0]];
    }

    pub fn flip_horizontal(&mut self) {
        self.transform = self.transform.flip_horizontal();
    }

    pub fn flip_vertical(&mut self) {
        self.transform = self.transform.flip_vertical();
    }

    /// Crops to `r` (output px, rounded to whole pixels), keeping the pixel scale.
    pub fn crop_to(&mut self, r: Rect) {
        let r = Rect::from_min_max(r.min.round(), r.max.round()).intersect(Rect::from_min_size(Pos2::ZERO, self.size_vec()));
        if r.width() < 1.0 || r.height() < 1.0 {
            return;
        }
        self.crop = self.to_source(r);
        self.size = [r.width() as u32, r.height() as u32];
    }

    /// Scales the output to `w`×`h` px.
    pub fn resize(&mut self, w: u32, h: u32) {
        self.size = [w.max(1), h.max(1)];
    }
}

/// Resizes the `crop` part of `frame` to `w`×`h` px (before any transform).
pub fn resize(frame: &Frame, crop: Rect, w: u32, h: u32, alg: ResizeAlg) -> Vec<u8> {
    let pixel_type = if frame.alpha { PixelType::U8x4 } else { PixelType::U8x3 };
    let src = ImageRef::new(frame.w, frame.h, &frame.pixels, pixel_type).unwrap();
    let mut dst = Image::new(w, h, pixel_type);
    let opts = ResizeOptions::new().resize_alg(alg).crop(crop.min.x.into(), crop.min.y.into(), crop.width().into(), crop.height().into());
    Resizer::new().resize(&src, &mut dst, &opts).unwrap();
    dst.into_vec()
}

pub fn to_dynamic(alpha: bool, w: u32, h: u32, pixels: Vec<u8>) -> Option<DynamicImage> {
    Some(if alpha {
        DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, pixels)?)
    } else {
        DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, pixels)?)
    })
}

/// The full-size edited image, for saving.
pub fn render(frame: &Frame, doc: &Doc) -> Result<DynamicImage, String> {
    let [w, h] = if doc.transform.rot % 2 == 1 { [doc.size[1], doc.size[0]] } else { doc.size };
    let img = if doc.crop.size() == Vec2::new(w as f32, h as f32) && doc.crop.min == doc.crop.min.round() {
        // Same scale: copy the pixels exactly instead of resampling.
        let img = to_dynamic(frame.alpha, frame.w, frame.h, frame.pixels.clone()).ok_or("bad frame")?;
        img.crop_imm(doc.crop.min.x as u32, doc.crop.min.y as u32, w, h)
    } else {
        let pixels = resize(frame, doc.crop, w, h, ResizeAlg::Convolution(FilterType::Lanczos3));
        to_dynamic(frame.alpha, w, h, pixels).ok_or("bad frame")?
    };
    Ok(doc.transform.apply(img))
}

/// The Resize dialog's state.
pub struct ResizeDialog {
    /// The size before resizing, which the percentage is relative to.
    base: [u32; 2],
    w: u32,
    h: u32,
    keep_aspect: bool,
    percent: f32,
    dpi: u16,
    dpi_before: u16,
}

impl ResizeDialog {
    pub fn new(doc: &Doc) -> Self {
        let dpi = doc.dpi.unwrap_or(72);
        ResizeDialog { base: doc.size, w: doc.size[0], h: doc.size[1], keep_aspect: true, percent: 100.0, dpi, dpi_before: dpi }
    }

    /// The new size and DPI, from the dialog's values.
    pub fn result(&self) -> ([u32; 2], Option<u16>) {
        ([self.w, self.h], (self.dpi != self.dpi_before).then_some(self.dpi))
    }

    fn set_width(&mut self, w: u32) {
        self.w = w.max(1);
        if self.keep_aspect {
            self.h = ((self.w as f64 * self.base[1] as f64 / self.base[0] as f64).round() as u32).max(1);
        }
        self.percent = self.w as f32 / self.base[0] as f32 * 100.0;
    }

    fn set_height(&mut self, h: u32) {
        self.h = h.max(1);
        if self.keep_aspect {
            self.w = ((self.h as f64 * self.base[0] as f64 / self.base[1] as f64).round() as u32).max(1);
        }
        self.percent = self.h as f32 / self.base[1] as f32 * 100.0;
    }

    fn set_percent(&mut self, percent: f32) {
        self.percent = percent;
        self.w = ((self.base[0] as f32 * percent / 100.0).round() as u32).max(1);
        self.h = ((self.base[1] as f32 * percent / 100.0).round() as u32).max(1);
    }
}

/// The Resize dialog body: Some(true) to resize, Some(false) to cancel.
pub fn resize_ui(ui: &mut egui::Ui, d: &mut ResizeDialog) -> Option<bool> {
    ui.heading("Resize");
    egui::Grid::new("resize-grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
        let (mut w, mut h, mut percent) = (d.w, d.h, d.percent);
        ui.label("Width");
        if ui.add(egui::DragValue::new(&mut w).range(1..=65535).suffix(" px")).changed() {
            d.set_width(w);
        }
        ui.end_row();
        ui.label("Height");
        if ui.add(egui::DragValue::new(&mut h).range(1..=65535).suffix(" px")).changed() {
            d.set_height(h);
        }
        ui.end_row();
        ui.label("");
        if ui.checkbox(&mut d.keep_aspect, "Keep aspect ratio").changed() && d.keep_aspect {
            d.set_width(d.w);
        }
        ui.end_row();
        ui.label("Percentage");
        if ui.add(egui::DragValue::new(&mut percent).range(1.0..=1000.0).speed(0.5).suffix(" %")).changed() {
            d.set_percent(percent);
        }
        ui.end_row();
        ui.label("Resolution");
        ui.add(egui::DragValue::new(&mut d.dpi).range(1..=4800).suffix(" dpi")).on_hover_text("Saved in JPEG files");
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let ok = ui.button("Resize").clicked();
        let cancel = ui.button("Cancel").clicked();
        ok.then_some(true).or(cancel.then_some(false))
    }).inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 6×5 RGB frame, red channel = pixel index.
    fn frame() -> Frame {
        Frame { w: 6, h: 5, pixels: (0..30u8).flat_map(|i| [i, 0, 0]).collect(), alpha: false, delay: Duration::ZERO }
    }

    fn reference() -> DynamicImage {
        to_dynamic(false, 6, 5, frame().pixels).unwrap()
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
    }

    #[test]
    fn no_edits_is_the_original() {
        assert_eq!(render(&frame(), &Doc::new(6, 5)).unwrap(), reference());
    }

    #[test]
    fn edit_sequences_match_image_ops() {
        type Step = (fn(&mut Doc), fn(DynamicImage) -> DynamicImage);
        let steps: [Step; 5] = [
            (Doc::rotate_right, |i| i.rotate90()),
            (Doc::rotate_left, |i| i.rotate270()),
            (Doc::flip_horizontal, |i| i.fliph()),
            (Doc::flip_vertical, |i| i.flipv()),
            // Crop off the first row and column of whatever is shown.
            (|d| d.crop_to(rect(1.0, 1.0, d.size[0] as f32 - 1.0, d.size[1] as f32 - 1.0)), |i| i.crop_imm(1, 1, i.width() - 1, i.height() - 1)),
        ];
        for a in 0..5 {
            for b in 0..5 {
                for c in 0..5 {
                    let (mut doc, mut expected) = (Doc::new(6, 5), reference());
                    for i in [a, b, c] {
                        steps[i].0(&mut doc);
                        expected = steps[i].1(expected);
                    }
                    assert_eq!(render(&frame(), &doc).unwrap().to_rgb8(), expected.to_rgb8(), "steps {a} {b} {c} → {doc:?}");
                }
            }
        }
    }

    #[test]
    fn crop_after_resize_keeps_the_scale() {
        let mut doc = Doc::new(6, 5);
        doc.resize(12, 10);
        doc.crop_to(rect(2.0, 2.0, 4.0, 2.0));
        assert_eq!(doc.size, [4, 2]);
        assert_eq!(doc.crop, rect(1.0, 1.0, 2.0, 1.0));
        let out = render(&frame(), &doc).unwrap();
        assert_eq!((out.width(), out.height()), (4, 2));
    }

    #[test]
    fn resize_dialog_keeps_aspect() {
        let mut d = ResizeDialog::new(&Doc::new(4000, 3000));
        d.set_width(400);
        assert_eq!(d.result(), ([400, 300], None));
        d.set_percent(50.0);
        assert_eq!(d.result().0, [2000, 1500]);
        d.keep_aspect = false;
        d.set_height(10);
        assert_eq!(d.result().0, [2000, 10]);
        d.dpi = 300;
        assert_eq!(d.result().1, Some(300));
    }

    #[test]
    fn crop_is_clamped_and_rounded() {
        let mut doc = Doc::new(6, 5);
        doc.crop_to(rect(-5.0, 0.4, 100.0, 100.0));
        assert_eq!(doc.crop, rect(0.0, 0.0, 6.0, 5.0));
        let before = doc.clone();
        doc.crop_to(rect(1.0, 1.0, 0.2, 0.2));
        assert_eq!(doc, before, "an empty crop does nothing");
    }
}
