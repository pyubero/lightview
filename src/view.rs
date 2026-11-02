use eframe::egui::{Pos2, Rect, Vec2};

/// How the image maps onto the viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    Fit,
    /// `scale` is physical px per image px; `center` is the image point at the viewport centre.
    Manual { scale: f32, center: Pos2 },
}

const MAX_SCALE: f32 = 32.0;

/// Physical px per image px that fits the whole image in `viewport`.
pub fn fit_scale(img: Vec2, viewport: Rect, ppp: f32) -> f32 {
    (viewport.width() / img.x).min(viewport.height() / img.y) * ppp
}

/// Turns `view` into concrete (scale, center), with the pan clamped so no gap shows
/// on an axis the image overflows, and the image centred on an axis it doesn't.
fn resolve(view: View, img: Vec2, viewport: Rect, ppp: f32) -> (f32, Pos2) {
    let (scale, center) = match view {
        View::Fit => (fit_scale(img, viewport, ppp), (img / 2.0).to_pos2()),
        View::Manual { scale, center } => (scale, center),
    };
    // Half the viewport, in image px.
    let half = viewport.size() * ppp / scale / 2.0;
    let clamp = |c: f32, half: f32, len: f32| if half * 2.0 >= len { len / 2.0 } else { c.clamp(half, len - half) };
    (scale, Pos2::new(clamp(center.x, half.x, img.x), clamp(center.y, half.y, img.y)))
}

/// The current zoom: physical px per image px, so 1.0 is actual size.
pub fn scale(view: View, img: Vec2, viewport: Rect, ppp: f32) -> f32 {
    resolve(view, img, viewport, ppp).0
}

/// Where the visible part of the image goes on screen (points) and which part of the image it is (image px).
pub fn layout(view: View, img: Vec2, viewport: Rect, ppp: f32) -> (Rect, Rect) {
    let (scale, center) = resolve(view, img, viewport, ppp);
    let pts_per_px = scale / ppp;
    let image_rect = Rect::from_min_size(viewport.center() - center.to_vec2() * pts_per_px, img * pts_per_px);
    let dest = image_rect.intersect(viewport);
    let crop = Rect::from_min_size(((dest.min - image_rect.min) / pts_per_px).to_pos2(), dest.size() / pts_per_px);
    // Float rounding can push the crop a hair outside the image, which the resizer rejects.
    (dest, crop.intersect(Rect::from_min_size(Pos2::ZERO, img)))
}

/// Zooms by `factor`, keeping the image point under `cursor` (points) where it is.
/// Zoom stays between min(fit, 1:1) and 32×; zooming out to the fit scale returns `Fit`.
pub fn zoom_at(view: View, factor: f32, cursor: Pos2, img: Vec2, viewport: Rect, ppp: f32) -> View {
    let (scale, center) = resolve(view, img, viewport, ppp);
    let fit = fit_scale(img, viewport, ppp);
    let new_scale = (scale * factor).clamp(fit.min(1.0), MAX_SCALE.max(fit));
    if new_scale <= fit && fit <= 1.0 {
        return View::Fit;
    }
    let offset = (cursor - viewport.center()) * ppp;
    let point = center + offset / scale;
    let view = View::Manual { scale: new_scale, center: point - offset / new_scale };
    let (scale, center) = resolve(view, img, viewport, ppp);
    View::Manual { scale, center }
}

/// Pans by `delta` points (the drag direction). `Fit` has nothing to pan.
pub fn pan(view: View, delta: Vec2, img: Vec2, viewport: Rect, ppp: f32) -> View {
    match view {
        View::Fit => View::Fit,
        View::Manual { scale, center } => {
            let (scale, center) = resolve(View::Manual { scale, center: center - delta * ppp / scale }, img, viewport, ppp);
            View::Manual { scale, center }
        }
    }
}

/// The user's view-only rotate/flip: flip horizontally first (if `flip`), then rotate `rot` quarter turns clockwise.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transform {
    pub rot: u8,
    pub flip: bool,
}

impl Transform {
    pub fn rotate_right(self) -> Self {
        Transform { rot: (self.rot + 1) % 4, ..self }
    }

    pub fn rotate_left(self) -> Self {
        Transform { rot: (self.rot + 3) % 4, ..self }
    }

    /// Flips what's on screen: H·R^r·F^f = R^(−r)·F^(f+1).
    pub fn flip_horizontal(self) -> Self {
        Transform { rot: (4 - self.rot) % 4, flip: !self.flip }
    }

    /// V = R²·H, so V·R^r·F^f = R^(2−r)·F^(f+1).
    pub fn flip_vertical(self) -> Self {
        Transform { rot: (6 - self.rot) % 4, flip: !self.flip }
    }

