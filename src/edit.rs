use crate::i18n::tr;
use crate::load::Frame;
use crate::markup::{self, Element};
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
    pub adjust: Adjust,
    /// Markup, in output px; flattened into the image when saving.
    pub elements: Vec<Element>,
}

/// Colour adjustments. All zero (the default) changes nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Adjust {
    /// In stops: −3 to 3.
    pub exposure: f32,
    /// −1 (flat grey) to 1 (twice the contrast).
    pub contrast: f32,
    /// −1 (greyscale) to 1 (twice as saturated).
    pub saturation: f32,
    /// −1 (green) to 1 (magenta).
    pub tint: f32,
    /// Per channel, the input values that become black and white (Auto Levels).
    pub levels: Option<[[u8; 2]; 3]>,
}

impl Adjust {
    pub fn is_identity(&self) -> bool {
        *self == Adjust::default()
    }

    /// Adjusts RGB (`channels` 3) or RGBA (4) pixels in place.
    pub fn apply(&self, pixels: &mut [u8], channels: usize) {
        if self.is_identity() {
            return;
        }
        let gain = 2f32.powf(self.exposure);
        let tint = [1.0 + self.tint * 0.1, 1.0 - self.tint * 0.1, 1.0 + self.tint * 0.1];
        // Levels, exposure, tint and contrast are per channel, so they fit in lookup tables.
        let luts: [[u8; 256]; 3] = std::array::from_fn(|c| {
            let [lo, hi] = self.levels.map_or([0, 255], |l| l[c]);
            std::array::from_fn(|v| {
                let x = (v as f32 - lo as f32) / (hi as f32 - lo as f32).max(1.0) * gain * tint[c];
                (((x - 0.5) * (1.0 + self.contrast) + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8
            })
        });
        // Fixed point (×256), so the per-pixel loop stays cheap enough for live sliders.
        let sat = ((1.0 + self.saturation) * 256.0).round() as i32;
        let pass = |px: &mut [u8]| {
            let [r, g, b] = [luts[0][px[0] as usize], luts[1][px[1] as usize], luts[2][px[2] as usize]];
            if sat == 256 {
                (px[0], px[1], px[2]) = (r, g, b);
            } else {
                let l = (77 * r as i32 + 150 * g as i32 + 29 * b as i32) >> 8;
                let mix = |v: u8| (l + (((v as i32 - l) * sat + 128) >> 8)).clamp(0, 255) as u8;
                (px[0], px[1], px[2]) = (mix(r), mix(g), mix(b));
            }
        };
        // Split across cores: a whole-screen image is a few million pixels.
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let part = (pixels.len() / channels).div_ceil(threads).max(1) * channels;
        std::thread::scope(|s| {
            for part in pixels.chunks_mut(part) {
                s.spawn(move || part.chunks_exact_mut(channels).for_each(pass));
            }
        });
    }
}

/// Auto Levels: per channel, the values that clip 0.5% of the `crop` part of `frame` to black and to white.
pub fn auto_levels(frame: &Frame, crop: Rect) -> [[u8; 2]; 3] {
    let channels = if frame.alpha { 4 } else { 3 };
    let mut hist = [[0u64; 256]; 3];
    let (x0, x1) = (crop.min.x.max(0.0) as usize, (crop.max.x as usize).min(frame.w as usize));
    for y in crop.min.y.max(0.0) as usize..(crop.max.y as usize).min(frame.h as usize) {
        let row = &frame.pixels[(y * frame.w as usize + x0) * channels..(y * frame.w as usize + x1) * channels];
        for px in row.chunks_exact(channels) {
            for c in 0..3 {
                hist[c][px[c] as usize] += 1;
            }
        }
    }
    hist.map(|h| {
        let clip = h.iter().sum::<u64>() / 200;
        let find = |values: Vec<usize>| {
            let mut sum = 0;
            values.into_iter().find(|&v| {
                sum += h[v];
                sum > clip
            }).unwrap_or(0) as u8
        };
        let (lo, hi) = (find((0..256).collect()), find((0..256).rev().collect()));
        if hi > lo { [lo, hi] } else { [0, 255] }
    })
}

impl Doc {
    /// No edits on a `w`×`h` image.
    pub fn new(w: u32, h: u32) -> Self {
        Doc { crop: Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32)), transform: Transform::default(), size: [w, h], dpi: None, adjust: Adjust::default(), elements: Vec::new() }
    }

