use crate::i18n::{tr, tr_key};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use image::RgbaImage;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sketch,
    Rect,
    RoundedRect,
    Ellipse,
    Line,
    Arrow,
    Text,
    Highlight,
    Magnifier,
    /// Deleted or cut pixels: filled with `style.color` (background, or transparent).
    Erase,
    /// A pasted image, drawn into the rect of its two points.
    Image,
}

/// A pasted image. Compared by identity, so comparing edits never compares pixels.
#[derive(Clone)]
pub struct Pasted(pub Arc<RgbaImage>);

impl PartialEq for Pasted {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Pasted {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Pasted({}×{})", self.0.width(), self.0.height())
    }
}

/// A selection, in output px.
#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    Rect(Rect),
    Ellipse(Rect),
    Lasso(Vec<Pos2>),
}

impl Selection {
    /// The outline as a polygon.
    pub fn outline(&self) -> Vec<Pos2> {
        match self {
            Selection::Rect(r) => vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()],
            Selection::Ellipse(r) => (0..64).map(|i| r.center() + Vec2::angled(i as f32 / 64.0 * std::f32::consts::TAU) * r.size() / 2.0).collect(),
            Selection::Lasso(points) => points.clone(),
        }
    }

    pub fn bbox(&self) -> Rect {
        Rect::from_points(&self.outline())
    }

    pub fn is_empty(&self) -> bool {
        let b = self.bbox();
        b.width() < 1.0 || b.height() < 1.0
    }

    pub fn translate(&self, d: Vec2) -> Selection {
        match self {
            Selection::Rect(r) => Selection::Rect(r.translate(d)),
            Selection::Ellipse(r) => Selection::Ellipse(r.translate(d)),
            Selection::Lasso(points) => Selection::Lasso(points.iter().map(|&p| p + d).collect()),
        }
    }
}

/// The polygon as a path; with `outside`, everything in that rect except the polygon (even-odd).
fn polygon_path(points: &[Pos2], outside: Option<Rect>) -> Option<tiny_skia::Path> {
    let mut pb = tiny_skia::PathBuilder::new();
    if let Some(r) = outside.and_then(|r| tiny_skia::Rect::from_ltrb(r.left(), r.top(), r.right(), r.bottom())) {
        pb.push_rect(r);
    }
    let (first, rest) = points.split_first()?;
    pb.move_to(first.x, first.y);
    for p in rest {
        pb.line_to(p.x, p.y);
    }
    pb.close();
    pb.finish()
}

/// Coverage (0–255) of `polygon` over the `w`×`h` px area whose top-left is `origin`, scaled by `scale`.
pub fn polygon_mask(polygon: &[Pos2], outside: Option<Rect>, origin: Pos2, scale: f32, w: u32, h: u32) -> Option<tiny_skia::Mask> {
    let mut mask = tiny_skia::Mask::new(w, h)?;
    let t = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, -origin.x * scale, -origin.y * scale);
    mask.fill_path(&polygon_path(polygon, outside)?, tiny_skia::FillRule::EvenOdd, true, t);
    Some(mask)
}

/// The Shapes submenu.
pub const SHAPES: [(Shape, &str); 5] = [
    (Shape::Rect, tr_key("Rectangle")),
    (Shape::RoundedRect, tr_key("Rounded Rectangle")),
    (Shape::Ellipse, tr_key("Ellipse")),
    (Shape::Line, tr_key("Line")),
    (Shape::Arrow, tr_key("Arrow")),
];

/// Sizes are in image pixels, so markup keeps its size on the image at any zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// Stroke, text or highlight colour.
    pub color: Color32,
    pub width: f32,
    pub fill: Option<Color32>,
    pub font_size: f32,
    /// Magnifier: how much it enlarges.
    pub zoom: f32,
}

impl Style {
    /// Defaults that are visible on an image of this size.
    pub fn for_image(size: Vec2) -> Self {
        let long = size.max_elem();
        Style { color: Color32::from_rgb(229, 57, 53), width: (long / 400.0).round().max(2.0), fill: None, font_size: (long / 30.0).round().max(12.0), zoom: 2.0 }
    }
}

/// One markup element, in output-image pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub shape: Shape,
    /// Two corners for shapes and highlights, the path for Sketch, the top-left for Text,
    /// the centre and a point on the edge for Magnifier.
    pub points: Vec<Pos2>,
    pub text: String,
    pub style: Style,
    /// Erase: the first two points are a rect, and everything in it outside the polygon (the
    /// rest of the points) is erased.
    pub invert: bool,
    pub image: Option<Pasted>,
}

impl Element {
    pub fn new(shape: Shape, at: Pos2, style: Style) -> Self {
        let mut style = style;
        if shape == Shape::Highlight {
            style.color = Color32::from_rgba_unmultiplied(255, 235, 59, 100);
        }
        // All tools share one style, and only a lens can do without a stroke.
        if shape != Shape::Magnifier {
            style.width = style.width.max(1.0);
        }
        Element { shape, points: vec![at; if shape == Shape::Sketch { 1 } else { 2 }], text: String::new(), style, invert: false, image: None }
    }

    /// A magnifier at `at`, sized for an image of `image_size`.
    pub fn lens(at: Pos2, image_size: Vec2, style: Style) -> Self {
        let mut e = Element::new(Shape::Magnifier, at, style);
        e.points[1] = at + Vec2::new(image_size.min_elem() / 8.0, 0.0);
        e
    }

