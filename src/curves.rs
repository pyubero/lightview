//! Tone curves: the Adjust Curves panel, and what `Adjust::apply` needs to run them.

use crate::edit::{Adjust, done_cancel};
use crate::i18n::{tr, tr_key};
use crate::load::Frame;
use eframe::egui::{self, Pos2, Rect, Sense, Shape, Stroke, Vec2};

const MAX_POINTS: usize = 16;
/// Between two points' x: keeps the spline's slopes finite.
const MIN_GAP: f32 = 1.0 / 255.0;
const GRAPH_SIZE: f32 = 256.0;
/// How near a point the pointer must be to grab it, in UI px.
const GRAB: f32 = 10.0;
/// Pixels sampled for the histogram.
const HIST_SAMPLES: usize = 200_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CurveSpace {
    #[default]
    Rgb,
    Hsl,
    Hsv,
}

impl CurveSpace {
    fn name(self) -> &'static str {
        match self {
            CurveSpace::Rgb => "RGB",
            CurveSpace::Hsl => "HSL",
            CurveSpace::Hsv => "HSV",
        }
    }

    /// Each channel's selector label and description.
    fn channels(self) -> &'static [(&'static str, &'static str)] {
        const HUE: (&str, &str) = ("H", tr_key("Hue: the color itself, such as red or blue"));
        const SATURATION: (&str, &str) = ("S", tr_key("Saturation: from grey to vivid color"));
        const RGB: &[(&str, &str)] = &[
            ("RGB", tr_key("All channels together: overall brightness")),
            ("R", tr_key("Red: how much red each pixel has")),
            ("G", tr_key("Green: how much green each pixel has")),
            ("B", tr_key("Blue: how much blue each pixel has")),
        ];
        const HSL: &[(&str, &str)] = &[HUE, SATURATION, ("L", tr_key("Lightness: from black, through color, to white"))];
        const HSV: &[(&str, &str)] = &[HUE, SATURATION, ("V", tr_key("Value: from black to full color"))];
        match self {
            CurveSpace::Rgb => RGB,
            CurveSpace::Hsl => HSL,
            CurveSpace::Hsv => HSV,
        }
    }
}

/// Input → output points, both 0–1, sorted by x. Fixed size so that `Adjust` stays `Copy`;
/// slots past `n` stay zero, so `==` compares only real points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    n: usize,
    points: [[f32; 2]; MAX_POINTS],
}

impl Default for Curve {
    /// The diagonal, which changes nothing.
    fn default() -> Self {
        let mut points = [[0.0; 2]; MAX_POINTS];
        points[1] = [1.0, 1.0];
        Curve { n: 2, points }
    }
}

impl Curve {
    fn points(&self) -> &[[f32; 2]] {
        &self.points[..self.n]
    }

    /// Adds a point between the end points and returns its index, or None if there's no room.
    fn add(&mut self, x: f32, y: f32) -> Option<usize> {
        let i = self.points().iter().position(|p| p[0] > x)?;
        if i == 0 || self.n == MAX_POINTS || x - self.points[i - 1][0] < MIN_GAP || self.points[i][0] - x < MIN_GAP {
            return None;
        }
        self.points.copy_within(i..self.n, i + 1);
        self.points[i] = [x, y.clamp(0.0, 1.0)];
        self.n += 1;
        Some(i)
    }

    /// Removes a point; the end points stay.
    fn remove(&mut self, i: usize) {
        if i == 0 || i + 1 >= self.n {
            return;
        }
        self.points.copy_within(i + 1..self.n, i);
        self.n -= 1;
        self.points[self.n] = [0.0; 2];
    }

    /// Moves a point, stopping short of its neighbours.
    fn move_point(&mut self, i: usize, x: f32, y: f32) {
        let lo = if i == 0 { 0.0 } else { self.points[i - 1][0] + MIN_GAP };
        let hi = if i + 1 == self.n { 1.0 } else { self.points[i + 1][0] - MIN_GAP };
        self.points[i] = [x.clamp(lo, hi), y.clamp(0.0, 1.0)];
    }

