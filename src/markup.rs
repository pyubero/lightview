use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use image::RgbaImage;
use std::sync::{Arc, OnceLock};

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
}

/// The Shapes submenu.
pub const SHAPES: [(Shape, &str); 5] = [
    (Shape::Rect, "Rectangle"),
    (Shape::RoundedRect, "Rounded Rectangle"),
    (Shape::Ellipse, "Ellipse"),
    (Shape::Line, "Line"),
    (Shape::Arrow, "Arrow"),
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
}

impl Element {
    pub fn new(shape: Shape, at: Pos2, style: Style) -> Self {
        let mut style = style;
        if shape == Shape::Highlight {
            style.color = Color32::from_rgba_unmultiplied(255, 235, 59, 100);
        }
        Element { shape, points: vec![at; if shape == Shape::Sketch { 1 } else { 2 }], text: String::new(), style }
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
pub fn paint(painter: &egui::Painter, e: &Element, to_screen: &dyn Fn(Pos2) -> Pos2, k: f32, texture: egui::TextureId, uv: &dyn Fn(Pos2) -> Pos2) {
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
    }
}

/// egui's default proportional font, so saved text matches what's on screen.
fn font() -> Option<ab_glyph::FontRef<'static>> {
    static FONT: OnceLock<Arc<egui::FontData>> = OnceLock::new();
    let data = FONT.get_or_init(|| egui::FontDefinitions::default().font_data["Ubuntu-Light"].clone());
    let bytes: &'static [u8] = (*data.font).as_ref();
    ab_glyph::FontRef::try_from_slice(bytes).ok()
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
        for v in &mut px[..3] {
            *v = ((*v as u32 * a + 127) / 255) as u8;
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
                if let Some(path) = tiny_skia::PathBuilder::from_circle(c.x, c.y, radius) {
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
        tool(ui, Pick::Select, "Select");
        tool(ui, Pick::Draw(Shape::Sketch), "Sketch");
        let shape = SHAPES.iter().find(|(s, _)| current == Some(Pick::Draw(*s)));
        let shapes = egui::Button::selectable(shape.is_some(), format!("{} ⏷", shape.map_or("Shapes", |(_, name)| name)));
        egui::containers::menu::MenuButton::from_button(shapes).ui(ui, |ui| {
            for (s, name) in SHAPES {
                if ui.button(name).clicked() {
                    out.pick = Some(Pick::Draw(s));
                }
            }
        });
        let mut tool = |ui: &mut egui::Ui, pick: Pick, label: &str| {
            if ui.add(egui::Button::selectable(current == Some(pick), label)).clicked() {
                out.pick = Some(pick);
            }
        };
        tool(ui, Pick::Draw(Shape::Text), "Text");
        tool(ui, Pick::Draw(Shape::Highlight), "Highlight");
        tool(ui, Pick::Draw(Shape::Magnifier), "Magnifier");
        tool(ui, Pick::Measure, "Measure");
        out.describe = ui.button("Description").on_hover_text("Edit the image description").clicked();
        if let Some(shape) = styled {
            ui.separator();
            out.style_changed = style_ui(ui, shape, style);
        }
        ui.separator();
        out.undo = ui.add_enabled(can_undo, egui::Button::new("Undo")).clicked();
        out.redo = ui.add_enabled(can_redo, egui::Button::new("Redo")).clicked();
        out.done = ui.button("Done").clicked();
    });
    out
}

/// The style controls that apply to `shape`. Returns whether anything changed.
fn style_ui(ui: &mut egui::Ui, shape: Shape, s: &mut Style) -> bool {
    use egui::color_picker::{Alpha, color_edit_button_srgba};
    let mut changed = false;
    let label = match shape {
        Shape::Text => "Text colour",
        Shape::Highlight => "Colour",
        _ => "Stroke",
    };
    ui.label(label);
    changed |= color_edit_button_srgba(ui, &mut s.color, Alpha::OnlyBlend).changed();
    if shape != Shape::Text && shape != Shape::Highlight {
        changed |= ui.add(egui::DragValue::new(&mut s.width).range(1.0..=500.0).suffix(" px")).on_hover_text("Thickness").changed();
    }
    match shape {
        Shape::Rect | Shape::RoundedRect | Shape::Ellipse => {
            let mut filled = s.fill.is_some();
            if ui.checkbox(&mut filled, "Fill").changed() {
                s.fill = filled.then(|| s.color.gamma_multiply(0.5));
                changed = true;
            }
            if let Some(fill) = &mut s.fill {
                changed |= color_edit_button_srgba(ui, fill, Alpha::OnlyBlend).changed();
            }
        }
        Shape::Text => {
            changed |= ui.add(egui::DragValue::new(&mut s.font_size).range(4.0..=2000.0).suffix(" px")).on_hover_text("Font size").changed();
        }
        Shape::Magnifier => {
            ui.label("Zoom");
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
        Element { shape, points: vec![Pos2::new(a.0, a.1), Pos2::new(b.0, b.1)], text: String::new(), style: style() }
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
    fn map_and_bbox() {
        let mut e = element(Shape::Rect, (10.0, 20.0), (30.0, 40.0));
        e.map(|p| (p.to_vec2() * 2.0).to_pos2(), 2.0);
        assert_eq!(e.points, [Pos2::new(20.0, 40.0), Pos2::new(60.0, 80.0)]);
        assert_eq!(e.style.width, 8.0);
        assert_eq!(e.bbox(), Rect::from_min_max(Pos2::new(16.0, 36.0), Pos2::new(64.0, 84.0)));
        assert!(element(Shape::Line, (5.0, 5.0), (5.2, 5.0)).is_empty());
    }
}
