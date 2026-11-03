//! Resize Optimally: the intrinsic size, how far an image can shrink before it loses detail.
//!
//! Lanczos round trips (shrink, enlarge back) at 20 widths from w−1 down to w/8; the loss is the
//! 99th percentile of the luma difference; a two-line fit of loss against log width finds the
//! knee, and when the narrow side isn't 1.25× steeper than the wide side the image is native.
//! The estimate is iterated (resize to it, estimate again) to a fixed point, so running Resize
//! Optimally twice changes nothing.
//!
//! Found by experiment (resampler × measure × knee rule on known enlargements): thresholds on
//! the loss (RMSE, PSNR, SSIM…) drift 15–90% each time they're re-run, because a native image
//! loses detail gradually rather than at a cliff; mean-based measures are swamped by flat areas;
//! the two-line fit with p99 was the only rule that recognises an image already at its size.
//! Intrinsic size is local (one photo's tiles ranged from 11% to 72% of its width, flat sky read
//! as native), so it's measured on the most detailed 1024 px tile: the sharpest part sets the
//! size. Precision is the ~11% step between widths. Python's PIL and this code agree exactly.

use crate::load::Frame;
use eframe::egui::Rect;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::{Image, ImageRef}};

const SAMPLES: usize = 20;
const SPAN: f64 = 8.0;
const RATIO: f64 = 1.25;
const MAX_ROUNDS: usize = 5;
const TILE: u32 = 1024;

struct Grey {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

/// Intrinsic width ÷ width (at most 1) of the `crop` part of `frame`.
pub fn fraction(frame: &Frame, crop: Rect) -> f32 {
    let grey = luma(frame, crop);
    let (tw, th) = (grey.w.min(TILE), grey.h.min(TILE));
    let (x, y) = pick_tile(&grey, tw, th);
    let tile = sub(&grey, x, y, tw, th);
    intrinsic(&tile) as f32 / tw as f32
}

/// PIL "L" (ITU-R 601-2 luma, 16-bit fixed point) of the crop; alpha is ignored.
fn luma(frame: &Frame, crop: Rect) -> Grey {
    let ch = if frame.alpha { 4 } else { 3 };
    let x0 = (crop.min.x.max(0.0) as u32).min(frame.w - 1);
    let y0 = (crop.min.y.max(0.0) as u32).min(frame.h - 1);
    let w = ((crop.max.x.ceil() as u32).min(frame.w) - x0).max(1);
    let h = ((crop.max.y.ceil() as u32).min(frame.h) - y0).max(1);
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in y0..y0 + h {
        let row = &frame.pixels[((y * frame.w + x0) as usize) * ch..((y * frame.w + x0 + w) as usize) * ch];
        px.extend(row.chunks_exact(ch).map(|p| ((p[0] as u32 * 19595 + p[1] as u32 * 38470 + p[2] as u32 * 7471 + 0x8000) >> 16) as u8));
    }
    Grey { w, h, px }
}

/// The top-left corner of the `tw`×`th` tile with the most fine detail (mean |dx| + |dy|). Tiles
/// cover the image, the last row and column shifted inwards so all are full size.
// shortcut: a very noisy flat area could win; score a blurred copy if that shows up.
fn pick_tile(g: &Grey, tw: u32, th: u32) -> (u32, u32) {
    let starts = |n: u32, t: u32| {
        let count = n.div_ceil(t);
        (0..count).map(move |i| if count == 1 { 0 } else { i * (n - t) / (count - 1) })
    };
    let detail = |x0: u32, y0: u32| {
        let mut sum = 0u64;
        for y in y0..y0 + th - 1 {
            let row = &g.px[(y * g.w + x0) as usize..(y * g.w + x0 + tw) as usize];
            let next = &g.px[((y + 1) * g.w + x0) as usize..((y + 1) * g.w + x0 + tw) as usize];
            for x in 0..tw as usize - 1 {
                sum += row[x].abs_diff(row[x + 1]) as u64 + row[x].abs_diff(next[x]) as u64;
            }
        }
        sum
    };
    let tiles: Vec<(u32, u32)> = starts(g.h, th).flat_map(|y| starts(g.w, tw).map(move |x| (x, y))).collect();
    if tiles.len() == 1 {
        return tiles[0];
    }
    tiles.into_iter().max_by_key(|&(x, y)| detail(x, y)).unwrap()
}

fn sub(g: &Grey, x0: u32, y0: u32, w: u32, h: u32) -> Grey {
    let px = (y0..y0 + h).flat_map(|y| g.px[(y * g.w + x0) as usize..(y * g.w + x0 + w) as usize].iter().copied()).collect();
    Grey { w, h, px }
}

fn resize(r: &mut Resizer, g: &Grey, w: u32, h: u32) -> Grey {
    let src = ImageRef::new(g.w, g.h, &g.px, PixelType::U8).unwrap();
    let mut dst = Image::new(w, h, PixelType::U8);
    r.resize(&src, &mut dst, &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))).unwrap();
    Grey { w, h, px: dst.into_vec() }
}