    /// Size after the transform (quarter turns swap width and height).
    pub fn size(self, size: Vec2) -> Vec2 {
        if self.rot % 2 == 1 { Vec2::new(size.y, size.x) } else { size }
    }

    /// The source-image point that ends up at `p` (in transformed coordinates) for a `src`-sized image.
    pub fn source_point(self, mut p: Pos2, src: Vec2) -> Pos2 {
        let mut dims = self.size(src);
        for _ in 0..self.rot {
            // Undo one clockwise turn: (x, y) in (w, h) came from (y, w − x) in (h, w).
            p = Pos2::new(p.y, dims.x - p.x);
            dims = Vec2::new(dims.y, dims.x);
        }
        if self.flip { Pos2::new(src.x - p.x, p.y) } else { p }
    }

    /// The source-image rect that ends up at `rect` (in transformed coordinates).
    pub fn to_source(self, rect: Rect, src: Vec2) -> Rect {
        Rect::from_two_pos(self.source_point(rect.min, src), self.source_point(rect.max, src))
    }

    pub fn apply(self, mut img: image::DynamicImage) -> image::DynamicImage {
        if self.flip {
            img = img.fliph();
        }
        match self.rot {
            1 => img.rotate90(),
            2 => img.rotate180(),
            3 => img.rotate270(),
            _ => img,
        }
    }
}

/// The image px under screen point `p`, for a `layout` result (`dest`, `crop`).
pub fn to_image(dest: Rect, crop: Rect, p: Pos2) -> Pos2 {
    crop.min + (p - dest.min) * (crop.width() / dest.width())
}

/// The screen point showing image px `p`, for a `layout` result (`dest`, `crop`).
pub fn to_screen(dest: Rect, crop: Rect, p: Pos2) -> Pos2 {
    dest.min + (p - crop.min) * (dest.width() / crop.width())
}

/// 1:1 pixels, centred.
pub fn actual_size(img: Vec2) -> View {
    View::Manual { scale: 1.0, center: (img / 2.0).to_pos2() }
}

/// The longer side of the navigator thumbnail, in points.
const NAVIGATOR_SIZE: f32 = 160.0;
/// Below this many screen px per image px, a pixel grid would be nearly solid.
const GRID_MIN_SCALE: f32 = 8.0;

/// Where the navigator goes for an image of size `img`: `viewport`'s bottom-right corner.
pub fn navigator_rect(img: Vec2, viewport: Rect) -> Rect {
    let size = (img * (NAVIGATOR_SIZE / img.max_elem())).max(Vec2::splat(1.0));
    Rect::from_min_size(viewport.max - size - Vec2::splat(12.0), size)
}

/// Whether `crop`, what's shown of an image of size `img`, leaves part of it out.
pub fn zoomed_in(crop: Rect, img: Vec2) -> bool {
    crop.width() < img.x - 0.5 || crop.height() < img.y - 0.5
}