    pub fn size_vec(&self) -> Vec2 {
        Vec2::new(self.size[0] as f32, self.size[1] as f32)
    }

    pub fn bounds(&self) -> Rect {
        Rect::from_min_size(Pos2::ZERO, self.size_vec())
    }

    /// The crop and the transform: what decides the view's layout.
    pub fn geometry(&self) -> (Rect, Transform, [u32; 2]) {
        (self.crop, self.transform, self.size)
    }

    /// The output size before the transform.
    pub fn plain_size(&self) -> [u32; 2] {
        if self.transform.rot % 2 == 1 { [self.size[1], self.size[0]] } else { self.size }
    }

    /// Whether the output has fewer pixels than the source on some axis.
    pub fn shrinks(&self) -> bool {
        self.scale().min_elem() < 1.0
    }

    /// This doc on the pixels from `bake`, which already hold the crop and the size.
    pub fn on_baked(&self) -> Doc {
        let [w, h] = self.plain_size();
        Doc { crop: Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32)), ..self.clone() }
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
        // Float rounding can land a hair outside the crop (and the image), which the resizer rejects.
        self.transform.to_source(scaled, self.crop.size()).translate(self.crop.min.to_vec2()).intersect(self.crop)
    }

    /// Moves the markup along with a geometric edit.
    /// shortcut: text and pasted images move but don't turn with a rotate/flip; turn them too if that looks wrong.
    fn map_elements(&mut self, f: impl Fn(Pos2) -> Pos2, scale: f32) {
        for e in &mut self.elements {
            e.map(&f, scale);
        }
    }

    pub fn rotate_right(&mut self) {
        let h = self.size_vec().y;
        self.map_elements(|p| Pos2::new(h - p.y, p.x), 1.0);
        self.transform = self.transform.rotate_right();
        self.size = [self.size[1], self.size[0]];
    }

    pub fn rotate_left(&mut self) {
        let w = self.size_vec().x;
        self.map_elements(|p| Pos2::new(p.y, w - p.x), 1.0);
        self.transform = self.transform.rotate_left();
        self.size = [self.size[1], self.size[0]];
    }

    pub fn flip_horizontal(&mut self) {
        let w = self.size_vec().x;
        self.map_elements(|p| Pos2::new(w - p.x, p.y), 1.0);
        self.transform = self.transform.flip_horizontal();
    }

    pub fn flip_vertical(&mut self) {
        let h = self.size_vec().y;
        self.map_elements(|p| Pos2::new(p.x, h - p.y), 1.0);
        self.transform = self.transform.flip_vertical();
    }

    /// Crops to `r` (output px, rounded to whole pixels), keeping the pixel scale.
    pub fn crop_to(&mut self, r: Rect) {
        let r = Rect::from_min_max(r.min.round(), r.max.round()).intersect(self.bounds());
        if r.width() < 1.0 || r.height() < 1.0 {
            return;
        }
        self.crop = self.to_source(r);
        self.size = [r.width() as u32, r.height() as u32];
        self.map_elements(|p| p - r.min.to_vec2(), 1.0);
    }

    /// ΔX, ΔY and the distance from `a` to `b` (output px), in the original image's pixels
    /// but along the axes as shown.
    pub fn measure(&self, a: Pos2, b: Pos2) -> [f32; 3] {
        let d = self.source_point(b) - self.source_point(a);
        let (dx, dy) = if self.transform.rot % 2 == 1 { (d.y, d.x) } else { (d.x, d.y) };
        [dx.abs(), dy.abs(), d.length()]
    }

    /// Scales the output to `w`×`h` px.
    pub fn resize(&mut self, w: u32, h: u32) {
        let old = self.size_vec();
        self.size = [w.max(1), h.max(1)];
        let scale = self.size_vec() / old;
        self.map_elements(|p| Pos2::new(p.x * scale.x, p.y * scale.y), (scale.x * scale.y).sqrt());
    }
}

