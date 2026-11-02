use crate::curves::Curves;
use crate::i18n::tr;
use crate::load::Frame;
use crate::markup::{self, Element};
use crate::view::Transform;
use eframe::egui::{self, Pos2, Rect, Vec2};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::{Image, ImageRef}};
use image::DynamicImage;
use std::sync::Arc;

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
    /// Pixels that replace the decoded image, after a perspective correction.
    pub base: Option<Base>,
}

/// A frame held by a `Doc`. Compared by identity: Docs are compared every frame.
#[derive(Clone)]
pub struct Base(pub Arc<Frame>);

impl PartialEq for Base {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Base {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Base({}×{})", self.0.w, self.0.h)
    }
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
    /// After the above, before saturation.
    pub curves: Curves,
    /// Last: every channel becomes 255 − v.
    pub invert: bool,
}

impl Adjust {
    pub fn is_identity(&self) -> bool {
        *self == Adjust::default()
    }

    /// The sliders at zero; the curves and Invert stay, since they're set elsewhere.
    fn sliders_reset(&self) -> Adjust {
        Adjust { curves: self.curves, invert: self.invert, ..Adjust::default() }
    }

    /// Adjusts RGB (`channels` 3) or RGBA (4) pixels in place.
    pub fn apply(&self, pixels: &mut [u8], channels: usize) {
        if self.is_identity() {
            return;
        }
        let gain = 2f32.powf(self.exposure);
        let tint = [1.0 + self.tint * 0.1, 1.0 - self.tint * 0.1, 1.0 + self.tint * 0.1];
        // Exposure, tint, contrast and RGB curves are per channel, so they fit in lookup tables.
        let curves = self.curves.rgb_luts();
        let luts: [[u8; 256]; 3] = std::array::from_fn(|c| {
            std::array::from_fn(|v| {
                let x = v as f32 / 255.0 * gain * tint[c];
                curves[c][(((x - 0.5) * (1.0 + self.contrast) + 0.5).clamp(0.0, 1.0) * 255.0).round() as usize]
            })
        });
        let colors = self.curves.color_tables();
        // Fixed point (×256), so the per-pixel loop stays cheap enough for live sliders.
        let sat = ((1.0 + self.saturation) * 256.0).round() as i32;
        let flip = if self.invert { 255 } else { 0 };
        let pass = |px: &mut [u8]| {
            let rgb = [luts[0][px[0] as usize], luts[1][px[1] as usize], luts[2][px[2] as usize]];
            let [r, g, b] = colors.as_ref().map_or(rgb, |c| c.map(rgb));
            if sat == 256 {
                (px[0], px[1], px[2]) = (r ^ flip, g ^ flip, b ^ flip);
            } else {
                let l = (77 * r as i32 + 150 * g as i32 + 29 * b as i32) >> 8;
                let mix = |v: u8| (l + (((v as i32 - l) * sat + 128) >> 8)).clamp(0, 255) as u8 ^ flip;
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

/// Auto Levels: the [exposure, contrast] that stretch the `crop` part of `frame` to full range,
/// clipping 0.5% of each channel to black and to white. One stretch for all channels keeps the colours.
pub fn auto_levels(frame: &Frame, crop: Rect) -> [f32; 2] {
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
    let levels = hist.map(|h| {
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
    });
    let lo = levels.iter().map(|l| l[0]).min().unwrap_or(0) as f32 / 255.0;
    let hi = levels.iter().map(|l| l[1]).max().unwrap_or(255) as f32 / 255.0;
    // `Adjust::apply` with lo → 0 and hi → 1, solved for its gain and contrast.
    [(-(lo + hi).log2()).clamp(-3.0, 3.0), ((lo + hi) / (hi - lo) - 1.0).clamp(0.0, 1.0)]
}

/// The size of `size` turned by `degrees`'s bounding box: Straighten's new canvas.
pub fn straighten_size(size: Vec2, degrees: f32) -> Vec2 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (sin, cos) = (sin.abs(), cos.abs());
    Vec2::new(size.x * cos + size.y * sin, size.x * sin + size.y * cos)
}

/// Straighten's canvas in the unturned image (output px), clockwise from the top left: the
/// rectangle that becomes upright once the image is turned by `degrees` (clockwise on screen).
/// It encloses the whole image, so its corners lie outside it.
pub fn straighten_corners(size: Vec2, degrees: f32) -> [Pos2; 4] {
    let half = straighten_size(size, degrees) / 2.0;
    let centre = (size / 2.0).to_pos2();
    let back = egui::emath::Rot2::from_angle(-degrees.to_radians());
    [Vec2::new(-half.x, -half.y), Vec2::new(half.x, -half.y), half, Vec2::new(-half.x, half.y)].map(|c| centre + back * c)
}

/// Red eye: pixels whose red is half as much again as their green and their blue take the green
/// and blue's mean as their red, which keeps a pupil dark. Skin and yellows fail that test.
pub fn remove_red_eye(img: &mut image::RgbaImage) {
    for px in img.pixels_mut() {
        let [r, g, b, _] = px.0.map(u32::from);
        if r > 60 && 2 * r > 3 * g.max(b) {
            px[0] = ((g + b) / 2) as u8;
        }
    }
}

/// Pixelates `img` with a grid at a random offset, and gives each block its mean colour plus
/// random noise, so the same picture never pixelates the same way twice.
// shortcut: coarse colours and shapes still show (the user's choice, so it still looks pixelated); text is safer under a solid shape.
pub fn pixelate(img: &mut image::RgbaImage) {
    use std::hash::BuildHasher;
    // Keyed by the OS for each process, so this needs no extra crate.
    let random = std::collections::hash_map::RandomState::new();
    let mut n = 0u64;
    let mut next = move || {
        n += 1;
        random.hash_one(n)
    };
    let (w, h) = img.dimensions();
    let block = (w.min(h) / 8).max(4);
    let offset = [next() % block as u64, next() % block as u64].map(|o| o as u32);
    // Where blocks start: the first is cut short by the offset.
    let starts = |len: u32, offset: u32| {
        let first = if offset == 0 { block } else { offset };
        std::iter::once(0).chain((first..len).step_by(block as usize)).collect::<Vec<u32>>()
    };
    let (xs, ys) = (starts(w, offset[0]), starts(h, offset[1]));
    for (j, &y0) in ys.iter().enumerate() {
        let y1 = ys.get(j + 1).copied().unwrap_or(h);
        for (i, &x0) in xs.iter().enumerate() {
            let x1 = xs.get(i + 1).copied().unwrap_or(w);
            let mut sum = [0u64; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    for c in 0..3 {
                        sum[c] += img.get_pixel(x, y)[c] as u64;
                    }
                }
            }
            let count = ((x1 - x0) * (y1 - y0)) as u64;
            let colour: [u8; 3] = std::array::from_fn(|c| ((sum[c] / count) as i32 + (next() % 41) as i32 - 20).clamp(0, 255) as u8);
            for y in y0..y1 {
                for x in x0..x1 {
                    let px = img.get_pixel_mut(x, y);
                    px.0[..3].copy_from_slice(&colour);
                }
            }
        }
    }
}

impl Doc {
    /// No edits on a `w`×`h` image.
    pub fn new(w: u32, h: u32) -> Self {
        Doc { crop: Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32)), transform: Transform::default(), size: [w, h], dpi: None, adjust: Adjust::default(), elements: Vec::new(), base: None }
    }