    /// Erases `selection` (or everything outside it) with `color`, in an image of size `bounds`.
    pub fn erase(selection: &Selection, invert: bool, bounds: Rect, color: Color32) -> Self {
        let mut points = if invert { vec![bounds.min, bounds.max] } else { Vec::new() };
        points.extend(selection.outline());
        let style = Style { color, width: 0.0, fill: None, font_size: 0.0, zoom: 1.0 };
        Element { shape: Shape::Erase, points, text: String::new(), style, invert, image: None }
    }

    /// A pasted image, with its top-left at `at` and `size` image px.
    pub fn pasted(img: Arc<RgbaImage>, at: Pos2, size: Vec2, style: Style) -> Self {
        Element { shape: Shape::Image, points: vec![at, at + size], text: String::new(), style, invert: false, image: Some(Pasted(img)) }
    }

    /// Erase: the polygon, and the rect it's cut out of when inverted.
    fn erase_parts(&self) -> (&[Pos2], Option<Rect>) {
        if self.invert { (&self.points[2..], Some(Rect::from_two_pos(self.points[0], self.points[1]))) } else { (&self.points, None) }
    }

    /// Erase and Image elements are drawn from a texture; this keys it, and stays the same
    /// while the element moves.
    pub fn texture_key(&self) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        match (self.shape, &self.image) {
            (Shape::Image, Some(Pasted(img))) => (Arc::as_ptr(img) as usize).hash(&mut h),
            (Shape::Erase, _) => {
                let b = self.bbox();
                for p in &self.points {
                    [(p.x - b.min.x).to_bits(), (p.y - b.min.y).to_bits()].hash(&mut h);
                }
                (self.invert, self.style.color).hash(&mut h);
            }
            _ => return None,
        }
        Some(h.finish())
    }

    /// The texture's pixels, at most `max_side` px on the longer side.
    pub fn texture_image(&self, max_side: f32) -> Option<egui::ColorImage> {
        match (self.shape, &self.image) {
            (Shape::Image, Some(Pasted(img))) => {
                // shortcut: a big image (a 4× Enhance result can be 16000 px wide) is shown at most
                // 4096 px across, so it fits the GPU and its memory; it's blurrier only when zoomed far in.
                let side = max_side.min(4096.0) as u32;
                let shown = if img.width().max(img.height()) > side {
                    let f = side as f32 / img.width().max(img.height()) as f32;
                    let (w, h) = (((img.width() as f32 * f).round() as u32).max(1), ((img.height() as f32 * f).round() as u32).max(1));
                    std::borrow::Cow::Owned(image::imageops::resize(img.as_ref(), w, h, image::imageops::FilterType::Triangle))
                } else {
                    std::borrow::Cow::Borrowed(img.as_ref())
                };
                Some(egui::ColorImage::from_rgba_unmultiplied([shown.width() as usize, shown.height() as usize], shown.as_raw()))
            }
            (Shape::Erase, _) => {
                let b = self.bbox();
                let scale = (max_side / b.size().max_elem()).min(1.0);
                let (w, hgt) = ((b.width() * scale).ceil().max(1.0) as u32, (b.height() * scale).ceil().max(1.0) as u32);
                let (polygon, outside) = self.erase_parts();
                let mask = polygon_mask(polygon, outside, b.min, scale, w, hgt)?;
                // Erasing to transparent: a white mask, which `paint` turns into the checkerboard.
                let [r, g, bl, a] = if self.style.color.a() == 0 { [255; 4] } else { self.style.color.to_srgba_unmultiplied() };
                let pixels: Vec<u8> = mask.data().iter().flat_map(|&m| [r, g, bl, (m as u32 * a as u32 / 255) as u8]).collect();
                Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, hgt as usize], &pixels))
            }
            _ => None,
        }
    }

    /// The area it covers (image px), for hit tests and selection.
    pub fn bbox(&self) -> Rect {
        match self.shape {
            Shape::Text => {
                let lines = self.text.lines().count().max(1) as f32;
                let longest = self.text.lines().map(|l| l.chars().count()).max().unwrap_or(1).max(1) as f32;
                Rect::from_min_size(self.points[0], Vec2::new(longest * 0.55, lines * 1.2) * self.style.font_size)
            }
            Shape::Magnifier => Rect::from_center_size(self.points[0], Vec2::splat(2.0 * self.radius())),
            Shape::Erase => {
                let (polygon, outside) = self.erase_parts();
                outside.unwrap_or_else(|| Rect::from_points(polygon))
            }
            Shape::Image => Rect::from_two_pos(self.points[0], self.points[1]),
            _ => Rect::from_points(&self.points).expand(self.style.width / 2.0),
        }
    }

    pub fn radius(&self) -> f32 {
        self.points[0].distance(self.points[self.points.len() - 1])
    }

    /// Maps every point through `f` (a geometric edit), scaling sizes by `scale`.
    pub fn map(&mut self, f: impl Fn(Pos2) -> Pos2, scale: f32) {
        for p in &mut self.points {
            *p = f(*p);
        }
        self.style.width *= scale;
        self.style.font_size *= scale;
    }

    pub fn translate(&mut self, d: Vec2) {
        self.map(|p| p + d, 1.0);
    }

    /// Too small to keep (a click without a drag).
    pub fn is_empty(&self) -> bool {
        match self.shape {
            Shape::Text => self.text.trim().is_empty(),
            Shape::Sketch => self.points.len() < 2,
            Shape::Erase | Shape::Image => false,
            _ => self.points[0].distance(self.points[1]) < 1.0,
        }
    }
}