/// The lines between the pixels on screen, when they're big enough to tell apart.
pub fn grid_lines(dest: Rect, crop: Rect, ppp: f32) -> Vec<[Pos2; 2]> {
    if dest.width() / crop.width() * ppp < GRID_MIN_SCALE {
        return Vec::new();
    }
    let at = |x: f32, y: f32| to_screen(dest, crop, Pos2::new(x, y));
    let columns = (crop.left().ceil() as u32..=crop.right().floor() as u32).map(|x| [Pos2::new(at(x as f32, 0.0).x, dest.top()), Pos2::new(at(x as f32, 0.0).x, dest.bottom())]);
    let rows = (crop.top().ceil() as u32..=crop.bottom().floor() as u32).map(|y| [Pos2::new(dest.left(), at(0.0, y as f32).y), Pos2::new(dest.right(), at(0.0, y as f32).y)]);
    columns.chain(rows).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h))
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1))
    }

    #[test]
    fn navigator_keeps_the_images_shape_in_the_corner() {
        let viewport = panel(800.0, 600.0);
        let r = navigator_rect(Vec2::new(4000.0, 2000.0), viewport);
        assert_eq!(r.size(), Vec2::new(160.0, 80.0));
        assert!(viewport.contains_rect(r) && r.max.x > 780.0 && r.max.y > 580.0);
        assert_eq!(navigator_rect(Vec2::new(10000.0, 1.0), viewport).height(), 1.0);
    }

    #[test]
    fn zoomed_in_means_part_of_it_is_cut_off() {
        let img = Vec2::new(100.0, 50.0);
        assert!(!zoomed_in(rect(0.0, 0.0, 100.0, 50.0), img));
        assert!(zoomed_in(rect(0.0, 0.0, 100.0, 40.0), img));
        assert!(zoomed_in(rect(10.0, 0.0, 60.0, 50.0), img));
    }

    #[test]
    fn the_grid_needs_big_pixels_and_sits_on_their_edges() {
        let dest = rect(0.0, 0.0, 100.0, 100.0);
        assert!(grid_lines(dest, rect(0.0, 0.0, 20.0, 20.0), 1.0).is_empty(), "5 px per pixel");
        assert_eq!(grid_lines(dest, rect(0.0, 0.0, 20.0, 20.0), 2.0).len(), 42, "10 px per pixel, with 2 ppp");
        // Panned by half a pixel: 10 px per pixel, so edges at 5, 15…
        let lines = grid_lines(dest, rect(3.5, 7.5, 13.5, 17.5), 1.0);
        assert_eq!(lines.len(), 20);
        assert_eq!(lines[0], [Pos2::new(5.0, 0.0), Pos2::new(5.0, 100.0)]);
        assert_eq!(lines[10], [Pos2::new(0.0, 5.0), Pos2::new(100.0, 5.0)]);
    }

    #[test]
    fn fit() {
        let full = |w, h| rect(0.0, 0.0, w, h);
        // Wide image in a tall panel: full width, centred vertically.
        assert_eq!(layout(View::Fit, Vec2::new(400.0, 200.0), panel(100.0, 300.0), 1.0), (rect(0.0, 125.0, 100.0, 175.0), full(400.0, 200.0)));
        // Tall image in a wide panel: full height, centred horizontally.
        assert_eq!(layout(View::Fit, Vec2::new(200.0, 400.0), panel(300.0, 100.0), 1.0), (rect(125.0, 0.0, 175.0, 100.0), full(200.0, 400.0)));
        // Small image is enlarged.
        assert_eq!(layout(View::Fit, Vec2::new(48.0, 36.0), panel(480.0, 360.0), 1.0).0, panel(480.0, 360.0));
        // HiDPI: the same rect in points.
        assert_eq!(layout(View::Fit, Vec2::new(480.0, 360.0), panel(480.0, 360.0), 2.0).0, panel(480.0, 360.0));
    }

    #[test]
    fn actual_size_centred_and_cropped() {
        // 1000×1000 image at 1:1 in a 100×100 panel shows the middle 100×100.
        let (dest, crop) = layout(actual_size(Vec2::splat(1000.0)), Vec2::splat(1000.0), panel(100.0, 100.0), 1.0);
        assert_eq!(dest, panel(100.0, 100.0));
        assert_eq!(crop, rect(450.0, 450.0, 550.0, 550.0));
        // At 2 ppp the 100-point panel is 200 physical px, so it shows 200×200 image px.
        let (_, crop) = layout(actual_size(Vec2::splat(1000.0)), Vec2::splat(1000.0), panel(100.0, 100.0), 2.0);
        assert_eq!(crop, rect(400.0, 400.0, 600.0, 600.0));
    }

    #[test]
    fn crop_stays_inside_image() {
        let bounds = |img: Vec2| Rect::from_min_size(Pos2::ZERO, img);
        for (w, h) in [(320.0, 240.0), (300.0, 200.0), (4000.0, 2252.0), (2268.0, 4032.0), (7.0, 3.0)] {
            let img = Vec2::new(w, h);
            for (vw, vh, ppp) in [(480.0, 360.0, 1.0), (477.0, 333.0, 1.25), (1920.0, 1053.0, 1.5)] {
                let vp = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(vw, vh));
                for view in [View::Fit, actual_size(img), zoom_at(View::Fit, 3.7, Pos2::new(vw * 0.9, vh * 0.1), img, vp, ppp)] {
                    let (_, crop) = layout(view, img, vp, ppp);
                    assert!(bounds(img).contains_rect(crop), "{img:?} {vp:?} {ppp} {view:?} → {crop:?}");
                }
            }
        }
    }

    /// A 3×2 image with distinct pixels, so any transform gives a distinct result.
    fn sample() -> image::DynamicImage {
        image::DynamicImage::ImageLuma8(image::GrayImage::from_fn(3, 2, |x, y| image::Luma([(y * 3 + x) as u8])))
    }

    #[test]
    fn transform_ops_match_image_ops() {
        type Op = (fn(Transform) -> Transform, fn(&image::DynamicImage) -> image::DynamicImage);
        let ops: [Op; 4] = [
            (Transform::rotate_right, |i| i.rotate90()),
            (Transform::rotate_left, |i| i.rotate270()),
            (Transform::flip_horizontal, |i| i.fliph()),
            (Transform::flip_vertical, |i| i.flipv()),
        ];
        // Every sequence of 3 operations: the composed transform equals applying the ops one by one.
        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    let (mut t, mut expected) = (Transform::default(), sample());
                    for i in [a, b, c] {
                        t = ops[i].0(t);
                        expected = ops[i].1(&expected);
                    }
                    assert_eq!(t.apply(sample()), expected, "ops {a} {b} {c} → {t:?}");
                }
            }
        }
    }

    #[test]
    fn to_source_maps_pixels() {
        let src = sample();
        let size = Vec2::new(3.0, 2.0);
        for rot in 0..4 {
            for flip in [false, true] {
                let t = Transform { rot, flip };
                let shown = t.apply(src.clone()).to_luma8();
                assert_eq!(t.size(size), Vec2::new(shown.width() as f32, shown.height() as f32));
                // Each displayed pixel's rect maps back to the source pixel with the same value.
                for (x, y, px) in shown.enumerate_pixels() {
                    let r = t.to_source(Rect::from_min_size(Pos2::new(x as f32, y as f32), Vec2::splat(1.0)), size);
                    assert_eq!(r.size(), Vec2::splat(1.0));
                    assert_eq!(src.to_luma8().get_pixel(r.min.x as u32, r.min.y as u32), px, "{t:?} at ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn screen_image_round_trip() {
        let img = Vec2::new(1000.0, 500.0);
        let vp = panel(300.0, 200.0);
        for view in [View::Fit, zoom_at(View::Fit, 3.0, Pos2::new(250.0, 50.0), img, vp, 1.5)] {
            let (dest, crop) = layout(view, img, vp, 1.5);
            for p in [Pos2::new(0.0, 0.0), Pos2::new(123.0, 77.0), Pos2::new(1000.0, 500.0)] {
                assert!((to_image(dest, crop, to_screen(dest, crop, p)) - p).length() < 1e-2, "{view:?} {p:?}");
            }
        }
        // Fit: the image's corners are the dest rect's.
        let (dest, crop) = layout(View::Fit, img, vp, 1.0);
        assert_eq!(to_screen(dest, crop, Pos2::new(1000.0, 500.0)), dest.max);
    }

    #[test]
    fn crop_at_2x() {
        let view = View::Manual { scale: 2.0, center: Pos2::new(100.0, 100.0) };
        let (dest, crop) = layout(view, Vec2::splat(400.0), panel(100.0, 100.0), 1.0);
        assert_eq!(dest, panel(100.0, 100.0));
        assert_eq!(crop, rect(75.0, 75.0, 125.0, 125.0));
    }

    #[test]
    fn pan_clamps() {
        let img = Vec2::new(1000.0, 50.0);
        let view = View::Manual { scale: 1.0, center: Pos2::new(500.0, 25.0) };
        // Dragging right past the left edge stops with the image's left edge at the viewport's.
        let View::Manual { center, .. } = pan(view, Vec2::new(10_000.0, 30.0), img, panel(100.0, 100.0), 1.0) else { panic!() };
        // x clamped to half the viewport; y centred because the image is shorter than the viewport.
        assert_eq!(center, Pos2::new(50.0, 25.0));
        assert_eq!(pan(View::Fit, Vec2::new(5.0, 5.0), img, panel(100.0, 100.0), 1.0), View::Fit);
    }

    #[test]
    fn zoom_keeps_cursor_point() {
        let img = Vec2::splat(1000.0);
        let vp = panel(100.0, 100.0);
        let cursor = Pos2::new(80.0, 30.0);
        let view = View::Manual { scale: 1.0, center: Pos2::new(500.0, 500.0) };
        let before = layout(view, img, vp, 1.0);
        let zoomed = zoom_at(view, 2.0, cursor, img, vp, 1.0);
        let after = layout(zoomed, img, vp, 1.0);
        let point = |(dest, crop): (Rect, Rect)| crop.min + (cursor - dest.min) * crop.width() / dest.width();
        assert!((point(before) - point(after)).length() < 1e-3);
    }

    #[test]
    fn zoom_limits() {
        let img = Vec2::splat(1000.0);
        let vp = panel(100.0, 100.0);
        // Zooming out of a big image ends at Fit.
        assert_eq!(zoom_at(actual_size(img), 0.001, vp.center(), img, vp, 1.0), View::Fit);
        // Zooming in stops at 32×.
        assert!(matches!(zoom_at(actual_size(img), 1000.0, vp.center(), img, vp, 1.0), View::Manual { scale: 32.0, .. }));
        // A small image (fit > 1) can zoom out to 1:1 but not below.
        let small = Vec2::splat(10.0);
        assert!(matches!(zoom_at(View::Fit, 0.001, vp.center(), small, vp, 1.0), View::Manual { scale: 1.0, .. }));
    }
}