    /// The curve at `N` evenly spaced inputs: a monotone cubic spline (Fritsch–Carlson), which
    /// can't overshoot between points as a plain cubic would, and flat past the end points.
    fn table<const N: usize>(&self) -> [f32; N] {
        let p = self.points();
        let slopes: Vec<f32> = p.windows(2).map(|w| (w[1][1] - w[0][1]) / (w[1][0] - w[0][0])).collect();
        let mut m = vec![0.0; p.len()];
        m[0] = slopes[0];
        m[p.len() - 1] = slopes[slopes.len() - 1];
        for i in 1..p.len() - 1 {
            m[i] = if slopes[i - 1] * slopes[i] <= 0.0 { 0.0 } else { (slopes[i - 1] + slopes[i]) / 2.0 };
        }
        for (i, &d) in slopes.iter().enumerate() {
            if d == 0.0 {
                (m[i], m[i + 1]) = (0.0, 0.0);
            } else {
                let (a, b) = (m[i] / d, m[i + 1] / d);
                let s = a.hypot(b);
                if s > 3.0 {
                    (m[i], m[i + 1]) = (3.0 * a / s * d, 3.0 * b / s * d);
                }
            }
        }
        let mut seg = 0;
        std::array::from_fn(|k| {
            let x = k as f32 / (N - 1) as f32;
            if x <= p[0][0] {
                return p[0][1];
            }
            if x >= p[p.len() - 1][0] {
                return p[p.len() - 1][1];
            }
            while x > p[seg + 1][0] {
                seg += 1;
            }
            let (a, b) = (p[seg], p[seg + 1]);
            let w = b[0] - a[0];
            let t = (x - a[0]) / w;
            let (t2, t3) = (t * t, t * t * t);
            let y = (2.0 * t3 - 3.0 * t2 + 1.0) * a[1] + (t3 - 2.0 * t2 + t) * w * m[seg] + (3.0 * t2 - 2.0 * t3) * b[1] + (t3 - t2) * w * m[seg + 1];
            y.clamp(0.0, 1.0)
        })
    }
}

/// The curves of each colour space. Only `space`'s affect the image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Curves {
    pub space: CurveSpace,
    /// RGB (applied after the others), R, G, B.
    rgb: [Curve; 4],
    hsl: [Curve; 3],
    hsv: [Curve; 3],
}

impl Curves {
    fn active(&self) -> &[Curve] {
        match self.space {
            CurveSpace::Rgb => &self.rgb,
            CurveSpace::Hsl => &self.hsl,
            CurveSpace::Hsv => &self.hsv,
        }
    }

    fn active_mut(&mut self) -> &mut [Curve] {
        match self.space {
            CurveSpace::Rgb => &mut self.rgb,
            CurveSpace::Hsl => &mut self.hsl,
            CurveSpace::Hsv => &mut self.hsv,
        }
    }

    /// Drops the other spaces' curves, and the space itself if its curves change nothing.
    pub fn keep_active(&mut self) {
        let kept = *self;
        *self = Curves::default();
        if kept.active().iter().any(|c| *c != Curve::default()) {
            self.space = kept.space;
            self.active_mut().copy_from_slice(kept.active());
        }
    }

    /// Per-channel lookup tables for the RGB curves, the identity in other spaces.
    pub fn rgb_luts(&self) -> [[u8; 256]; 3] {
        let curves = if self.space == CurveSpace::Rgb { self.rgb } else { [Curve::default(); 4] };
        let master = curves[0].table::<256>();
        std::array::from_fn(|c| {
            let channel = curves[c + 1].table::<256>();
            std::array::from_fn(|v| (master[(channel[v] * 255.0).round() as usize] * 255.0).round() as u8)
        })
    }

    /// What HSL and HSV curves need per pixel; None when there's nothing to do.
    pub fn color_tables(&self) -> Option<ColorTables> {
        if self.space == CurveSpace::Rgb || self.active().iter().all(|c| *c == Curve::default()) {
            return None;
        }
        let tables = std::array::from_fn(|c| (self.active()[c] != Curve::default()).then(|| self.active()[c].table::<COLOR_TABLE>()));
        Some(ColorTables { space: self.space, tables })
    }
}

const COLOR_TABLE: usize = 1024;

/// One table per HSL/HSV channel; None leaves that channel exact, so a saturation curve doesn't quantise hue.
pub struct ColorTables {
    space: CurveSpace,
    tables: [Option<[f32; COLOR_TABLE]>; 3],
}