/// The arrowhead's three corners: tip, then the two back corners.
fn arrowhead(a: Pos2, b: Pos2, width: f32) -> [Pos2; 3] {
    let len = width * 3.0 + 6.0;
    let d = (b - a).normalized();
    let back = b - d * len;
    let side = d.rot90() * len * 0.5;
    [b, back + side, back - side]
}

fn rounding(r: Rect) -> f32 {
    r.width().min(r.height()).abs() * 0.15
}

/// Draws `e` on screen. `to_screen` maps image px to points, `k` is points per image px,
/// and `uv` maps image px to the image texture's UVs (for the magnifier).
/// `own` is the element's own texture (Erase, Image; see `Element::texture`).
pub fn paint(painter: &egui::Painter, e: &Element, to_screen: &dyn Fn(Pos2) -> Pos2, k: f32, texture: egui::TextureId, uv: &dyn Fn(Pos2) -> Pos2, own: Option<egui::TextureId>) {
    let s = &e.style;
    let stroke = Stroke::new(s.width * k, s.color);
    let rect = || Rect::from_two_pos(to_screen(e.points[0]), to_screen(e.points[1]));
    match e.shape {
        Shape::Sketch => {
            painter.add(egui::Shape::line(e.points.iter().map(|&p| to_screen(p)).collect(), stroke));
        }
        Shape::Rect | Shape::RoundedRect => {
            let r = rect();
            let radius = if e.shape == Shape::RoundedRect { rounding(r) } else { 0.0 };
            painter.rect(r, radius, s.fill.unwrap_or(Color32::TRANSPARENT), stroke, egui::StrokeKind::Middle);
        }
        Shape::Ellipse => {
            let r = rect();
            painter.add(egui::epaint::EllipseShape { center: r.center(), radius: r.size() / 2.0, fill: s.fill.unwrap_or(Color32::TRANSPARENT), stroke, angle: 0.0 });
        }
        Shape::Line => {
            painter.line_segment([to_screen(e.points[0]), to_screen(e.points[1])], stroke);
        }
        Shape::Arrow => {
            let [tip, l, r] = arrowhead(e.points[0], e.points[1], s.width).map(to_screen);
            painter.line_segment([to_screen(e.points[0]), Pos2::new((l.x + r.x) / 2.0, (l.y + r.y) / 2.0)], stroke);
            painter.add(egui::Shape::convex_polygon(vec![tip, l, r], s.color, Stroke::NONE));
        }
        Shape::Text => {
            painter.text(to_screen(e.points[0]), egui::Align2::LEFT_TOP, &e.text, egui::FontId::proportional(s.font_size * k), s.color);
        }
        Shape::Highlight => {
            painter.rect_filled(rect(), 0.0, s.color);
        }
        Shape::Magnifier => {
            // A fan of triangles whose rim samples the image closer to the centre.
            let (c, r) = (e.points[0], e.radius());
            let mut mesh = egui::Mesh::with_texture(texture);
            let mut vertex = |p: Pos2| {
                let src = c + (p - c) / s.zoom;
                mesh.vertices.push(egui::epaint::Vertex { pos: to_screen(p), uv: uv(src), color: Color32::WHITE });
            };
            vertex(c);
            const SEGMENTS: u32 = 48;
            for i in 0..=SEGMENTS {
                let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                vertex(c + Vec2::angled(a) * r);
            }
            for i in 1..=SEGMENTS {
                mesh.add_triangle(0, i, i + 1);
            }
            painter.add(mesh);
            painter.circle_stroke(to_screen(c), r * k, stroke);
        }
        Shape::Erase | Shape::Image => {
            if let Some(tex) = own {
                let b = e.bbox();
                let r = Rect::from_two_pos(to_screen(b.min), to_screen(b.max));
                if e.shape == Shape::Erase && e.style.color.a() == 0 {
                    painter.add(checker(r.intersect(painter.clip_rect()), Some((tex, r))));
                } else {
                    painter.image(tex, r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
            }
        }
    }
}

/// The checkerboard that stands for transparency, over `rect` (screen points). Its squares are
/// fixed to the screen, so they line up everywhere and keep their size at any zoom. With a
/// `mask` (a texture and the screen rect it covers), it only shows where the mask does.
pub fn checker(rect: Rect, mask: Option<(egui::TextureId, Rect)>) -> egui::Mesh {
    const CELL: f32 = 8.0;
    const GREYS: [Color32; 2] = [Color32::from_gray(160), Color32::from_gray(110)];
    let mut mesh = egui::Mesh::with_texture(mask.map_or(egui::TextureId::default(), |(t, _)| t));
    if !rect.is_positive() {
        return mesh;
    }
    let uv = |p: Pos2| match mask {
        Some((_, m)) => ((p - m.min) / m.size()).to_pos2(),
        None => egui::epaint::WHITE_UV,
    };
    let (x0, y0) = ((rect.left() / CELL).floor() as i64, (rect.top() / CELL).floor() as i64);
    let (x1, y1) = ((rect.right() / CELL).ceil() as i64, (rect.bottom() / CELL).ceil() as i64);
    for cy in y0..y1 {
        for cx in x0..x1 {
            let cell = Rect::from_min_size(Pos2::new(cx as f32 * CELL, cy as f32 * CELL), Vec2::splat(CELL)).intersect(rect);
            let color = GREYS[(cx + cy).rem_euclid(2) as usize];
            let base = mesh.vertices.len() as u32;
            for p in [cell.left_top(), cell.right_top(), cell.right_bottom(), cell.left_bottom()] {
                mesh.vertices.push(egui::epaint::Vertex { pos: p, uv: uv(p), color });
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
    }
    mesh
}

const ROBOTO: &[u8] = include_bytes!("../fonts/Roboto-Regular.ttf");

/// The fonts of the whole UI. Apache-2.0 and MIT only: egui's default set isn't clearly GPL-compatible.
pub fn font_definitions() -> egui::FontDefinitions {
    use egui::{FontData, FontFamily, FontTweak};
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("Roboto".into(), Arc::new(FontData::from_static(ROBOTO)));
    fonts.font_data.insert("Droid Sans Mono".into(), Arc::new(FontData::from_static(include_bytes!("../fonts/DroidSansMono.ttf"))));
    let icons = FontData::from_static(include_bytes!("../fonts/emoji-icon-font.ttf")).tweak(FontTweak { scale: 0.90, ..Default::default() });
    fonts.font_data.insert("emoji-icon-font".into(), Arc::new(icons));
    let family = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    fonts.families.insert(FontFamily::Proportional, family(&["Roboto", "emoji-icon-font"]));
    fonts.families.insert(FontFamily::Monospace, family(&["Droid Sans Mono", "Roboto", "emoji-icon-font"]));
    fonts
}

/// The UI's proportional font, so saved text matches what's on screen.
fn font() -> Option<ab_glyph::FontRef<'static>> {
    ab_glyph::FontRef::try_from_slice(ROBOTO).ok()
}

fn skia_color(c: Color32) -> tiny_skia::Color {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    tiny_skia::Color::from_rgba8(r, g, b, a)
}

/// Draws the markup into `img` (straight-alpha RGBA, output size), in order.
pub fn flatten(img: &mut RgbaImage, elements: &[Element]) {
    if elements.is_empty() {
        return;
    }
    let (w, h) = img.dimensions();
    // tiny-skia works on premultiplied pixels.
    let mut data = std::mem::take(img).into_raw();
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a != 255 {
            for v in &mut px[..3] {
                *v = ((*v as u32 * a + 127) / 255) as u8;
            }
        }
    }
    let Some(mut pixmap) = tiny_skia::IntSize::from_wh(w, h).and_then(|size| tiny_skia::Pixmap::from_vec(data, size)) else { return };
    let id = tiny_skia::Transform::identity();
    for e in elements {
        let s = &e.style;
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(skia_color(s.color));
        paint.anti_alias = true;
        let stroke = tiny_skia::Stroke { width: s.width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        let (a, b) = (e.points[0], e.points[e.points.len() - 1]);
        let rect = Rect::from_two_pos(a, b);
        let sk_rect = tiny_skia::Rect::from_ltrb(rect.left(), rect.top(), rect.right(), rect.bottom());
        let fill = |pixmap: &mut tiny_skia::Pixmap, path: &tiny_skia::Path| {
            if let Some(f) = s.fill {
                let mut p = tiny_skia::Paint::default();
                p.set_color(skia_color(f));
                p.anti_alias = true;
                pixmap.fill_path(path, &p, tiny_skia::FillRule::Winding, id, None);
            }
        };
        match e.shape {
            Shape::Sketch | Shape::Line => {
                let mut pb = tiny_skia::PathBuilder::new();
                pb.move_to(a.x, a.y);
                for p in &e.points[1..] {
                    pb.line_to(p.x, p.y);
                }
                if let Some(path) = pb.finish() {
                    pixmap.stroke_path(&path, &paint, &stroke, id, None);
                }
            }
            Shape::Rect | Shape::RoundedRect => {
                let Some(r) = sk_rect else { continue };
                let path = if e.shape == Shape::RoundedRect { rounded_rect(rect, rounding(rect)) } else { Some(tiny_skia::PathBuilder::from_rect(r)) };
                if let Some(path) = path {
                    fill(&mut pixmap, &path);
                    pixmap.stroke_path(&path, &paint, &stroke, id, None);
                }
            }
            Shape::Ellipse => {
                if let Some(path) = sk_rect.and_then(tiny_skia::PathBuilder::from_oval) {
                    fill(&mut pixmap, &path);
                    pixmap.stroke_path(&path, &paint, &stroke, id, None);
                }
            }
            Shape::Arrow => {
                let [tip, l, r] = arrowhead(a, b, s.width);
                let mut pb = tiny_skia::PathBuilder::new();
                pb.move_to(a.x, a.y);
                pb.line_to((l.x + r.x) / 2.0, (l.y + r.y) / 2.0);
                if let Some(path) = pb.finish() {
                    pixmap.stroke_path(&path, &paint, &stroke, id, None);
                }
                let mut pb = tiny_skia::PathBuilder::new();
                pb.move_to(tip.x, tip.y);
                pb.line_to(l.x, l.y);
                pb.line_to(r.x, r.y);
                pb.close();
                if let Some(path) = pb.finish() {
                    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, id, None);
                }
            }
            Shape::Highlight => {
                if let Some(r) = sk_rect {
                    pixmap.fill_rect(r, &paint, id, None);
                }
            }
            Shape::Text => draw_text(&mut pixmap, e),
            Shape::Erase => {
                let (polygon, outside) = e.erase_parts();
                if let Some(path) = polygon_path(polygon, outside) {
                    // Source, not source-over, so a transparent colour clears.
                    paint.blend_mode = tiny_skia::BlendMode::Source;
                    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::EvenOdd, id, None);
                }
            }
            Shape::Image => {
                let Some(Pasted(src)) = &e.image else { continue };
                let mut data = src.as_raw().clone();
                for px in data.as_chunks_mut::<4>().0 {
                    let a = px[3] as u32;
                    if a != 255 {
                        for v in &mut px[..3] {
                            *v = ((*v as u32 * a + 127) / 255) as u8;
                        }
                    }
                }
                let Some(src_map) = tiny_skia::IntSize::from_wh(src.width(), src.height()).and_then(|size| tiny_skia::Pixmap::from_vec(data, size)) else { continue };
                let t = tiny_skia::Transform::from_row(rect.width() / src.width() as f32, 0.0, 0.0, rect.height() / src.height() as f32, rect.left(), rect.top());
                pixmap.draw_pixmap(0, 0, src_map.as_ref(), &tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() }, t, None);
            }
            Shape::Magnifier => {
                let (c, radius) = (a, e.radius());
                let source = pixmap.clone();
                let (sw, sh) = (source.width() as i32, source.height() as i32);
                let (x0, x1) = ((c.x - radius).floor().max(0.0) as i32, (c.x + radius).ceil().min(sw as f32) as i32);
                let (y0, y1) = ((c.y - radius).floor().max(0.0) as i32, (c.y + radius).ceil().min(sh as f32) as i32);
                let src = source.data();
                let dst = pixmap.data_mut();
                for y in y0..y1 {
                    for x in x0..x1 {
                        let p = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                        if p.distance(c) > radius {
                            continue;
                        }
                        let q = c + (p - c) / s.zoom;
                        let (qx, qy) = ((q.x as i32).clamp(0, sw - 1), (q.y as i32).clamp(0, sh - 1));
                        let (d, s) = (((y * sw + x) * 4) as usize, ((qy * sw + qx) * 4) as usize);
                        dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
                    }
                }
                // tiny-skia draws width 0 as a hairline.
                if s.width > 0.0 && let Some(path) = tiny_skia::PathBuilder::from_circle(c.x, c.y, radius) {
                    pixmap.stroke_path(&path, &paint, &stroke, id, None);
                }
            }
        }
    }
    let mut data = pixmap.take();
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            for v in &mut px[..3] {
                *v = ((*v as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    *img = RgbaImage::from_raw(w, h, data).expect("same size");
}

fn rounded_rect(r: Rect, radius: f32) -> Option<tiny_skia::Path> {
    // Quarter circles approximated with one cubic each (the usual 0.5523 factor).
    let k = radius * 0.5523;
    let (l, t, rt, b) = (r.left(), r.top(), r.right(), r.bottom());
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(l + radius, t);
    pb.line_to(rt - radius, t);
    pb.cubic_to(rt - radius + k, t, rt, t + radius - k, rt, t + radius);
    pb.line_to(rt, b - radius);
    pb.cubic_to(rt, b - radius + k, rt - radius + k, b, rt - radius, b);
    pb.line_to(l + radius, b);
    pb.cubic_to(l + radius - k, b, l, b - radius + k, l, b - radius);
    pb.line_to(l, t + radius);
    pb.cubic_to(l, t + radius - k, l + radius - k, t, l + radius, t);
    pb.close();
    pb.finish()
}

/// Rasterizes a text element with egui's font, blending into the premultiplied pixmap.
fn draw_text(pixmap: &mut tiny_skia::Pixmap, e: &Element) {
    use ab_glyph::{Font, ScaleFont};
    let Some(font) = font() else { return };
    // egui's font size is the em size; ab_glyph's PxScale is ascent − descent.
    let scale = font.pt_to_px_scale(e.style.font_size).unwrap_or(ab_glyph::PxScale::from(e.style.font_size));
    let scaled = font.as_scaled(scale);
    let [r, g, b, a] = e.style.color.to_srgba_unmultiplied();
    let (w, h) = (pixmap.width(), pixmap.height());
    let data = pixmap.data_mut();
    let mut y = e.points[0].y + scaled.ascent();
    for line in e.text.lines() {
        let mut x = e.points[0].x;
        let mut prev = None;
        for ch in line.chars() {
            let id = font.glyph_id(ch);
            if let Some(p) = prev {
                x += scaled.kern(p, id);
            }
            prev = Some(id);
            let glyph = id.with_scale_and_position(scale, ab_glyph::point(x, y));
            x += scaled.h_advance(id);
            let Some(outlined) = font.outline_glyph(glyph) else { continue };
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let (px, py) = (bounds.min.x as i64 + gx as i64, bounds.min.y as i64 + gy as i64);
                if px < 0 || py < 0 || px >= w as i64 || py >= h as i64 {
                    return;
                }
                let i = ((py as u32 * w + px as u32) * 4) as usize;
                let alpha = coverage.clamp(0.0, 1.0) * a as f32 / 255.0;
                for (c, v) in [r, g, b, 255].into_iter().enumerate() {
                    // Source-over on premultiplied values.
                    let src = v as f32 * alpha;
                    data[i + c] = (src + data[i + c] as f32 * (1.0 - alpha)).round() as u8;
                }
            });
        }
        y += scaled.height() + scaled.line_gap();
    }
}

/// The tools the markup toolbar picks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    Select,
    Draw(Shape),
    Measure,
}

