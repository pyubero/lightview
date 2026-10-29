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
    pub adjust: Adjust,
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
        Doc { crop: Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32)), transform: Transform::default(), size: [w, h], dpi: None, adjust: Adjust::default() }
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
    let mut img = doc.transform.apply(img);
    match &mut img {
        DynamicImage::ImageRgba8(i) => doc.adjust.apply(i, 4),
        DynamicImage::ImageRgb8(i) => doc.adjust.apply(i, 3),
        _ => unreachable!("to_dynamic only makes RGB(A)"),
    }
    Ok(img)
}

/// The Adjust Color panel body: sliders that change `a` live. Returns Some(true) for Done,
/// Some(false) for Cancel; `auto_levels` is called for the Auto Levels button.
pub fn adjust_ui(ui: &mut egui::Ui, a: &mut Adjust, auto_levels: impl FnOnce() -> [[u8; 2]; 3]) -> Option<bool> {
    egui::Grid::new("adjust-grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
        for (label, value, range) in [
            ("Exposure", &mut a.exposure, -3.0..=3.0),
            ("Contrast", &mut a.contrast, -1.0..=1.0),
            ("Saturation", &mut a.saturation, -1.0..=1.0),
            ("Tint", &mut a.tint, -1.0..=1.0),
        ] {
            ui.label(label);
            ui.add(egui::Slider::new(value, range).fixed_decimals(2));
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        let mut levels = a.levels.is_some();
        if ui.checkbox(&mut levels, "Auto Levels").changed() {
            a.levels = levels.then(auto_levels);
        }
        if ui.button("Reset").clicked() {
            *a = Adjust::default();
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let done = ui.button("Done").clicked();
        let cancel = ui.button("Cancel").clicked();
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