/// 99th percentile of |a − b|, interpolated between ranks like numpy's default.
fn p99(a: &[u8], b: &[u8]) -> f64 {
    let mut hist = [0usize; 256];
    for (x, y) in a.iter().zip(b) {
        hist[x.abs_diff(*y) as usize] += 1;
    }
    let at = |k: usize| {
        let mut seen = 0;
        hist.iter().position(|&n| {
            seen += n;
            seen > k
        }).unwrap_or(255) as f64
    };
    let rank = 0.99 * (a.len() - 1) as f64;
    let (lo, frac) = (rank.floor() as usize, rank.fract());
    let v = at(lo);
    if frac == 0.0 { v } else { v + frac * (at(lo + 1) - v) }
}

/// Least-squares line through the points: (sum of squared residuals, slope).
fn fit(x: &[f64], y: &[f64]) -> (f64, f64) {
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let sxy: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
    let sxx: f64 = x.iter().map(|a| (a - mx) * (a - mx)).sum();
    let slope = sxy / sxx;
    (x.iter().zip(y).map(|(a, b)| (b - (my + slope * (a - mx))).powi(2)).sum(), slope)
}

/// One estimate: where the loss curve's two-line fit bends, or the full width when it doesn't.
fn estimate(g: &Grey) -> u32 {
    let (w, h) = (g.w, g.h);
    if w < 2 * SAMPLES as u32 {
        return w; // too small to sample
    }
    let (a, b) = (((w - 1) as f64).ln(), (w as f64 / SPAN).ln());
    let mut widths: Vec<u32> = (0..SAMPLES).map(|i| (a + (b - a) * i as f64 / (SAMPLES - 1) as f64).exp().round() as u32).collect();
    widths.dedup();
    // The round trips are independent: split them over the cores, each with its own Resizer.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let loss: Vec<f64> = std::thread::scope(|s| {
        let jobs: Vec<_> = widths.chunks(widths.len().div_ceil(threads)).map(|chunk| s.spawn(move || {
            let mut r = Resizer::new();
            chunk.iter().map(|&sw| {
                let sh = ((h as f64 * sw as f64 / w as f64).round() as u32).max(1);
                let small = resize(&mut r, g, sw, sh);
                p99(&g.px, &resize(&mut r, &small, w, h).px)
            }).collect::<Vec<_>>()
        })).collect();
        jobs.into_iter().flat_map(|j| j.join().unwrap()).collect()
    });
    let x: Vec<f64> = widths.iter().map(|&v| -(v as f64).log2()).collect();
    let mut best = (f64::INFINITY, 0, 0.0, 0.0);
    for k in 2..widths.len() - 2 {
        let (sse_a, wide) = fit(&x[..=k], &loss[..=k]);
        let (sse_b, narrow) = fit(&x[k..], &loss[k..]);
        if sse_a + sse_b < best.0 {
            best = (sse_a + sse_b, k, wide, narrow);
        }
    }
    let (_, k, wide, narrow) = best;
    if narrow > RATIO * wide.max(1e-12) { widths[k] } else { w }
}

