use crate::load::Frame;
use eframe::egui::Rect;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Adjust;
    use eframe::egui::{Pos2, Vec2};
    use std::time::Duration;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
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
}