/// What the toolbar asks for this frame.
#[derive(Default)]
pub struct ToolbarOut {
    pub pick: Option<Pick>,
    pub undo: bool,
    pub redo: bool,
    pub done: bool,
    pub describe: bool,
    pub style_changed: bool,
}

/// The markup toolbar. `styled` is the shape whose style controls to show (the selected
/// element's, or the drawing tool's), editing `style`.
pub fn toolbar_ui(ui: &mut egui::Ui, current: Option<Pick>, style: &mut Style, styled: Option<Shape>, can_undo: bool, can_redo: bool) -> ToolbarOut {
    let mut out = ToolbarOut::default();
    ui.horizontal_wrapped(|ui| {
        let mut tool = |ui: &mut egui::Ui, pick: Pick, label: &str| {
            if ui.add(egui::Button::selectable(current == Some(pick), label)).clicked() {
                out.pick = Some(pick);
            }
        };
        tool(ui, Pick::Select, tr("Select"));
        tool(ui, Pick::Draw(Shape::Sketch), tr("Sketch"));
        let shape = SHAPES.iter().find(|(s, _)| current == Some(Pick::Draw(*s)));
        let shapes = egui::Button::selectable(shape.is_some(), format!("{} ⏷", shape.map_or(tr("Shapes"), |(_, name)| tr(name))));
        egui::containers::menu::MenuButton::from_button(shapes).ui(ui, |ui| {
            for (s, name) in SHAPES {
                if ui.button(tr(name)).clicked() {
                    out.pick = Some(Pick::Draw(s));
                }
            }
        });
        let mut tool = |ui: &mut egui::Ui, pick: Pick, label: &str| {
            if ui.add(egui::Button::selectable(current == Some(pick), label)).clicked() {
                out.pick = Some(pick);
            }
        };
        tool(ui, Pick::Draw(Shape::Text), tr("Text"));
        tool(ui, Pick::Draw(Shape::Highlight), tr("Highlight"));
        tool(ui, Pick::Draw(Shape::Magnifier), tr("Magnifier"));
        tool(ui, Pick::Measure, tr("Measure"));
        out.describe = ui.button(tr("Description")).on_hover_text(tr("Edit the image description")).clicked();
        if let Some(shape) = styled {
            ui.separator();
            out.style_changed = style_ui(ui, shape, style);
        }
        ui.separator();
        out.undo = ui.add_enabled(can_undo, egui::Button::new(tr("Undo"))).clicked();
        out.redo = ui.add_enabled(can_redo, egui::Button::new(tr("Redo"))).clicked();
        out.done = ui.button(tr("Done")).clicked();
    });
    out
}

