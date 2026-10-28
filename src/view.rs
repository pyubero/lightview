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

/// 1:1 pixels, centred.
pub fn actual_size(img: Vec2) -> View {
    View::Manual { scale: 1.0, center: (img / 2.0).to_pos2() }
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