impl ColorTables {
    pub fn map(&self, px: [u8; 3]) -> [u8; 3] {
        let mut c = to_space(self.space, px.map(|v| v as f32 / 255.0));
        for (value, table) in c.iter_mut().zip(&self.tables) {
            if let Some(table) = table {
                *value = table[(*value * (COLOR_TABLE - 1) as f32 + 0.5) as usize];
            }
        }
        // A hue of 1 is a hue of 0.
        if c[0] >= 1.0 {
            c[0] = 0.0;
        }
        // Not `round`: it's a library call on baseline x86-64, and this runs per pixel. The cast clamps.
        from_space(self.space, c).map(|v| (v * 255.0 + 0.5) as u8)
    }
}

/// RGB (all 0–1) to HSL or HSV (all 0–1).
fn to_space(space: CurveSpace, [r, g, b]: [f32; 3]) -> [f32; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        let h = (g - b) / d;
        (if h < 0.0 { h + 6.0 } else { h }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    if space == CurveSpace::Hsl {
        let l = (max + min) / 2.0;
        [h, if d == 0.0 { 0.0 } else { (d / (1.0 - (2.0 * l - 1.0).abs())).min(1.0) }, l]
    } else {
        [h, if max == 0.0 { 0.0 } else { d / max }, max]
    }
}

/// The inverse of `to_space`.
fn from_space(space: CurveSpace, [h, s, v]: [f32; 3]) -> [f32; 3] {
    let (c, m) = if space == CurveSpace::Hsl {
        let c = (1.0 - (2.0 * v - 1.0).abs()) * s;
        (c, v - c / 2.0)
    } else {
        (v * s, v - v * s)
    };
    let sector = (h * 6.0) as u32;
    let f = h * 6.0 - sector as f32;
    let x = c * if sector.is_multiple_of(2) { f } else { 1.0 - f };
    let [r, g, b] = match sector {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    [r + m, g + m, b + m]
}

/// What the histogram of `channel` looks like going into the curves: `crop` of `frame`, sampled,
/// with the Adjust Color sliders applied.
fn histogram(frame: &Frame, crop: Rect, adjust: &Adjust, channel: usize) -> [u32; 256] {
    let channels = if frame.alpha { 4 } else { 3 };
    let (x0, x1) = (crop.min.x.max(0.0) as usize, (crop.max.x as usize).min(frame.w as usize));
    let (y0, y1) = (crop.min.y.max(0.0) as usize, (crop.max.y as usize).min(frame.h as usize));
    let step = ((x1.saturating_sub(x0) * y1.saturating_sub(y0)) as f32 / HIST_SAMPLES as f32).sqrt().ceil().max(1.0) as usize;
    let mut sample = Vec::with_capacity(HIST_SAMPLES * 3);
    for y in (y0..y1).step_by(step) {
        for x in (x0..x1).step_by(step) {
            sample.extend_from_slice(&frame.pixels[(y * frame.w as usize + x) * channels..][..3]);
        }
    }
    let space = adjust.curves.space;
    Adjust { curves: Default::default(), saturation: 0.0, ..*adjust }.apply(&mut sample, 3);
    let mut hist = [0; 256];
    for px in sample.as_chunks::<3>().0 {
        let bin = match (space, channel) {
            (CurveSpace::Rgb, 0) => (77 * px[0] as usize + 150 * px[1] as usize + 29 * px[2] as usize) >> 8,
            (CurveSpace::Rgb, c) => px[c - 1] as usize,
            (_, c) => (to_space(space, px.map(|v| v as f32 / 255.0))[c] * 255.0).round() as usize,
        };
        hist[bin] += 1;
    }
    hist
}

/// The Adjust Curves panel's own state.
#[derive(Default)]
pub struct CurvePanel {
    channel: usize,
    /// The point being dragged.
    dragging: Option<usize>,
    /// For this space and channel.
    hist: Option<(CurveSpace, usize, [u32; 256])>,
}

/// The Adjust Curves panel body: changes `a` live. Returns Some(true) for Done, Some(false) for Cancel.
pub fn curves_ui(ui: &mut egui::Ui, a: &mut Adjust, panel: &mut CurvePanel, frame: Option<&Frame>, crop: Rect) -> Option<bool> {
    ui.set_max_width(GRAPH_SIZE);
    ui.horizontal(|ui| {
        ui.label(tr("Color space"));
        egui::ComboBox::from_id_salt("curve-space").selected_text(a.curves.space.name()).show_ui(ui, |ui| {
            for space in [CurveSpace::Rgb, CurveSpace::Hsl, CurveSpace::Hsv] {
                if ui.selectable_value(&mut a.curves.space, space, space.name()).changed() {
                    panel.channel = 0;
                }
            }
        });
    });
    ui.horizontal(|ui| {
        for (i, (name, _)) in a.curves.space.channels().iter().enumerate() {
            ui.selectable_value(&mut panel.channel, i, *name);
        }
    });
    ui.weak(tr(a.curves.space.channels()[panel.channel].1));
    let key = (a.curves.space, panel.channel);
    let hist = match (&panel.hist, frame) {
        (Some((space, channel, hist)), _) if (*space, *channel) == key => Some(*hist),
        (_, Some(frame)) => {
            let hist = histogram(frame, crop, a, panel.channel);
            panel.hist = Some((key.0, key.1, hist));
            Some(hist)
        }
        _ => None,
    };
    graph(ui, a, panel, hist);
    ui.weak(tr("Click to add a point, drag to move it, right-click to remove it"));
    if ui.button(tr("Reset")).clicked() {
        a.curves.active_mut().fill(Curve::default());
    }
    ui.add_space(6.0);
    done_cancel(ui)
}

fn graph(ui: &mut egui::Ui, a: &mut Adjust, panel: &mut CurvePanel, hist: Option<[u32; 256]>) {
    let (resp, painter) = ui.allocate_painter(Vec2::splat(GRAPH_SIZE), Sense::click_and_drag());
    let r = resp.rect;
    let to_screen = |[x, y]: [f32; 2]| Pos2::new(r.left() + x * r.width(), r.bottom() - y * r.height());
    let from_screen = |p: Pos2| [(p.x - r.left()) / r.width(), (r.bottom() - p.y) / r.height()];
    // Greys have no hue: a saturation curve lifted at 0 would turn them all red.
    let left_pinned = a.curves.space != CurveSpace::Rgb && panel.channel == 1;
    let curve = &mut a.curves.active_mut()[panel.channel];
    let near = |curve: &Curve, p: Pos2| curve.points().iter().position(|&q| to_screen(q).distance(p) < GRAB);

    if resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        panel.dragging = near(curve, p).or_else(|| {
            let [x, y] = from_screen(p);
            curve.add(x, y)
        });
    }
    if resp.dragged_by(egui::PointerButton::Primary)
        && let (Some(i), Some(p)) = (panel.dragging, resp.interact_pointer_pos())
        && !(left_pinned && i == 0)
    {
        let [x, y] = from_screen(p);
        curve.move_point(i, x, y);
    }
    if resp.drag_stopped() {
        panel.dragging = None;
    }
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.clicked() && near(curve, p).is_none() {
            let [x, y] = from_screen(p);
            curve.add(x, y);
        }
        if resp.secondary_clicked()
            && let Some(i) = near(curve, p)
        {
            curve.remove(i);
        }
    }

    let visuals = ui.visuals();
    painter.rect_filled(r, 0.0, visuals.extreme_bg_color);
    if let Some(hist) = hist {
        // The square root keeps a tall spike from flattening everything else.
        let peak = (hist.iter().copied().max().unwrap_or(0).max(1) as f32).sqrt();
        let stroke = Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.5));
        for (i, &count) in hist.iter().enumerate() {
            let x = i as f32 / 255.0;
            painter.line_segment([to_screen([x, 0.0]), to_screen([x, (count as f32).sqrt() / peak])], stroke);
        }
    }
    let faint = Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.3));
    for i in 1..4 {
        let t = i as f32 / 4.0;
        painter.line_segment([to_screen([t, 0.0]), to_screen([t, 1.0])], faint);
        painter.line_segment([to_screen([0.0, t]), to_screen([1.0, t])], faint);
    }
    painter.line_segment([to_screen([0.0, 0.0]), to_screen([1.0, 1.0])], faint);
    let color = visuals.strong_text_color();
    let line = curve.table::<256>().iter().enumerate().map(|(i, &y)| to_screen([i as f32 / 255.0, y])).collect();
    painter.add(Shape::line(line, Stroke::new(1.5, color)));
    for &p in curve.points() {
        painter.circle_filled(to_screen(p), 4.0, color);
    }
}