/// The style controls that apply to `shape`. Returns whether anything changed.
fn style_ui(ui: &mut egui::Ui, shape: Shape, s: &mut Style) -> bool {
    use egui::color_picker::{Alpha, color_edit_button_srgba};
    let mut changed = false;
    let label = match shape {
        Shape::Text => tr("Text colour"),
        Shape::Highlight => tr("Colour"),
        _ => tr("Stroke"),
    };
    ui.label(label);
    changed |= color_edit_button_srgba(ui, &mut s.color, Alpha::OnlyBlend).changed();
    if shape != Shape::Text && shape != Shape::Highlight {
        let min = if shape == Shape::Magnifier { 0.0 } else { 1.0 };
        changed |= ui.add(egui::DragValue::new(&mut s.width).range(min..=500.0).suffix(" px")).on_hover_text(tr("Thickness")).changed();
    }
    match shape {
        Shape::Rect | Shape::RoundedRect | Shape::Ellipse => {
            let mut filled = s.fill.is_some();
            if ui.checkbox(&mut filled, tr("Fill")).changed() {
                s.fill = filled.then(|| s.color.gamma_multiply(0.5));
                changed = true;
            }
            if let Some(fill) = &mut s.fill {
                changed |= color_edit_button_srgba(ui, fill, Alpha::OnlyBlend).changed();
            }
        }
        Shape::Text => {
            changed |= ui.add(egui::DragValue::new(&mut s.font_size).range(4.0..=2000.0).suffix(" px")).on_hover_text(tr("Font size")).changed();
        }
        Shape::Magnifier => {
            ui.label(tr("Zoom"));
            changed |= ui.add(egui::Slider::new(&mut s.zoom, 1.25..=8.0).suffix("×")).changed();
        }
        _ => {}
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]))
    }

    fn style() -> Style {
        Style { color: Color32::from_rgb(255, 0, 0), width: 4.0, fill: None, font_size: 20.0, zoom: 2.0 }
    }

    fn element(shape: Shape, a: (f32, f32), b: (f32, f32)) -> Element {
        Element { shape, points: vec![Pos2::new(a.0, a.1), Pos2::new(b.0, b.1)], text: String::new(), style: style(), invert: false, image: None }
    }

    fn is_red(img: &RgbaImage, x: u32, y: u32) -> bool {
        let p = img.get_pixel(x, y);
        p[0] > 200 && p[1] < 60 && p[2] < 60
    }

    fn is_white(img: &RgbaImage, x: u32, y: u32) -> bool {
        img.get_pixel(x, y).0 == [255, 255, 255, 255]
    }

    #[test]
    fn rect_outline_where_drawn() {
        let mut img = white(100, 100);
        flatten(&mut img, &[element(Shape::Rect, (20.0, 20.0), (80.0, 60.0))]);
        assert!(is_red(&img, 50, 20) && is_red(&img, 20, 40) && is_red(&img, 80, 40) && is_red(&img, 50, 60), "edges");
        assert!(is_white(&img, 50, 40), "no fill inside");
        assert!(is_white(&img, 10, 10) && is_white(&img, 90, 90), "nothing outside");
    }

    #[test]
    fn filled_ellipse_and_highlight() {
        let mut img = white(100, 100);
        let mut e = element(Shape::Ellipse, (10.0, 10.0), (90.0, 50.0));
        e.style.fill = Some(Color32::from_rgb(255, 0, 0));
        flatten(&mut img, &[e]);
        assert!(is_red(&img, 50, 30), "filled centre");
        assert!(is_white(&img, 12, 12), "corner of the bbox is outside the ellipse");

        let mut img = white(100, 100);
        let mut highlight = Element::new(Shape::Highlight, Pos2::ZERO, style());
        highlight.points[1] = Pos2::new(50.0, 50.0);
        flatten(&mut img, &[highlight]);
        let p = img.get_pixel(25, 25);
        assert!(p[2] < 255 && p[0] == 255, "translucent yellow over white: {p:?}");
        assert!(is_white(&img, 75, 75));
    }

    #[test]
    fn line_arrow_sketch_text() {
        let mut img = white(100, 100);
        flatten(&mut img, &[element(Shape::Arrow, (10.0, 50.0), (90.0, 50.0))]);
        assert!(is_red(&img, 30, 50) && is_red(&img, 85, 50), "shaft and tip");
        assert!(is_red(&img, 76, 45), "arrowhead is wider than the shaft");
        assert!(is_white(&img, 30, 44));

        let mut img = white(100, 100);
        let mut sketch = element(Shape::Sketch, (10.0, 10.0), (50.0, 10.0));
        sketch.points.push(Pos2::new(50.0, 80.0));
        flatten(&mut img, &[sketch]);
        assert!(is_red(&img, 30, 10) && is_red(&img, 50, 60));

        let mut img = white(200, 60);
        let mut text = element(Shape::Text, (10.0, 10.0), (10.0, 10.0));
        text.text = "Hello".into();
        flatten(&mut img, &[text.clone()]);
        let inked = img.pixels().filter(|p| p[1] < 128).count();
        assert!(inked > 50, "text drew {inked} px");
        assert!(text.bbox().contains(Pos2::new(40.0, 20.0)));
        // Every inked pixel is near the text's box.
        let near = text.bbox().expand(6.0);
        assert!(img.enumerate_pixels().filter(|(_, _, p)| p[1] < 128).all(|(x, y, _)| near.contains(Pos2::new(x as f32, y as f32))));
    }

    #[test]
    fn magnifier_enlarges() {
        // Left half black, right half white; a 2× lens centred on the edge.
        let mut img = RgbaImage::from_fn(100, 100, |x, _| if x < 50 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255, 255, 255, 255]) });
        let mut lens = element(Shape::Magnifier, (50.0, 50.0), (80.0, 50.0));
        lens.style.width = 1.0;
        flatten(&mut img, &[lens]);
        // 10 px right of the centre shows what was 5 px right: still white; 10 px left: black.
        assert_eq!(img.get_pixel(60, 50)[0], 255);
        assert_eq!(img.get_pixel(40, 50)[0], 0);
    }

    #[test]
    fn lens_without_stroke_has_no_ring() {
        let mut img = white(100, 100);
        let mut lens = element(Shape::Magnifier, (50.0, 50.0), (30.0, 50.0));
        lens.style.width = 0.0;
        flatten(&mut img, &[lens]);
        assert!(img.pixels().all(|p| p.0 == [255, 255, 255, 255]));
    }

    #[test]
    fn only_a_lens_can_have_no_stroke() {
        let style = Style { width: 0.0, ..Style::for_image(Vec2::splat(100.0)) };
        assert_eq!(Element::new(Shape::Sketch, Pos2::ZERO, style).style.width, 1.0);
        assert_eq!(Element::lens(Pos2::ZERO, Vec2::splat(100.0), style).style.width, 0.0);
    }

    #[test]
    fn erase_and_paste() {
        let full = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 100.0));
        let sel = Selection::Ellipse(Rect::from_min_max(Pos2::new(20.0, 20.0), Pos2::new(80.0, 80.0)));
        let black = Color32::from_rgb(0, 0, 0);
        // Deleting a selection fills it; everything else is untouched.
        let mut img = white(100, 100);
        flatten(&mut img, &[Element::erase(&sel, false, full, black)]);
        assert_eq!(img.get_pixel(50, 50).0, [0, 0, 0, 255]);
        assert!(is_white(&img, 22, 22), "outside the ellipse");
        // Inverted (Create Image from Selection): the outside goes, the inside stays.
        let mut img = white(100, 100);
        flatten(&mut img, &[Element::erase(&sel, true, full, Color32::TRANSPARENT)]);
        assert_eq!(img.get_pixel(5, 5)[3], 0, "transparent outside");
        assert!(is_white(&img, 50, 50));
        // A pasted 2×1 image (red, blue) stretched over a 40×20 rect.
        let src = Arc::new(RgbaImage::from_fn(2, 1, |x, _| if x == 0 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) }));
        let mut img = white(100, 100);
        flatten(&mut img, &[Element::pasted(src, Pos2::new(10.0, 10.0), Vec2::new(40.0, 20.0), style())]);
        assert!(is_red(&img, 12, 20));
        assert!(img.get_pixel(48, 20)[2] > 200 && img.get_pixel(48, 20)[0] < 60, "blue on the right");
        assert!(is_white(&img, 60, 20) && is_white(&img, 20, 40));
    }

    #[test]
    fn selection_masks() {
        let lasso = Selection::Lasso(vec![Pos2::new(0.0, 0.0), Pos2::new(10.0, 0.0), Pos2::new(0.0, 10.0)]);
        let mask = polygon_mask(&lasso.outline(), None, Pos2::ZERO, 1.0, 10, 10).unwrap();
        let at = |x: usize, y: usize| mask.data()[y * 10 + x];
        assert_eq!(at(1, 1), 255);
        assert_eq!(at(8, 8), 0);
        // Offset and scaled: the same triangle at half size, from (5, 5).
        let half = polygon_mask(&lasso.outline(), None, Pos2::new(-10.0, -10.0), 0.5, 10, 10).unwrap();
        assert_eq!(half.data()[6 * 10 + 6], 255);
        assert_eq!(half.data()[2 * 10 + 2], 0);
        assert!(Selection::Rect(Rect::from_min_size(Pos2::ZERO, Vec2::new(0.5, 9.0))).is_empty());
        let e = Selection::Ellipse(Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(30.0, 60.0)));
        assert!((e.bbox().min - Pos2::new(10.0, 20.0)).length() < 1e-3 && (e.bbox().max - Pos2::new(30.0, 60.0)).length() < 1e-3);
    }

    #[test]
    fn checkerboard() {
        // 20×10 points from (4, 4): cells 0..3 across and 0..2 down, clipped to the rect.
        let rect = Rect::from_min_size(Pos2::new(4.0, 4.0), Vec2::new(20.0, 10.0));
        let mesh = checker(rect, None);
        assert_eq!(mesh.vertices.len(), 3 * 2 * 4);
        assert!(mesh.vertices.iter().all(|v| rect.expand(1e-3).contains(v.pos)));
        assert!(mesh.vertices.iter().all(|v| v.uv == egui::epaint::WHITE_UV));
        // Neighbouring cells alternate between the two greys.
        assert_ne!(mesh.vertices[0].color, mesh.vertices[4].color);
        assert_eq!(mesh.vertices[0].color, mesh.vertices[8].color);
        // With a mask, the corners sample it across its rect.
        let tex = egui::TextureId::User(7);
        let masked = checker(Rect::from_min_size(Pos2::ZERO, Vec2::splat(16.0)), Some((tex, Rect::from_min_size(Pos2::ZERO, Vec2::splat(16.0)))));
        assert_eq!(masked.texture_id, tex);
        assert!(masked.vertices.iter().any(|v| v.uv == Pos2::new(1.0, 1.0)));
        assert!(checker(Rect::NOTHING, None).vertices.is_empty());
    }

    #[test]
    fn big_pasted_images_are_shown_smaller() {
        let big = Element::pasted(Arc::new(RgbaImage::new(5000, 100)), Pos2::ZERO, Vec2::new(5000.0, 100.0), style());
        assert_eq!(big.texture_image(16384.0).unwrap().size, [4096, 82]);
        let small = Element::pasted(Arc::new(RgbaImage::new(300, 200)), Pos2::ZERO, Vec2::new(300.0, 200.0), style());
        assert_eq!(small.texture_image(16384.0).unwrap().size, [300, 200]);
    }

    #[test]
    fn transparent_erase_is_a_white_mask() {
        let sel = Selection::Rect(Rect::from_min_size(Pos2::ZERO, Vec2::splat(4.0)));
        let full = Rect::from_min_size(Pos2::ZERO, Vec2::splat(4.0));
        let clear = Element::erase(&sel, false, full, Color32::TRANSPARENT).texture_image(64.0).unwrap();
        assert!(clear.pixels.iter().all(|p| *p == Color32::WHITE));
        let red = Element::erase(&sel, false, full, Color32::from_rgb(200, 0, 0)).texture_image(64.0).unwrap();
        assert!(red.pixels.iter().all(|p| *p == Color32::from_rgb(200, 0, 0)));
    }

    #[test]
    fn map_and_bbox() {
        let mut e = element(Shape::Rect, (10.0, 20.0), (30.0, 40.0));
        e.map(|p| (p.to_vec2() * 2.0).to_pos2(), 2.0);
        assert_eq!(e.points, [Pos2::new(20.0, 40.0), Pos2::new(60.0, 80.0)]);
        assert_eq!(e.style.width, 8.0);
        assert_eq!(e.bbox(), Rect::from_min_max(Pos2::new(16.0, 36.0), Pos2::new(64.0, 84.0)));
        assert!(element(Shape::Line, (5.0, 5.0), (5.2, 5.0)).is_empty());
    }
}