    /// The pixels these edits apply to, given the `decoded` image.
    pub fn source<'a>(&'a self, decoded: &'a Frame) -> &'a Frame {
        self.base.as_ref().map_or(decoded, |b| &b.0)
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
        Doc { crop: Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32)), base: None, ..self.clone() }
    }

    /// Output px per transformed-crop px, on each axis.
    pub fn scale(&self) -> Vec2 {
        self.size_vec() / self.transform.size(self.crop.size())
    }

    /// The decoded-image point that ends up at output point `p`.
    /// These edits on pixels `k` times bigger than the decoded ones (a sharper SVG render).
    pub fn on_sharp(&self, k: u32) -> Doc {
        Doc { crop: self.crop * k as f32, ..self.clone() }
    }

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

    /// Straightens what `corners` (output px, clockwise from the top left) enclose into a
    /// rectangle that becomes the whole image. Markup is flattened into it; colour stays live.
    /// With no `fill` the corners are kept inside the image; with one they may lie outside it,
    /// and what falls outside gets that colour (transparent if the image has alpha).
    pub fn perspective(&mut self, decoded: &Frame, corners: [Pos2; 4], fill: Option<[u8; 3]>) -> Result<(), String> {
        let bounds = self.bounds();
        let corners = if fill.is_some() { corners } else { corners.map(|p| bounds.clamp(p)) };
        if !is_convex(corners) {
            return Err(tr("The corners must form a shape with no crossing sides").into());
        }
        let shown = flat(decoded, &Doc { adjust: Adjust::default(), ..self.clone() })?;
        let warped = warp(&shown, corners, fill.unwrap_or_default());
        let new = Doc::new(warped.w, warped.h);
        *self = Doc { adjust: self.adjust, dpi: self.dpi, base: Some(Base(Arc::new(warped))), ..new };
        Ok(())
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

/// Maps the unit square to the quad `q`, clockwise from (0, 0) (Heckbert's closed form).
pub fn homography(q: [Pos2; 4]) -> impl Fn(f64, f64) -> [f64; 2] + Copy + Send {
    let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = q.map(|p| (p.x as f64, p.y as f64));
    let (dx1, dx2, sx) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
    let (dy1, dy2, sy) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
    let det = dx1 * dy2 - dx2 * dy1;
    let (g, h) = ((sx * dy2 - dx2 * sy) / det, (dx1 * sy - sx * dy1) / det);
    let (a, b, d, e) = (x1 - x0 + g * x1, x3 - x0 + h * x3, y1 - y0 + g * y1, y3 - y0 + h * y3);
    move |u, v| {
        let w = g * u + h * v + 1.0;
        [(a * u + b * v + x0) / w, (d * u + e * v + y0) / w]
    }
}

/// Whether the quad's corners turn the same way all round: not crossed, nor collapsed to a line.
pub fn is_convex(q: [Pos2; 4]) -> bool {
    let turns: [f32; 4] = std::array::from_fn(|i| (q[(i + 1) % 4] - q[i]).x * (q[(i + 2) % 4] - q[(i + 1) % 4]).y - (q[(i + 1) % 4] - q[i]).y * (q[(i + 2) % 4] - q[(i + 1) % 4]).x);
    turns.iter().all(|t| *t > 0.0) || turns.iter().all(|t| *t < 0.0)
}

/// The straightened size of quad `q`: its longer edge each way.
// shortcut: edge lengths, not the true aspect ratio (that needs the focal length); use Resize if it looks off.
fn warped_size(q: [Pos2; 4]) -> [u32; 2] {
    let len = |a: usize, b: usize| q[a].distance(q[b]);
    [len(0, 1).max(len(3, 2)).round().max(1.0) as u32, len(0, 3).max(len(1, 2)).round().max(1.0) as u32]
}

/// Bilinear sample of `frame` at (x, y), in pixel-centre coordinates, into `out`.
fn sample(frame: &Frame, x: f64, y: f64, out: &mut [u8]) {
    let (w, h, channels) = (frame.w as usize, frame.h as usize, out.len());
    let (x, y) = (x.clamp(0.0, (w - 1) as f64), y.clamp(0.0, (h - 1) as f64));
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let px = |x: usize, y: usize, c: usize| frame.pixels[(y * w + x) * channels + c] as f32;
    for (c, v) in out.iter_mut().enumerate() {
        let top = px(x0, y0, c) * (1.0 - fx) + px(x1, y0, c) * fx;
        let bottom = px(x0, y1, c) * (1.0 - fx) + px(x1, y1, c) * fx;
        *v = (top * (1.0 - fy) + bottom * fy + 0.5) as u8;
    }
}

/// The quad `q` of `frame`, straightened into a rectangle; what falls outside `frame` gets
/// `fill` (transparent if `frame` has alpha).
// shortcut: a hard, aliased edge where the image ends; blend the border pixels if it shows.
fn warp(frame: &Frame, q: [Pos2; 4], fill: [u8; 3]) -> Frame {
    let [w, h] = warped_size(q);
    let (wu, hu, channels) = (w as usize, h as usize, if frame.alpha { 4 } else { 3 });
    let at = homography(q);
    let [r, g, b] = fill;
    let fill = [r, g, b, 0];
    let (fw, fh) = (frame.w as f64, frame.h as f64);
    let mut pixels = vec![0; wu * hu * channels];
    // Split across cores, as `Adjust::apply` does: this runs on the UI thread.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows = hu.div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for (part, chunk) in pixels.chunks_mut(rows * wu * channels).enumerate() {
            s.spawn(move || {
                for (r, row) in chunk.chunks_exact_mut(wu * channels).enumerate() {
                    let v = ((part * rows + r) as f64 + 0.5) / h as f64;
                    for (x, px) in row.chunks_exact_mut(channels).enumerate() {
                        let [sx, sy] = at((x as f64 + 0.5) / w as f64, v);
                        if (0.0..=fw).contains(&sx) && (0.0..=fh).contains(&sy) {
                            sample(frame, sx - 0.5, sy - 0.5, px);
                        } else {
                            px.copy_from_slice(&fill[..channels]);
                        }
                    }
                }
            });
        }
    });
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

