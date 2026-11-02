//! Colour management: images with an ICC profile are converted to sRGB as they're decoded, so
//! everything after that (screen, edits, Copy, AI) works in one space. Assumes an sRGB monitor.
use crate::load::Frame;
use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};

/// The profile to convert from, or `None` when there's nothing to do: `icc` is unreadable, isn't
/// RGB, or is sRGB under whatever name. The bytes are untrusted. Most tagged photos stop here, so
/// the check is a few colours through the transform, not a comparison of curves.
pub fn to_srgb(icc: &[u8]) -> Option<ColorProfile> {
    let profile = ColorProfile::new_from_slice(icc).ok()?;
    if profile.color_space != DataColorSpace::Rgb {
        return None;
    }
    let transform = profile.create_transform_8bit(Layout::Rgb, &ColorProfile::new_srgb(), Layout::Rgb, TransformOptions::default()).ok()?;
    const LEVELS: [u8; 5] = [0, 64, 128, 192, 255];
    let colors: Vec<u8> = LEVELS.iter().flat_map(|&r| LEVELS.iter().flat_map(move |&g| LEVELS.iter().flat_map(move |&b| [r, g, b]))).collect();
    let mut out = vec![0; colors.len()];
    transform.transform(&colors, &mut out).ok()?;
    colors.iter().zip(&out).any(|(a, b)| a.abs_diff(*b) > 1).then_some(profile)
}

/// Converts `frame`'s pixels from `profile` to sRGB. On a failure they stay as they are.
pub fn convert(profile: &ColorProfile, frame: &mut Frame) {
    let layout = if frame.alpha { Layout::Rgba } else { Layout::Rgb };
    let Ok(transform) = profile.create_transform_8bit(layout, &ColorProfile::new_srgb(), layout, TransformOptions::default()) else { return };
    let mut out = vec![0; frame.pixels.len()];
    // Split by rows over a few cores: one core takes ~65 ms for 12 MP.
    let row = frame.w as usize * if frame.alpha { 4 } else { 3 };
    let parts = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
    let chunk = frame.h.div_ceil(parts as u32).max(1) as usize * row;
    let ok = std::thread::scope(|s| {
        let jobs: Vec<_> = frame.pixels.chunks(chunk).zip(out.chunks_mut(chunk)).map(|(src, dst)| s.spawn(|| transform.transform(src, dst).is_ok())).collect();
        jobs.into_iter().all(|j| j.join().unwrap_or(false))
    });
    if ok {
        frame.pixels = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    const COLORS: [[u8; 3]; 6] = [[200, 100, 50], [30, 180, 90], [10, 20, 240], [128, 128, 128], [255, 0, 0], [0, 255, 0]];

    fn p3() -> Vec<u8> {
        ColorProfile::new_display_p3().encode().unwrap()
    }

    fn converted(icc: &[u8]) -> Vec<[u8; 3]> {
        let mut frame = Frame { w: 6, h: 1, pixels: COLORS.concat(), alpha: false, delay: Duration::ZERO };
        convert(&to_srgb(icc).unwrap(), &mut frame);
        frame.pixels.chunks(3).map(|c| [c[0], c[1], c[2]]).collect()
    }

    /// The references are lcms 2.19's (Pillow's `ImageCms`), not moxcms's own idea of itself.
    fn matches(icc: &[u8], reference: [[u8; 3]; 6]) {
        for (got, want) in converted(icc).into_iter().zip(reference) {
            assert!(got.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2), "{got:?} vs {want:?}");
        }
    }

    #[test]
    fn display_p3_matches_lcms() {
        matches(&p3(), [[215, 93, 31], [0, 183, 78], [7, 20, 250], [128, 128, 128], [255, 0, 0], [0, 255, 0]]);
    }

    #[test]
    fn adobe_rgb_matches_lcms() {
        let icc = ColorProfile::new_adobe_rgb().encode().unwrap();
        matches(&icc, [[227, 100, 42], [0, 181, 83], [0, 12, 245], [129, 129, 129], [255, 0, 0], [0, 255, 0]]);
    }

    #[test]
    fn srgb_needs_nothing() {
        assert!(to_srgb(&ColorProfile::new_srgb().encode().unwrap()).is_none());
    }

    #[test]
    fn profiles_it_cant_use_are_skipped() {
        let good = p3();
        for len in 0..good.len() {
            let _ = to_srgb(&good[..len]);
        }
        assert!(to_srgb(b"").is_none());
        assert!(to_srgb(&[0xFF; 4096]).is_none());
        assert!(to_srgb(&ColorProfile::new_gray_with_gamma(2.2).encode().unwrap()).is_none(), "not RGB");
    }

    #[test]
    fn every_flipped_byte_is_survivable() {
        let good = p3();
        for i in 0..good.len() {
            for bits in [0xFF, 0x80, 0x01] {
                let mut bad = good.clone();
                bad[i] ^= bits;
                if let Some(profile) = to_srgb(&bad) {
                    let mut frame = Frame { w: 1, h: 1, pixels: vec![1, 2, 3], alpha: false, delay: Duration::ZERO };
                    convert(&profile, &mut frame);
                }
            }
        }
    }

    #[test]
    fn a_big_frame_converts_the_same_in_every_part() {
        let (w, h) = (37, 101);
        let pixels: Vec<u8> = (0..w * h).flat_map(|_| [200, 100, 50]).collect();
        let mut frame = Frame { w, h, pixels, alpha: false, delay: Duration::ZERO };
        convert(&to_srgb(&p3()).unwrap(), &mut frame);
        assert!(frame.pixels.chunks(3).all(|p| p == &frame.pixels[..3]) && frame.pixels[0].abs_diff(215) <= 2);
    }

    #[test]
    fn alpha_stays() {
        let mut frame = Frame { w: 2, h: 1, pixels: vec![200, 100, 50, 7, 30, 180, 90, 200], alpha: true, delay: Duration::ZERO };
        convert(&to_srgb(&p3()).unwrap(), &mut frame);
        assert_eq!((frame.pixels[3], frame.pixels[7]), (7, 200));
        assert!(frame.pixels[0].abs_diff(215) <= 2);
    }
}