/// The fixed point of estimate → resize to it → estimate: the intrinsic width of `g`.
fn intrinsic(g: &Grey) -> u32 {
    let mut r = Resizer::new();
    let mut cur = None::<Grey>;
    for _ in 0..MAX_ROUNDS {
        let c = cur.as_ref().unwrap_or(g);
        let n = estimate(c);
        if n >= c.w {
            return c.w;
        }
        let nh = ((g.h as f64 * n as f64 / g.w as f64).round() as u32).max(1);
        cur = Some(resize(&mut r, c, n, nh));
    }
    cur.map_or(g.w, |c| c.w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Pos2, Vec2};

    /// Seeded rectangles from 2 px to a quarter of the image, sharp to the pixel: detail at every
    /// scale like a real image (pure noise loses everything by half size, which no photo does).
    fn detailed(w: u32, h: u32) -> Grey {
        let mut s = 12345u32;
        let mut next = |n: u32| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) % n.max(1)
        };
        let mut px = vec![128u8; (w * h) as usize];
        for _ in 0..w * h / 100 {
            let size = 2u32 << next(6).min((w / 4).ilog2());
            let (rw, rh, v) = (size / 2 + next(size), size / 2 + next(size), next(256) as u8);
            let (x0, y0) = (next(w), next(h));
            for y in y0..(y0 + rh).min(h) {
                px[(y * w + x0) as usize..(y * w + (x0 + rw).min(w)) as usize].fill(v);
            }
        }
        Grey { w, h, px }
    }

    fn frame(g: &Grey) -> Frame {
        Frame { w: g.w, h: g.h, pixels: g.px.iter().flat_map(|&v| [v, v, v]).collect(), alpha: false, delay: std::time::Duration::ZERO }
    }

    fn whole(g: &Grey) -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(g.w as f32, g.h as f32))
    }

    #[test]
    fn native_detail_keeps_its_size() {
        let g = detailed(600, 400);
        assert!(fraction(&frame(&g), whole(&g)) >= 0.9);
    }

    #[test]
    fn enlargement_is_found_and_the_result_is_a_fixed_point() {
        let native = detailed(300, 200);
        let big = resize(&mut Resizer::new(), &native, 600, 400);
        let f = fraction(&frame(&big), whole(&big));
        assert!((0.4..=0.6).contains(&f), "{f}");
        let n = (600.0 * f).round() as u32;
        let shrunk = resize(&mut Resizer::new(), &big, n, (400.0 * f).round() as u32);
        let again = fraction(&frame(&shrunk), whole(&shrunk));
        assert!(again >= 0.95, "{again}");
    }

    #[test]
    fn flat_tiles_are_never_picked() {
        let detail = detailed(1024, 1024);
        let (w, h) = (2048, 1024);
        let px = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| if x < 1024 { 128 } else { detail.px[(y * 1024 + x - 1024) as usize] }).collect();
        assert_eq!(pick_tile(&Grey { w, h, px }, TILE, TILE), (1024, 0));
    }

    #[test]
    fn crop_and_alpha_are_respected() {
        let g = detailed(200, 100);
        let rgba = Frame { w: 200, h: 100, pixels: g.px.iter().flat_map(|&v| [v, v, v, 0]).collect(), alpha: true, delay: std::time::Duration::ZERO };
        let l = luma(&rgba, Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(60.0, 50.0)));
        assert_eq!((l.w, l.h, l.px[0]), (50, 30, g.px[20 * 200 + 10]));
    }

    /// `LIGHTVIEW_RESIZE_OPTIMALLY=<file> cargo test --release resize_optimally_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn resize_optimally_file() {
        let path = std::env::var("LIGHTVIEW_RESIZE_OPTIMALLY").expect("set LIGHTVIEW_RESIZE_OPTIMALLY");
        let img = image::open(&path).unwrap().into_rgb8();
        let (w, h) = img.dimensions();
        let f = Frame { w, h, pixels: img.into_raw(), alpha: false, delay: std::time::Duration::ZERO };
        let t = std::time::Instant::now();
        let fr = fraction(&f, Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32)));
        println!("{path}: {w}×{h} -> intrinsic width {} ({:.1}%) in {} ms", (fr * w as f32).round(), fr * 100.0, t.elapsed().as_millis());
    }
}