/// `into_rgba8`, five times faster for RGB: these loops run every time markup is shown.
fn to_rgba(img: DynamicImage) -> image::RgbaImage {
    let DynamicImage::ImageRgb8(rgb) = img else { return img.into_rgba8() };
    let (w, h) = rgb.dimensions();
    let mut out = vec![255; w as usize * h as usize * 4];
    for (to, from) in out.as_chunks_mut::<4>().0.iter_mut().zip(rgb.as_raw().as_chunks::<3>().0) {
        to[..3].copy_from_slice(from);
    }
    image::RgbaImage::from_raw(w, h, out).expect("same size")
}

fn drop_alpha(rgba: &image::RgbaImage) -> image::RgbImage {
    let (w, h) = rgba.dimensions();
    let mut out = vec![0; w as usize * h as usize * 3];
    for (to, from) in out.as_chunks_mut::<3>().0.iter_mut().zip(rgba.as_raw().as_chunks::<4>().0) {
        to.copy_from_slice(&from[..3]);
    }
    image::RgbImage::from_raw(w, h, out).expect("same size")
}

/// The full-size edited image, for saving.
pub fn render(frame: &Frame, doc: &Doc) -> Result<DynamicImage, String> {
    let frame = doc.source(frame);
    let [w, h] = doc.plain_size();
    let img = if doc.crop.size() == Vec2::new(w as f32, h as f32) && doc.crop.min == doc.crop.min.round() {
        // Same scale: copy the pixels exactly instead of resampling.
        let img = to_dynamic(frame.alpha, frame.w, frame.h, frame.pixels.clone()).ok_or("bad frame")?;
        // Cropping copies pixel by pixel, which is slow even when it keeps them all.
        if [w, h] == [frame.w, frame.h] { img } else { img.crop_imm(doc.crop.min.x as u32, doc.crop.min.y as u32, w, h) }
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
        let mut rgba = to_rgba(img);
        markup::flatten(&mut rgba, &doc.elements);
        // Erasing to transparent (background removal) gives an opaque image an alpha channel.
        let alpha = frame.alpha || doc.elements.iter().any(|e| e.shape == markup::Shape::Erase && e.style.color.a() == 0);
        img = if alpha { DynamicImage::ImageRgba8(rgba) } else { DynamicImage::ImageRgb8(drop_alpha(&rgba)) };
    }
    Ok(img)
}