/// Resizes the `crop` part of `frame` to `w`×`h` px (before any transform).
pub fn resize(frame: &Frame, crop: Rect, w: u32, h: u32, alg: ResizeAlg) -> Vec<u8> {
    let pixel_type = if frame.alpha { PixelType::U8x4 } else { PixelType::U8x3 };
    let src = ImageRef::new(frame.w, frame.h, &frame.pixels, pixel_type).unwrap();
    let mut dst = Image::new(w, h, pixel_type);
    let (left, width) = crop_axis(crop.min.x, crop.max.x, frame.w);
    let (top, height) = crop_axis(crop.min.y, crop.max.y, frame.h);
    let opts = ResizeOptions::new().resize_alg(alg).crop(left, top, width, height);
    Resizer::new().resize(&src, &mut dst, &opts).unwrap();
    dst.into_vec()
}

/// `frame` resized as `render` does it, so a shrunk image shows what saving it would write.
pub fn bake(frame: &Frame, doc: &Doc) -> Frame {
    let [w, h] = doc.plain_size();
    let pixels = resize(frame, doc.crop, w, h, ResizeAlg::Convolution(FilterType::Lanczos3));
    Frame { w, h, pixels, alpha: frame.alpha, delay: frame.delay }
}

/// One axis of a crop, as the resizer wants it: start and length inside `0..len`. The resizer
/// adds them up itself and refuses anything past the edge, and float rounding can put a crop that
/// ends at the edge a hair past it, so the length is trimmed until the sum fits.
fn crop_axis(min: f32, max: f32, len: u32) -> (f64, f64) {
    let len = len as f64;
    let start = (min as f64).clamp(0.0, (len - 1.0).max(0.0));
    let mut size = ((max as f64).min(len) - start).max(0.0);
    while start + size > len {
        size = size.next_down();
    }
    (start, size)
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
    let [w, h] = doc.plain_size();
    let img = if doc.crop.size() == Vec2::new(w as f32, h as f32) && doc.crop.min == doc.crop.min.round() {
        // Same scale: copy the pixels exactly instead of resampling.
        let img = to_dynamic(frame.alpha, frame.w, frame.h, frame.pixels.clone()).ok_or("bad frame")?;
        img.crop_imm(doc.crop.min.x as u32, doc.crop.min.y as u32, w, h)
    } else {
        let pixels = resize(frame, doc.crop, w, h, ResizeAlg::Convolution(FilterType::Lanczos3));
        to_dynamic(frame.alpha, w, h, pixels).ok_or("bad frame")?
    };
    let mut img = doc.transform.apply(img);
    match &mut img {
        DynamicImage::ImageRgba8(i) => doc.adjust.apply(i, 4),
        DynamicImage::ImageRgb8(i) => doc.adjust.apply(i, 3),
        _ => unreachable!("to_dynamic only makes RGB(A)"),
    }
    if !doc.elements.is_empty() {
        let mut rgba = img.into_rgba8();
        markup::flatten(&mut rgba, &doc.elements);
        // Erasing to transparent (background removal) gives an opaque image an alpha channel.
        let alpha = frame.alpha || doc.elements.iter().any(|e| e.shape == markup::Shape::Erase && e.style.color.a() == 0);
        img = if alpha { DynamicImage::ImageRgba8(rgba) } else { DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba).into_rgb8()) };
    }
    Ok(img)
}