#[cfg(test)]
impl Curves {
    /// A bent HSL curve on all three channels.
    pub fn sample() -> Self {
        let mut curves = Curves { space: CurveSpace::Hsl, ..Curves::default() };
        for curve in &mut curves.hsl {
            curve.add(0.5, 0.6);
        }
        curves
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn with(space: CurveSpace, channel: usize, points: &[[f32; 2]]) -> Adjust {
        let mut a = Adjust::default();
        a.curves.space = space;
        let curve = &mut a.curves.active_mut()[channel];
        curve.move_point(0, points[0][0], points[0][1]);
        curve.move_point(1, points[points.len() - 1][0], points[points.len() - 1][1]);
        for p in &points[1..points.len() - 1] {
            curve.add(p[0], p[1]).unwrap();
        }
        a
    }

    fn apply(a: Adjust, mut px: Vec<u8>, channels: usize) -> Vec<u8> {
        a.apply(&mut px, channels);
        px
    }

    #[test]
    fn default_changes_nothing() {
        let table = Curve::default().table::<256>();
        assert!(table.iter().enumerate().all(|(i, y)| (y - i as f32 / 255.0).abs() < 1e-5));
        assert!(Adjust::default().is_identity());
        // Selecting a space without drawing a curve changes nothing either.
        let mut a = Adjust::default();
        a.curves.space = CurveSpace::Hsv;
        assert_eq!(apply(a, vec![200, 100, 50], 3), [200, 100, 50]);
    }

    #[test]
    fn spline_hits_its_points_without_overshooting() {
        let mut c = Curve::default();
        c.add(0.2, 0.0).unwrap();
        c.add(0.25, 1.0).unwrap();
        c.add(0.7, 1.0).unwrap();
        let t = c.table::<1024>();
        for &[x, y] in c.points() {
            assert!((t[(x * 1023.0).round() as usize] - y).abs() < 0.02, "{x} {y}");
        }
        // Flat between equal points, and never decreasing on the way up.
        assert!(t[300..700].iter().all(|&y| y == 1.0));
        assert!(t[..300].windows(2).all(|w| w[1] >= w[0]));
        // Flat past moved end points.
        let mut c = Curve::default();
        c.move_point(0, 0.25, 0.25);
        c.move_point(1, 0.75, 0.75);
        let t = c.table::<256>();
        assert_eq!((t[0], t[255]), (0.25, 0.75));
    }

    #[test]
    fn points_stay_in_order_and_ends_stay() {
        let mut c = Curve::default();
        assert_eq!(c.add(0.5, 0.5), Some(1));
        assert_eq!(c.add(0.5, 0.1), None, "too close");
        c.move_point(1, 2.0, -1.0);
        assert_eq!(c.points()[1], [1.0 - MIN_GAP, 0.0]);
        c.remove(0);
        c.remove(2);
        assert_eq!(c.points().len(), 3);
        c.remove(1);
        assert_eq!(c, Curve::default());
        // Outside moved end points there's nothing to add to.
        c.move_point(1, 0.5, 0.5);
        assert_eq!(c.add(0.75, 0.5), None);
        c = Curve::default();
        for i in 1..=20 {
            c.add(i as f32 / 21.0, 0.5);
        }
        assert_eq!(c.points().len(), MAX_POINTS);
        assert!(c.points().windows(2).all(|w| w[0][0] < w[1][0]));
    }

    #[test]
    fn color_spaces_round_trip() {
        for space in [CurveSpace::Hsl, CurveSpace::Hsv] {
            for r in (0..=255).step_by(5) {
                for g in (0..=255).step_by(5) {
                    for b in (0..=255).step_by(5) {
                        let px = [r, g, b].map(|v| v as u8);
                        let c = to_space(space, px.map(|v| v as f32 / 255.0));
                        assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "{px:?} {c:?}");
                        let back = from_space(space, c).map(|v| (v * 255.0).round() as i32);
                        assert!((0..3).all(|i| (back[i] - px[i] as i32).abs() <= 1), "{px:?} {back:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn rgb_curves() {
        let up = [[0.0, 0.5], [1.0, 1.0]];
        let px = vec![0, 0, 0, 7];
        assert_eq!(apply(with(CurveSpace::Rgb, 1, &up), px.clone(), 4), [128, 0, 0, 7]);
        // The RGB curve applies after the channel's: 0 → 128 → 192.
        let mut both = with(CurveSpace::Rgb, 1, &up);
        both.curves.rgb[0] = with(CurveSpace::Rgb, 0, &up).curves.rgb[0];
        assert_eq!(apply(both, px, 4), [192, 128, 128, 7]);
    }

    #[test]
    fn hsl_and_hsv_curves() {
        // Every hue moved to 0.5: red becomes cyan, keeping saturation and lightness.
        let cyan = [[0.0, 0.5], [1.0, 0.5]];
        assert_eq!(apply(with(CurveSpace::Hsl, 0, &cyan), vec![255, 0, 0, 7], 4), [0, 255, 255, 7]);
        // A hue of 1 wraps round to red.
        assert_eq!(apply(with(CurveSpace::Hsv, 0, &[[0.0, 1.0], [1.0, 1.0]]), vec![0, 255, 0], 3), [255, 0, 0]);
        // Raising saturation away from 0 leaves greys alone.
        let more = [[0.0, 0.0], [0.25, 1.0], [1.0, 1.0]];
        let a = with(CurveSpace::Hsl, 1, &more);
        assert_eq!(apply(a, vec![90, 90, 90], 3), [90, 90, 90]);
        let vivid = apply(a, vec![150, 100, 100], 3);
        assert!(vivid[0] > 150 && vivid[1] < 100, "{vivid:?}");
        // Halving value and lightness both keep the hue, but differ on pure red: L 0.5 → 0.25.
        let half = [[0.0, 0.0], [1.0, 0.5]];
        assert_eq!(apply(with(CurveSpace::Hsv, 2, &half), vec![255, 0, 0], 3), [128, 0, 0]);
        assert_eq!(apply(with(CurveSpace::Hsl, 2, &half), vec![255, 0, 0], 3), [128, 0, 0]);
        assert_eq!(apply(with(CurveSpace::Hsl, 2, &half), vec![255, 255, 255], 3), [128, 128, 128]);
    }

    #[test]
    fn only_the_selected_space_applies() {
        let up = [[0.0, 0.5], [1.0, 1.0]];
        let mut a = with(CurveSpace::Rgb, 1, &up);
        a.curves.space = CurveSpace::Hsl;
        assert_eq!(apply(a, vec![0, 0, 0], 3), [0, 0, 0]);
        // Back in RGB the curve is still there, until Done keeps only the selected space.
        a.curves.space = CurveSpace::Rgb;
        assert_eq!(apply(a, vec![0, 0, 0], 3), [128, 0, 0]);
        a.curves.space = CurveSpace::Hsl;
        a.curves.keep_active();
        assert!(a.is_identity());
        let mut hsl = with(CurveSpace::Hsl, 2, &up);
        hsl.curves.rgb[1] = with(CurveSpace::Rgb, 1, &up).curves.rgb[1];
        hsl.curves.keep_active();
        assert_eq!(hsl.curves.space, CurveSpace::Hsl);
        assert_eq!(hsl.curves.rgb, [Curve::default(); 4]);
        assert_ne!(hsl.curves.hsl[2], Curve::default());
    }

    /// Plays pointer events into `graph`, one frame per batch.
    struct Mouse {
        ctx: egui::Context,
        a: Adjust,
        panel: CurvePanel,
        origin: Pos2,
    }

    impl Mouse {
        fn new(a: Adjust, channel: usize) -> Self {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::markup::font_definitions());
            let mut mouse = Mouse { ctx, a, panel: CurvePanel { channel, ..CurvePanel::default() }, origin: Pos2::ZERO };
            mouse.frame(vec![]);
            mouse
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
            input.events = events;
            let ctx = self.ctx.clone();
            let mut output = ctx.run_ui(input, |ui| {
                self.origin = ui.next_widget_position();
                graph(ui, &mut self.a, &mut self.panel, None);
            });
            output.textures_delta.clear();
        }

        fn at(&self, x: f32, y: f32) -> Pos2 {
            self.origin + Vec2::new(x, 1.0 - y) * GRAPH_SIZE
        }

        fn button(&mut self, pos: Pos2, button: egui::PointerButton, pressed: bool) {
            self.frame(vec![egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE }]);
        }

        fn move_to(&mut self, pos: Pos2) {
            self.frame(vec![egui::Event::PointerMoved(pos)]);
        }

        fn click(&mut self, x: f32, y: f32, button: egui::PointerButton) {
            let pos = self.at(x, y);
            self.move_to(pos);
            self.button(pos, button, true);
            self.button(pos, button, false);
            self.frame(vec![]);
        }

        fn drag(&mut self, from: [f32; 2], to: [f32; 2]) {
            let (from, to) = (self.at(from[0], from[1]), self.at(to[0], to[1]));
            self.move_to(from);
            self.button(from, egui::PointerButton::Primary, true);
            self.move_to((from + to.to_vec2()) / 2.0);
            self.move_to(to);
            self.button(to, egui::PointerButton::Primary, false);
            self.frame(vec![]);
        }

        fn points(&self, channel: usize) -> Vec<[f32; 2]> {
            self.a.curves.active()[channel].points().to_vec()
        }
    }

    #[test]
    fn graph_clicks_and_drags() {
        let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 0.02 && (a[1] - b[1]).abs() < 0.02;
        let mut m = Mouse::new(Adjust::default(), 0);
        m.click(0.5, 0.75, egui::PointerButton::Primary);
        let p = m.points(0);
        assert_eq!(p.len(), 3);
        assert!(close(p[1], [0.5, 0.75]), "{p:?}");
        // Dragging a point moves it instead of adding one.
        m.drag([0.5, 0.75], [0.6, 0.25]);
        let p = m.points(0);
        assert_eq!(p.len(), 3);
        assert!(close(p[1], [0.6, 0.25]), "{p:?}");
        // Dragging empty space adds a point there and follows the pointer.
        m.drag([0.3, 0.9], [0.2, 0.8]);
        let p = m.points(0);
        assert_eq!(p.len(), 4);
        assert!(close(p[1], [0.2, 0.8]), "{p:?}");
        // End points move, and don't go away on a right-click.
        m.drag([1.0, 1.0], [0.9, 0.7]);
        assert!(close(m.points(0)[3], [0.9, 0.7]));
        m.click(0.9, 0.7, egui::PointerButton::Secondary);
        assert_eq!(m.points(0).len(), 4);
        m.click(0.6, 0.25, egui::PointerButton::Secondary);
        m.click(0.2, 0.8, egui::PointerButton::Secondary);
        assert_eq!(m.points(0).len(), 2);
    }

    #[test]
    fn saturation_keeps_its_left_end_at_zero() {
        for space in [CurveSpace::Hsl, CurveSpace::Hsv] {
            let mut a = Adjust::default();
            a.curves.space = space;
            let mut m = Mouse::new(a, 1);
            m.drag([0.0, 0.0], [0.0, 0.5]);
            assert_eq!(m.points(1)[0], [0.0, 0.0]);
            // Hue's left end isn't pinned.
            m.panel.channel = 0;
            m.drag([0.0, 0.0], [0.0, 0.5]);
            assert!(m.points(0)[0][1] > 0.4);
        }
    }

    #[test]
    fn histogram_follows_the_channel() {
        let frame = Frame { w: 2, h: 1, pixels: vec![255, 0, 0, 7, 0, 0, 255, 7], alpha: true, delay: Duration::ZERO };
        let crop = Rect::from_min_size(Pos2::ZERO, Vec2::new(2.0, 1.0));
        let mut a = Adjust::default();
        let red = histogram(&frame, crop, &a, 1);
        assert_eq!((red[255], red[0]), (1, 1));
        a.curves.space = CurveSpace::Hsv;
        let hue = histogram(&frame, crop, &a, 0);
        assert_eq!((hue[0], hue[170]), (1, 1));
        // The sliders go first: two stops down halves value twice.
        a.exposure = -2.0;
        assert_eq!(histogram(&frame, crop, &a, 2)[64], 2);
    }
}