/// `render` as a frame, to show the markup as it will be saved.
pub fn flat(frame: &Frame, doc: &Doc) -> Result<Frame, String> {
    let img = render(frame, doc)?;
    let (w, h, alpha) = (img.width(), img.height(), img.color().has_alpha());
    Ok(Frame { w, h, pixels: img.into_bytes(), alpha, delay: frame.delay })
}

/// The Adjust Color panel body: sliders that change `a` live. Returns Some(true) for Done,
/// Some(false) for Cancel; `auto_levels` is called for the Auto Levels button.
pub fn adjust_ui(ui: &mut egui::Ui, a: &mut Adjust, auto_levels: impl FnOnce() -> Option<[f32; 2]>) -> Option<bool> {
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
        if ui.button(tr("Auto Levels")).clicked() {
            if let Some([exposure, contrast]) = auto_levels() {
                (a.exposure, a.contrast) = (exposure, contrast);
            }
        }
        if ui.button(tr("Reset")).clicked() {
            *a = a.sliders_reset();
        }
    });
    ui.add_space(6.0);
    done_cancel(ui)
}

/// The colour panels' last row: Some(true) for Done, Some(false) for Cancel.
pub fn done_cancel(ui: &mut egui::Ui) -> Option<bool> {
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

    #[test]
    fn invert_and_greyscale() {
        let mut px = vec![10, 200, 30, 255, 0, 255, 128, 7];
        Adjust { invert: true, ..Adjust::default() }.apply(&mut px, 4);
        assert_eq!(px, [245, 55, 225, 255, 255, 0, 127, 7], "colour flipped, alpha kept");
        let mut px = vec![10, 200, 30];
        Adjust { saturation: -1.0, ..Adjust::default() }.apply(&mut px, 3);
        assert!(px[0] == px[1] && px[1] == px[2], "{px:?}");
        // Invert comes last, after greyscale.
        let mut both = vec![10, 200, 30];
        Adjust { saturation: -1.0, invert: true, ..Adjust::default() }.apply(&mut both, 3);
        assert_eq!(both, px.iter().map(|v| 255 - v).collect::<Vec<u8>>());
    }

    /// Invert is per pixel, so the saved file is the view's pixels inverted at any size or crop.
    #[test]
    fn invert_matches_at_any_scale() {
        let mut doc = Doc::new(6, 5);
        doc.crop_to(Rect::from_min_max(Pos2::new(1.0, 1.0), Pos2::new(5.0, 4.0)));
        doc.resize(8, 6);
        let plain = render(&frame(), &doc).unwrap().into_bytes();
        let inverted = render(&frame(), &Doc { adjust: Adjust { invert: true, ..Adjust::default() }, ..doc }).unwrap().into_bytes();
        assert_eq!(inverted, plain.iter().map(|v| 255 - v).collect::<Vec<u8>>());
    }

    #[test]
    fn straighten_corners_enclose_the_turned_image() {
        let size = Vec2::new(400.0, 300.0);
        assert_eq!(straighten_size(size, 0.0), size);
        assert!((straighten_size(size, 90.0) - Vec2::new(300.0, 400.0)).length() < 1e-3);
        assert_eq!(straighten_size(size, 10.0), straighten_size(size, -10.0));
        for angle in [-170.0, -45.0, 3.0, 90.0, 135.0, 180.0] {
            let c = straighten_corners(size, angle);
            assert!(is_convex(c), "{angle}");
            // Every image corner is inside the quad: on the inner side of each clockwise edge.
            for p in [Pos2::ZERO, Pos2::new(400.0, 0.0), size.to_pos2(), Pos2::new(0.0, 300.0)] {
                let inside = |i: usize| {
                    let e = (c[(i + 1) % 4] - c[i]).normalized();
                    e.x * (p - c[i]).y - e.y * (p - c[i]).x >= -1e-2
                };
                assert!((0..4).all(inside), "{angle}: {p:?}");
            }
            let (w, h) = (c[0].distance(c[1]), c[1].distance(c[2]));
            assert!((Vec2::new(w, h) - straighten_size(size, angle)).length() < 1e-2, "{angle}");
            // The top edge runs at −angle in the unturned image, so it's level once turned.
            let d = c[1] - c[0];
            let off = (d.y.atan2(d.x).to_degrees() + angle).rem_euclid(360.0);
            assert!(off < 1e-3 || off > 360.0 - 1e-3, "{angle}");
        }
    }

    /// A line drawn 10° off level comes out level, on a canvas that holds the whole image, with
    /// the fill colour in the corners.
    #[test]
    fn straighten_levels_a_tilted_line() {
        let (w, h) = (200u32, 120u32);
        let mut pixels = vec![255u8; (w * h * 3) as usize];
        let tilt = 10f32.to_radians().tan();
        for x in 0..w {
            let y = (60.0 + (x as f32 - 100.0) * tilt).round() as u32;
            for dy in 0..2 {
                pixels[(((y + dy) * w + x) * 3) as usize] = 0;
            }
        }
        let decoded = Frame { w, h, pixels, alpha: false, delay: Duration::ZERO };
        let mut doc = Doc::new(w, h);
        doc.perspective(&decoded, straighten_corners(doc.size_vec(), -10.0), Some([200, 150, 100])).unwrap();
        let s = straighten_size(Vec2::new(w as f32, h as f32), 10.0);
        assert_eq!(doc.size, [s.x.round() as u32, s.y.round() as u32]);
        let out = render(&decoded, &doc).unwrap().into_rgb8();
        assert_eq!(out.get_pixel(0, 0).0, [200, 150, 100]);
        assert_eq!(out.get_pixel(out.width() - 1, out.height() - 1).0, [200, 150, 100]);
        // Each column's darkest row, across the middle where the line runs: the same all along.
        let (x0, x1) = ((out.width() - w) / 2 + 10, (out.width() + w) / 2 - 10);
        let rows: Vec<u32> = (x0..x1).map(|x| (0..out.height()).min_by_key(|&y| out.get_pixel(x, y)[0]).unwrap()).collect();
        let (lo, hi) = (rows.iter().min().unwrap(), rows.iter().max().unwrap());
        assert!(hi - lo <= 2, "{lo}..{hi}");
    }

    #[test]
    fn red_eye_only_touches_red() {
        // A red pupil, grey, a dark red too dim to matter, yellow, and skin.
        let mut img = image::RgbaImage::from_vec(5, 1, vec![220, 40, 30, 255, 120, 110, 100, 255, 50, 10, 10, 255, 200, 180, 30, 128, 220, 160, 130, 255]).unwrap();
        remove_red_eye(&mut img);
        assert_eq!(img.into_raw(), [35, 40, 30, 255, 120, 110, 100, 255, 50, 10, 10, 255, 200, 180, 30, 128, 220, 160, 130, 255]);
    }

    #[test]
    fn pixelate_blocks_are_noisy_means_on_a_random_grid() {
        let (w, h) = (64u32, 40u32);
        let source = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 4) as u8, (y * 6) as u8, 100, 200]));
        let block = (w.min(h) / 8).max(4);
        let run = || {
            let mut img = source.clone();
            pixelate(&mut img);
            img
        };
        let a = run();
        // Blocks are uniform: every pixel matches its neighbour unless a block edge runs between them.
        let edges = |horizontal: bool| -> Vec<u32> {
            let len = if horizontal { w } else { h };
            (1..len).filter(|&i| (0..if horizontal { h } else { w }).any(|j| if horizontal { a.get_pixel(i, j) != a.get_pixel(i - 1, j) } else { a.get_pixel(j, i) != a.get_pixel(j, i - 1) })).collect()
        };
        for (edges, len) in [(edges(true), w), (edges(false), h)] {
            assert!(edges.windows(2).all(|e| e[1] - e[0] == block), "{edges:?}");
            assert!(edges.len() as u32 >= len / block - 1);
        }
        // Each block is its mean plus at most ±20, and alpha is kept.
        for (x, y, px) in a.enumerate_pixels() {
            let mean = source.get_pixel(x, y);
            assert!(px[2].abs_diff(100) <= 20 && px[3] == 200, "{px:?}");
            assert!(px[0].abs_diff(mean[0]) <= 20 + block as u8 * 4, "{x},{y}");
        }
        assert!((0..5).any(|_| run() != a), "two runs never matched");
    }

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
    fn sharp_pixels_map_by_their_scale() {
        let mut doc = Doc::new(30, 20);
        doc.crop_to(Rect::from_min_size(Pos2::new(3.0, 2.0), Vec2::new(20.0, 10.0)));
        doc.resize(40, 20);
        for edit in [Doc::rotate_right, Doc::flip_horizontal, Doc::rotate_left] {
            edit(&mut doc);
            let sharp = doc.on_sharp(4);
            let frame = Rect::from_min_size(Pos2::ZERO, Vec2::new(120.0, 80.0));
            let (p, r) = (Pos2::new(7.0, 5.0), doc.bounds());
            assert!((sharp.source_point(p).to_vec2() - doc.source_point(p).to_vec2() * 4.0).length() < 1e-3);
            assert!((sharp.to_source(r).size() - doc.to_source(r).size() * 4.0).length() < 1e-3);
            assert!(frame.contains_rect(sharp.to_source(r)));
        }
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
    fn slider_reset_keeps_curves() {
        let curves = crate::curves::Curves::sample();
        let a = Adjust { exposure: 1.0, tint: 0.5, curves, ..Adjust::default() };
        assert_eq!(a.sliders_reset(), Adjust { curves, ..Adjust::default() });
    }

    #[test]
    fn auto_levels_stretches() {
        // Values 50..=200 on every channel.
        let pixels: Vec<u8> = (0..151u8).flat_map(|i| [50 + i; 3]).collect();
        let frame = Frame { w: 151, h: 1, pixels, alpha: false, delay: Duration::ZERO };
        let [exposure, contrast] = auto_levels(&frame, rect(0.0, 0.0, 151.0, 1.0));
        let a = Adjust { exposure, contrast, ..Adjust::default() };
        let mut px = vec![50, 125, 200];
        a.apply(&mut px, 3);
        assert_eq!(px, [0, 128, 255]);
        // A flat image keeps the full range.
        let flat = Frame { w: 2, h: 1, pixels: vec![9; 6], alpha: false, delay: Duration::ZERO };
        assert_eq!(auto_levels(&flat, rect(0.0, 0.0, 2.0, 1.0)), [0.0, 0.0]);
        // A bright image stops at the Contrast slider's end.
        let pixels: Vec<u8> = (0..128u8).flat_map(|i| [128 + i; 3]).collect();
        let bright = Frame { w: 128, h: 1, pixels, alpha: false, delay: Duration::ZERO };
        assert_eq!(auto_levels(&bright, rect(0.0, 0.0, 128.0, 1.0))[1], 1.0);
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
        let a = Adjust { exposure: 0.5, contrast: 0.2, saturation: 0.3, tint: 0.1, ..Adjust::default() };
        // The app keeps the resized pixels, so a slider step only re-runs the adjustment.
        let px = resize(&big, rect(0.0, 0.0, 4000.0, 3000.0), 1280, 960, ResizeAlg::Convolution(FilterType::Bilinear));
        let t = std::time::Instant::now();
        for _ in 0..10 {
            a.apply(&mut px.clone(), 3);
        }
        eprintln!("adjust 1280×960: {:.1} ms per step", t.elapsed().as_secs_f64() * 100.0);
        let a = Adjust { curves: crate::curves::Curves::sample(), ..a };
        let t = std::time::Instant::now();
        for _ in 0..10 {
            a.apply(&mut px.clone(), 3);
        }
        eprintln!("with HSL curves: {:.1} ms per step", t.elapsed().as_secs_f64() * 100.0);
    }

    #[test]
    fn flat_is_the_saved_image() {
        use crate::markup::{Element, Shape, Style};
        for (alpha, channels) in [(false, 3), (true, 4)] {
            let frame = Frame { w: 40, h: 30, pixels: (0..40 * 30 * channels).map(|i| (i * 7 % 251) as u8).collect(), alpha, delay: Duration::ZERO };
            let plain = Doc::new(40, 30);
            let mut changed = Doc::new(40, 30);
            changed.rotate_right();
            changed.resize(60, 80);
            changed.adjust.exposure = 0.5;
            changed.adjust.curves = crate::curves::Curves::sample();
            for mut doc in [plain, changed] {
                let mut e = Element::new(Shape::Rect, Pos2::new(2.0, 2.0), Style::for_image(doc.size_vec()));
                e.points[1] = Pos2::new(20.0, 20.0);
                doc.elements.push(e);
                let (saved, shown) = (render(&frame, &doc).unwrap(), flat(&frame, &doc).unwrap());
                assert_eq!([shown.w, shown.h], doc.size);
                assert_eq!(shown.alpha, alpha);
                assert_eq!(shown.pixels, saved.as_bytes());
            }
        }
    }

    #[test]
    fn fast_pixel_conversions_match_the_slow_ones() {
        let rgb = image::RgbImage::from_fn(5, 3, |x, y| image::Rgb([x as u8, y as u8, 200]));
        let rgba = to_rgba(DynamicImage::ImageRgb8(rgb.clone()));
        assert_eq!(rgba, DynamicImage::ImageRgb8(rgb.clone()).into_rgba8());
        assert_eq!(drop_alpha(&rgba), rgb);
        let clear = image::RgbaImage::from_fn(2, 2, |x, _| image::Rgba([9, 8, 7, x as u8]));
        assert_eq!(drop_alpha(&clear), DynamicImage::ImageRgba8(clear.clone()).into_rgb8());
        assert_eq!(to_rgba(DynamicImage::ImageRgba8(clear.clone())), clear);
    }

    /// How long `flat` takes, for `FLAT_MAX_PIXELS`:
    /// `cargo test --release flat_timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn flat_timing() {
        use crate::markup::{Element, Shape, Style};
        for mp in [1, 2, 4, 8, 12, 16, 24] {
            let w = (mp as f64 * 1e6 * 4.0 / 3.0).sqrt() as u32;
            let h = w * 3 / 4;
            let frame = Frame { w, h, pixels: (0..w as usize * h as usize * 3).map(|i| (i % 251) as u8).collect(), alpha: false, delay: Duration::ZERO };
            let mut doc = Doc::new(w, h);
            let style = Style::for_image(doc.size_vec());
            let mut e = Element::new(Shape::Rect, Pos2::new(w as f32 / 4.0, h as f32 / 4.0), style);
            e.points[1] = Pos2::new(w as f32 / 2.0, h as f32 / 2.0);
            doc.elements.push(e);
            let run = |doc: &Doc| {
                let times: Vec<f64> = (0..5).map(|_| {
                    let t = std::time::Instant::now();
                    std::hint::black_box(flat(&frame, doc).unwrap());
                    t.elapsed().as_secs_f64() * 1000.0
                }).collect();
                let mut sorted = times;
                sorted.sort_by(f64::total_cmp);
                sorted[2]
            };
            let shape = run(&doc);
            let paste = image::RgbaImage::from_pixel(w, h, image::Rgba([9, 9, 9, 255]));
            doc.elements.push(Element::pasted(std::sync::Arc::new(paste), Pos2::ZERO, doc.size_vec(), style));
            eprintln!("{mp:>2} MP: shape {shape:.0} ms, full-size paste {:.0} ms", run(&doc));
        }
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

    fn square(w: f32, h: f32) -> [Pos2; 4] {
        [Pos2::ZERO, Pos2::new(w, 0.0), Pos2::new(w, h), Pos2::new(0.0, h)]
    }

    #[test]
    fn homography_hits_the_corners() {
        let q = [Pos2::new(10.0, 5.0), Pos2::new(90.0, 20.0), Pos2::new(70.0, 80.0), Pos2::new(20.0, 60.0)];
        let at = homography(q);
        for ((u, v), p) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].into_iter().zip(q) {
            let [x, y] = at(u, v);
            assert!((x - p.x as f64).abs() < 1e-3 && (y - p.y as f64).abs() < 1e-3, "{u},{v}: {x},{y}");
        }
        // The centre is where the diagonals cross, not the average of the corners.
        let [x, y] = homography(square(10.0, 20.0))(0.5, 0.5);
        assert_eq!([x, y], [5.0, 10.0]);
        let [x, y] = at(0.5, 0.5);
        let t = ((x - 10.0) / 60.0, (y - 5.0) / 75.0);
        assert!((t.0 - t.1).abs() < 1e-6, "on the diagonal from corner 0 to corner 2");
    }

    #[test]
    fn crossed_corners_are_not_convex() {
        let [a, b, c, d] = square(10.0, 10.0);
        assert!(is_convex([a, b, c, d]));
        assert!(!is_convex([a, c, b, d]), "crossed");
        assert!(!is_convex([a, b, Pos2::new(1.0, 1.0), d]), "a dent");
        assert!(!is_convex([a, b, b, d]), "collapsed");
        let mut doc = Doc::new(6, 5);
        assert!(doc.perspective(&frame(), [a, c, b, d], None).is_err());
        assert_eq!(doc, Doc::new(6, 5), "a refused shape changes nothing");
    }

    #[test]
    fn warping_the_whole_image_keeps_it() {
        let mut doc = Doc::new(6, 5);
        doc.perspective(&frame(), square(6.0, 5.0), None).unwrap();
        assert_eq!(doc.size, [6, 5]);
        assert_eq!(render(&frame(), &doc).unwrap(), reference());
    }

    #[test]
    fn a_keystoned_rectangle_comes_back_straight() {
        // A white rectangle drawn with `homography` as a trapezoid, on black, then corrected:
        // the result is white nearly to its edges.
        let q = [Pos2::new(60.0, 20.0), Pos2::new(140.0, 20.0), Pos2::new(190.0, 130.0), Pos2::new(10.0, 130.0)];
        let mut pixels = vec![0; 200 * 150 * 3];
        let mut on = |x: f64, y: f64| pixels[(y as usize * 200 + x as usize) * 3..][..3].fill(255);
        let at = homography(q);
        for i in 0..1500 {
            for j in 0..1500 {
                let [x, y] = at(i as f64 / 1500.0, j as f64 / 1500.0);
                on(x, y);
            }
        }
        let f = Frame { w: 200, h: 150, pixels, alpha: false, delay: Duration::ZERO };
        let mut doc = Doc::new(200, 150);
        doc.perspective(&f, q, None).unwrap();
        assert_eq!(doc.size, [180, 121]);
        let out = render(&f, &doc).unwrap().to_rgb8();
        for x in 2..178 {
            assert!(out.get_pixel(x, 2)[0] > 200 && out.get_pixel(x, 118)[0] > 200 && out.get_pixel(x, 60)[0] > 200, "column {x}");
        }
        // A straight line of the frame's top edge stays a straight line.
        for y in 2..119 {
            assert!(out.get_pixel(2, y)[0] > 200 && out.get_pixel(177, y)[0] > 200, "row {y}");
        }
    }

    #[test]
    fn later_edits_work_on_the_corrected_image() {
        let mut doc = Doc::new(6, 5);
        doc.perspective(&frame(), [Pos2::new(1.0, 1.0), Pos2::new(5.0, 1.0), Pos2::new(5.0, 4.0), Pos2::new(1.0, 4.0)], None).unwrap();
        assert_eq!(doc.size, [4, 3]);
        let warped = reference().crop_imm(1, 1, 4, 3);
        // Sits on whole pixels, so it's a plain crop.
        assert_eq!(render(&frame(), &doc).unwrap(), warped);
        type Step = (fn(&mut Doc), fn(DynamicImage) -> DynamicImage);
        let steps: [Step; 3] = [
            (Doc::rotate_right, |i| i.rotate90()),
            (Doc::flip_horizontal, |i| i.fliph()),
            (|d| d.crop_to(rect(1.0, 1.0, 2.0, 1.0)), |i| i.crop_imm(1, 1, 2, 1)),
        ];
        for (edit, image_op) in steps {
            let mut d = doc.clone();
            edit(&mut d);
            assert_eq!(render(&frame(), &d).unwrap(), image_op(warped.clone()));
        }
    }

    #[test]
    fn perspective_keeps_colour_live_and_flattens_markup() {
        use crate::markup::{Shape, Style};
        let frame = Frame { w: 40, h: 30, pixels: vec![100; 40 * 30 * 3], alpha: false, delay: Duration::ZERO };
        let mut doc = Doc::new(40, 30);
        doc.adjust.exposure = 1.0;
        let mut e = Element::new(Shape::Rect, Pos2::new(5.0, 5.0), Style::for_image(doc.size_vec()));
        e.points[1] = Pos2::new(35.0, 25.0);
        doc.elements.push(e);
        let drawn = render(&frame, &Doc { adjust: Adjust::default(), ..doc.clone() }).unwrap().to_rgb8();
        doc.perspective(&frame, square(40.0, 30.0), None).unwrap();
        assert!(doc.elements.is_empty());
        assert_eq!(doc.adjust.exposure, 1.0);
        doc.adjust.exposure = 0.0;
        assert_eq!(render(&frame, &doc).unwrap().to_rgb8(), drawn, "the markup is in the pixels, the colour isn't");
    }

    /// `cargo test --release warp_timing -- --ignored --nocapture`: applying runs on the UI thread.
    #[test]
    #[ignore]
    fn warp_timing() {
        let big = Frame { w: 4000, h: 3000, pixels: (0..4000 * 3000 * 3).map(|i| (i % 251) as u8).collect(), alpha: false, delay: Duration::ZERO };
        let q = [Pos2::new(300.0, 100.0), Pos2::new(3700.0, 200.0), Pos2::new(3900.0, 2900.0), Pos2::new(100.0, 2800.0)];
        let t = std::time::Instant::now();
        std::hint::black_box(warp(&big, q, [0; 3]));
        eprintln!("warp 12 MP: {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
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