/// The Adjust Color panel body: sliders that change `a` live. Returns Some(true) for Done,
/// Some(false) for Cancel; `auto_levels` is called for the Auto Levels button.
pub fn adjust_ui(ui: &mut egui::Ui, a: &mut Adjust, auto_levels: impl FnOnce() -> [[u8; 2]; 3]) -> Option<bool> {
    egui::Grid::new("adjust-grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
        for (label, value, range) in [
            (tr("Exposure"), &mut a.exposure, -3.0..=3.0),
            (tr("Contrast"), &mut a.contrast, -1.0..=1.0),
            (tr("Saturation"), &mut a.saturation, -1.0..=1.0),
            (tr("Tint"), &mut a.tint, -1.0..=1.0),
        ] {
            ui.label(label);
            ui.add(egui::Slider::new(value, range).fixed_decimals(2));
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        let mut levels = a.levels.is_some();
        if ui.checkbox(&mut levels, tr("Auto Levels")).changed() {
            a.levels = levels.then(auto_levels);
        }
        if ui.button(tr("Reset")).clicked() {
            *a = Adjust::default();
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let done = ui.button(tr("Done")).clicked();
        let cancel = ui.button(tr("Cancel")).clicked();
        done.then_some(true).or(cancel.then_some(false))
    }).inner
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
    ui.heading(tr("Resize"));
    egui::Grid::new("resize-grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
        let (mut w, mut h, mut percent) = (d.w, d.h, d.percent);
        ui.label(tr("Width"));
        if ui.add(egui::DragValue::new(&mut w).range(1..=65535).suffix(" px")).changed() {
            d.set_width(w);
        }
        ui.end_row();
        ui.label(tr("Height"));
        if ui.add(egui::DragValue::new(&mut h).range(1..=65535).suffix(" px")).changed() {
            d.set_height(h);
        }
        ui.end_row();
        ui.label("");
        if ui.checkbox(&mut d.keep_aspect, tr("Keep aspect ratio")).changed() && d.keep_aspect {
            d.set_width(d.w);
        }
        ui.end_row();
        ui.label(tr("Percentage"));
        if ui.add(egui::DragValue::new(&mut percent).range(1.0..=1000.0).speed(0.5).suffix(" %")).changed() {
            d.set_percent(percent);
        }
        ui.end_row();
        ui.label(tr("Resolution"));
        ui.add(egui::DragValue::new(&mut d.dpi).range(1..=4800).suffix(" dpi")).on_hover_text(tr("Saved in JPEG files"));
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let ok = ui.button(tr("Resize")).clicked();
        let cancel = ui.button(tr("Cancel")).clicked();
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
    fn markup_follows_geometric_edits() {
        use crate::markup::{Shape, Style};
        // A dot drawn on pixel (1, 0) of the 6×5 frame stays on that image pixel through any edit.
        let mut doc = Doc::new(6, 5);
        let mut dot = Element::new(Shape::Line, Pos2::new(1.2, 0.5), Style { width: 0.5, ..Style::for_image(Vec2::splat(6.0)) });
        dot.points[1] = Pos2::new(1.8, 0.5);
        doc.elements.push(dot);
        let marked = |doc: &Doc| {
            let px = |e: &Element| e.points[0].lerp(e.points[1], 0.5);
            doc.source_point(px(&doc.elements[0]))
        };
        type Step = fn(&mut Doc);
        let steps: [Step; 5] = [Doc::rotate_right, Doc::rotate_left, Doc::flip_horizontal, Doc::flip_vertical, |d| d.resize(d.size[0] * 2, d.size[1] * 3)];
        for a in 0..5 {
            for b in 0..5 {
                let mut d = doc.clone();
                steps[a](&mut d);
                steps[b](&mut d);
                assert!((marked(&d) - Pos2::new(1.5, 0.5)).length() < 1e-3, "steps {a} {b}: {:?}", marked(&d));
            }
        }
        let mut d = doc.clone();
        d.crop_to(rect(1.0, 0.0, 3.0, 3.0));
        assert!((marked(&d) - Pos2::new(1.5, 0.5)).length() < 1e-3);
        // And it's flattened into the saved image (the frame is dark, the default colour is red).
        let out = render(&frame(), &doc).unwrap().to_rgb8();
        assert!(out.get_pixel(1, 0)[0] > 100);
    }

    #[test]
    fn image_from_elliptical_selection() {
        use crate::markup::Selection;
        use eframe::egui::Color32;
        // What Create Image from Selection does, on an image with alpha.
        let frame = Frame { w: 40, h: 40, pixels: vec![200; 40 * 40 * 4], alpha: true, delay: Duration::ZERO };
        let sel = Selection::Ellipse(rect(10.0, 10.0, 20.0, 20.0));
        let mut doc = Doc::new(40, 40);
        let r = sel.bbox();
        doc.crop_to(r);
        doc.elements.push(Element::erase(&sel.translate(-r.min.to_vec2()), true, rect(0.0, 0.0, 20.0, 20.0), Color32::TRANSPARENT));
        let out = render(&frame, &doc).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (20, 20));
        assert_eq!(out.get_pixel(0, 0)[3], 0, "corner outside the ellipse is transparent");
        assert_eq!(out.get_pixel(10, 10).0, [200; 4], "centre kept");
    }

    #[test]
    fn resize_never_rejects_a_crop_at_the_edge() {
        // The visible area reaches the right edge; passed on as start + width (as the display does),
        // float rounding could put the end a hair past the image, which the resizer refuses.
        let frame = Frame { w: 4000, h: 3, pixels: vec![0; 4000 * 3 * 3], alpha: false, delay: Duration::ZERO };
        let mut seed = 12345u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        for _ in 0..400 {
            let edge = Rect::from_min_max(Pos2::new(next() * 50.0, 0.0), Pos2::new(4000.0 - next() * 1e-3, 3.0));
            let passed_on = Rect::from_min_size(edge.min, edge.size());
            resize(&frame, passed_on, 8, 1, ResizeAlg::Nearest);
        }
        // A rect slightly outside on every side (from rounding) is clamped, not refused.
        resize(&frame, Rect::from_min_max(Pos2::new(-0.001, -0.001), Pos2::new(4000.001, 3.001)), 8, 1, ResizeAlg::Nearest);
    }

    #[test]
    fn to_source_stays_inside_the_image() {
        use crate::view::{self, View};
        // Maximizing gives new viewport sizes and scales; float rounding in the mapping must never
        // reach outside the decoded image, which fast_image_resize rejects (it used to panic).
        let mut failures = Vec::new();
        for (w, h) in [(4000u32, 2252u32), (4032, 2268), (4000, 3000), (2268, 4032), (7, 3), (6000, 4000)] {
            let edits: [fn(&mut Doc); 7] = [
                |_| {},
                Doc::rotate_left,
                Doc::rotate_right,
                Doc::flip_horizontal,
                |d| d.resize(d.size[0] / 3 + 1, d.size[1] / 3 + 1),
                |d| d.resize(1999, 1125),
                |d| {
                    d.resize(d.size[0] * 2 / 3 + 1, d.size[1] * 2 / 3 + 1);
                    d.crop_to(Rect::from_min_size(Pos2::new(1.0, 1.0), d.size_vec() / 2.0));
                    d.rotate_right();
                },
            ];
            for (e, edit) in edits.iter().enumerate() {
                let mut doc = Doc::new(w, h);
                edit(&mut doc);
                let frame = Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32));
                let img = doc.size_vec();
                for (vw, vh) in [(1280.0, 694.0), (1920.0, 1053.0), (2560.0, 1371.0), (1917.6, 1043.2), (1535.2, 822.4)] {
                    for ppp in [1.0, 1.25, 1.5, 2.0] {
                        let vp = Rect::from_min_size(Pos2::new(0.0, 23.2), Vec2::new(vw, vh));
                        for view in [View::Fit, view::actual_size(img), view::zoom_at(View::Fit, 3.7, Pos2::new(vw * 0.95, vh * 0.9), img, vp, ppp)] {
                            let (_, crop) = view::layout(view, img, vp, ppp);
                            let src = doc.to_source(crop);
                            if !frame.contains_rect(src) {
                                failures.push(format!("{w}×{h} edit {e} {vw}×{vh}@{ppp} {view:?}: {src:?}"));
                            }
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{} cases outside the image, e.g. {:?}", failures.len(), &failures[..failures.len().min(3)]);
        // And fast_image_resize itself accepts the mapped rect (a small image, so it's quick).
        let mut doc = Doc::new(7, 3);
        doc.resize(5, 2);
        doc.rotate_right();
        let (_, crop) = view::layout(View::Fit, doc.size_vec(), Rect::from_min_size(Pos2::ZERO, Vec2::new(1917.6, 1043.2)), 1.25);
        let small = Frame { w: 7, h: 3, pixels: vec![0; 7 * 3 * 3], alpha: false, delay: Duration::ZERO };
        resize(&small, doc.to_source(crop), 10, 10, ResizeAlg::Nearest);
    }

    #[test]
    fn upscaled_result_covers_the_bigger_image() {
        // What applying a 2× Enhance does: the doc grows, and the result covers it at full size.
        use crate::markup::Style;
        use std::sync::Arc;
        let mut doc = Doc::new(6, 5);
        doc.resize(12, 10);
        let result = image::RgbaImage::from_fn(12, 10, |x, y| image::Rgba([(x * 20) as u8, (y * 20) as u8, 7, 255]));
        doc.elements.push(Element::pasted(Arc::new(result.clone()), Pos2::ZERO, Vec2::new(12.0, 10.0), Style::for_image(Vec2::new(12.0, 10.0))));
        let out = render(&frame(), &doc).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (12, 10));
        let worst = out.as_raw().iter().zip(result.as_raw()).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 1, "{worst}");
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
    fn measure_in_original_pixels() {
        use crate::view::{self, View};
        let mut doc = Doc::new(4000, 3000);
        let (a, b) = (Pos2::new(100.0, 200.0), Pos2::new(400.0, 600.0));
        assert_eq!(doc.measure(a, b), [300.0, 400.0, 500.0]);
        // The same image points picked on screen at any zoom measure the same.
        let vp = rect(0.0, 0.0, 1280.0, 720.0);
        for view in [View::Fit, view::actual_size(doc.size_vec()), view::zoom_at(View::Fit, 7.0, Pos2::new(300.0, 100.0), doc.size_vec(), vp, 1.25)] {
            let (dest, crop) = view::layout(view, doc.size_vec(), vp, 1.25);
            let pick = |p| view::to_image(dest, crop, view::to_screen(dest, crop, p));
            let [dx, dy, d] = doc.measure(pick(a), pick(b));
            assert!((dx - 300.0).abs() < 0.05 && (dy - 400.0).abs() < 0.05 && (d - 500.0).abs() < 0.05, "{view:?}");
        }
        // Rotated: the same two image features are now 400 apart horizontally, 300 vertically.
        doc.rotate_right();
        let (ra, rb) = (Pos2::new(3000.0 - 200.0, 100.0), Pos2::new(3000.0 - 600.0, 400.0));
        assert_eq!(doc.measure(ra, rb), [400.0, 300.0, 500.0]);
        // Resized to half: still measured in original pixels.
        doc.resize(1500, 2000);
        assert_eq!(doc.measure((ra.to_vec2() / 2.0).to_pos2(), (rb.to_vec2() / 2.0).to_pos2()), [400.0, 300.0, 500.0]);
    }

    #[test]
    fn adjustments() {
        let grey = |v: u8| vec![v, v, v];
        let adjusted = |a: Adjust, mut px: Vec<u8>| {
            a.apply(&mut px, 3);
            px
        };
        assert_eq!(adjusted(Adjust::default(), vec![1, 2, 3]), [1, 2, 3]);
        assert_eq!(adjusted(Adjust { exposure: 1.0, ..Adjust::default() }, grey(64)), grey(128));
        assert_eq!(adjusted(Adjust { contrast: -1.0, ..Adjust::default() }, vec![0, 90, 255]), grey(128));
        let desaturated = adjusted(Adjust { saturation: -1.0, ..Adjust::default() }, vec![200, 100, 50]);
        assert!(desaturated[0] == desaturated[1] && desaturated[1] == desaturated[2]);
        let magenta = adjusted(Adjust { tint: 1.0, ..Adjust::default() }, grey(128));
        assert!(magenta[0] > magenta[1] && magenta[2] > magenta[1]);
        // RGBA: alpha is left alone.
        assert_eq!(adjusted4(Adjust { exposure: 1.0, ..Adjust::default() }, vec![64, 64, 64, 7]), [128, 128, 128, 7]);
        fn adjusted4(a: Adjust, mut px: Vec<u8>) -> Vec<u8> {
            a.apply(&mut px, 4);
            px
        }
    }

    #[test]
    fn auto_levels_stretches() {
        // Values 50..=200 on every channel.
        let pixels: Vec<u8> = (0..151u8).flat_map(|i| [50 + i; 3]).collect();
        let frame = Frame { w: 151, h: 1, pixels, alpha: false, delay: Duration::ZERO };
        let levels = auto_levels(&frame, rect(0.0, 0.0, 151.0, 1.0));
        assert_eq!(levels, [[50, 200]; 3]);
        let a = Adjust { levels: Some(levels), ..Adjust::default() };
        let mut px = vec![50, 125, 200];
        a.apply(&mut px, 3);
        assert_eq!(px, [0, 128, 255]);
        // A flat image keeps the full range.
        let flat = Frame { w: 2, h: 1, pixels: vec![9; 6], alpha: false, delay: Duration::ZERO };
        assert_eq!(auto_levels(&flat, rect(0.0, 0.0, 2.0, 1.0)), [[0, 255]; 3]);
    }

    #[test]
    fn render_applies_adjustments() {
        let doc = Doc { adjust: Adjust { saturation: -1.0, ..Adjust::default() }, ..Doc::new(6, 5) };
        let out = render(&frame(), &doc).unwrap().to_rgb8();
        assert!(out.pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
    }

    /// Run with `cargo test --release -- --ignored --nocapture`: what one slider step costs on screen.
    #[test]
    #[ignore]
    fn adjust_speed() {
        let big = Frame { w: 4000, h: 3000, pixels: (0..4000 * 3000 * 3).map(|i| (i % 251) as u8).collect(), alpha: false, delay: Duration::ZERO };
        let a = Adjust { exposure: 0.5, contrast: 0.2, saturation: 0.3, tint: 0.1, levels: None };
        // The app keeps the resized pixels, so a slider step only re-runs the adjustment.
        let px = resize(&big, rect(0.0, 0.0, 4000.0, 3000.0), 1280, 960, ResizeAlg::Convolution(FilterType::Bilinear));
        let t = std::time::Instant::now();
        for _ in 0..10 {
            a.apply(&mut px.clone(), 3);
        }
        eprintln!("adjust 1280×960: {:.1} ms per step", t.elapsed().as_secs_f64() * 100.0);
    }

    #[test]
    fn baked_frame_renders_like_the_original() {
        for (alpha, channels) in [(false, 3), (true, 4)] {
            let frame = Frame { w: 40, h: 30, pixels: (0..40 * 30 * channels).map(|i| (i * 7 % 251) as u8).collect(), alpha, delay: Duration::ZERO };
            let edits: [fn(&mut Doc); 3] = [
                |d| d.resize(5, 5),
                |d| {
                    d.rotate_right();
                    d.flip_horizontal();
                    d.resize(9, 20);
                },
                |d| {
                    d.resize(20, 15);
                    d.crop_to(rect(3.0, 2.0, 16.0, 12.0));
                },
            ];
            for edit in edits {
                let mut doc = Doc::new(40, 30);
                edit(&mut doc);
                assert!(doc.shrinks());
                let baked = bake(&frame, &doc);
                assert_eq!([baked.w, baked.h], doc.plain_size());
                let (shown, saved) = (render(&baked, &doc.on_baked()).unwrap(), render(&frame, &doc).unwrap());
                assert_eq!(shown.as_bytes(), saved.as_bytes());
            }
        }
    }

    #[test]
    fn only_shrinking_shrinks() {
        let mut doc = Doc::new(40, 30);
        assert!(!doc.shrinks());
        doc.rotate_right();
        assert!(!doc.shrinks());
        doc.resize(60, 80);
        assert!(!doc.shrinks());
        doc.resize(60, 20);
        assert!(doc.shrinks());
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
