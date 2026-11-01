//! SVG files: parsed once, then rendered to pixels at any whole-number scale.
use crate::i18n::tr;
use crate::load::Frame;
use resvg::tiny_skia::{Pixmap, PremultipliedColorU8, Transform};
use resvg::usvg::{self, fontdb};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// The most pixels one render holds (~256 MB). Files can declare any size, and a render covers
/// the whole image, so past this a deep zoom goes soft.
const MAX_PIXELS: u64 = 64_000_000;

/// SVG text needs the system's fonts; finding them is slow, so only once and only for SVGs.
fn fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        // fontdb's generic families are Windows fonts, and usvg drops text it has no font for.
        let pick = |names: &[&str]| names.iter().find(|n| db.faces().any(|f| f.families.iter().any(|(family, _)| family == *n))).map(|n| n.to_string())
            .or_else(|| db.faces().next().and_then(|f| f.families.first()).map(|(family, _)| family.clone()));
        let sans = pick(&["Arial", "Noto Sans", "DejaVu Sans", "Liberation Sans"]);
        let serif = pick(&["Times New Roman", "Noto Serif", "DejaVu Serif", "Liberation Serif"]);
        let mono = pick(&["Courier New", "Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono"]);
        if let (Some(sans), Some(serif), Some(mono)) = (sans, serif, mono) {
            db.set_sans_serif_family(sans);
            db.set_serif_family(serif);
            db.set_monospace_family(mono);
        }
        Arc::new(db)
    }).clone()
}

/// Parses a file, which is untrusted. Images must be embedded ones: by default usvg opens any path
/// an `href` names, even with `resources_dir` unset. That hangs on a device or a FIFO, and on Windows
/// a `\\server\share` path makes the system log in to that server, handing it a password hash.
pub fn load(bytes: &[u8]) -> Result<Arc<usvg::Tree>, String> {
    // usvg would inflate it with no limit, and `.svgz` isn't one of the extensions opened.
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Err(tr("compressed SVG files aren't supported").into());
    }
    // A panic would kill the decode worker and leave this path marked busy for good.
    catch_unwind(|| {
        let image_href_resolver = usvg::ImageHrefResolver { resolve_string: Box::new(|_, _| None), ..Default::default() };
        let options = usvg::Options { fontdb: fonts(), image_href_resolver, ..Default::default() };
        usvg::Tree::from_data(bytes, &options).map(Arc::new).map_err(|e| e.to_string())
    }).unwrap_or_else(|_| Err("the SVG parser crashed".into()))
}

/// The 1× size in pixels: what the file declares, shrunk to fit `MAX_PIXELS`.
pub fn size(tree: &usvg::Tree) -> [u32; 2] {
    let (w, h) = (tree.size().width(), tree.size().height());
    let shrink = (MAX_PIXELS as f32 / (w * h)).sqrt().min(1.0);
    let (w, h) = (((w * shrink).ceil() as u64).max(1), ((h * shrink).ceil() as u64).max(1));
    // Rounding up can still overshoot on an extreme aspect ratio.
    let w = w.min(MAX_PIXELS / h).max(1);
    [w as u32, h.min(MAX_PIXELS / w) as u32]
}

/// How many times bigger than 1× to render for `zoom` (screen px per 1× px): the next power of 4
/// at or above it, never below 1×, or the most that `size` allows.
pub fn tier(zoom: f32, [w, h]: [u32; 2]) -> u32 {
    let cap = (MAX_PIXELS as f64 / (w as f64 * h as f64)).sqrt() as u32;
    let mut k = 1;
    while (k as f32) < zoom && k < cap {
        k *= 4;
    }
    k.min(cap).max(1)
}

/// Renders at `k` times the 1× size, always exactly: edits map onto these pixels by `k`.
pub fn render(tree: &usvg::Tree, k: u32) -> Option<Frame> {
    let [w, h] = size(tree).map(|side| side * k);
    catch_unwind(AssertUnwindSafe(|| {
        let mut pixmap = Pixmap::new(w, h)?;
        let scale = Transform::from_scale(w as f32 / tree.size().width(), h as f32 / tree.size().height());
        resvg::render(tree, scale, &mut pixmap.as_mut());
        let mut pixels = pixmap.take();
        for p in pixels.as_chunks_mut::<4>().0 {
            let c = PremultipliedColorU8::from_rgba(p[0], p[1], p[2], p[3])?.demultiply();
            p.copy_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        Some(Frame { w, h, pixels, alpha: true, delay: Duration::ZERO })
    })).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="2" height="2" fill="#f00"/></svg>"##;

    #[test]
    fn renders_exact_sizes_with_transparency() {
        let tree = load(RED).unwrap();
        assert_eq!(size(&tree), [4, 2]);
        let one = render(&tree, 1).unwrap();
        assert_eq!((one.w, one.h, one.alpha), (4, 2, true));
        assert_eq!(&one.pixels[..4], [255, 0, 0, 255]);
        assert_eq!(one.pixels[one.pixels.len() - 1], 0, "the right half is transparent");
        let four = render(&tree, 4).unwrap();
        assert_eq!((four.w, four.h, four.pixels.len()), (16, 8, 16 * 8 * 4));
    }

    #[test]
    fn text_is_drawn() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="50"><text x="5" y="40" font-size="40" font-family="{family}">Hi</text></svg>"#;
        for family in ["sans-serif", "serif", "monospace", "A Font Nobody Has"] {
            let svg = String::from_utf8_lossy(svg).replace("{family}", family);
            let frame = render(&load(svg.as_bytes()).unwrap(), 1).unwrap();
            assert!(frame.pixels.chunks(4).any(|p| p[3] > 0), "no text with {family}");
        }
    }

    #[test]
    fn tiers_step_by_four_and_stop_at_the_cap() {
        for (zoom, k) in [(0.1, 1), (1.0, 1), (1.01, 4), (4.0, 4), (4.1, 16), (16.0, 16), (30.0, 64), (f32::INFINITY, 333)] {
            assert_eq!(tier(zoom, [24, 24]), k, "zoom {zoom}");
        }
        assert_eq!(tier(32.0, [1000, 1000]), 8, "falls back to the largest factor that fits");
        assert_eq!(tier(32.0, [8000, 8000]), 1);
        assert_eq!(tier(32.0, [64_000_000, 1]), 1);
    }

    #[test]
    fn huge_declared_sizes_are_capped() {
        for (w, h) in [(100_000, 100_000), (1_000_000_000, 1), (1, 1_000_000_000)] {
            let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"><rect width="{w}" height="{h}"/></svg>"#);
            let [w, h] = size(&load(svg.as_bytes()).unwrap());
            assert!(w as u64 * h as u64 <= MAX_PIXELS && w > 0 && h > 0, "{w}×{h}");
        }
    }

    fn with_image(href: &str) -> String {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4"><image xlink:href="{href}" width="4" height="4"/></svg>"#)
    }

    fn draws_something(svg: &str) -> bool {
        render(&load(svg.as_bytes()).unwrap(), 1).unwrap().pixels.chunks(4).any(|p| p[3] > 0)
    }

    #[test]
    fn only_embedded_images_are_loaded() {
        let path = std::env::temp_dir().join(format!("lightview-svg-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255])).save(&path).unwrap();
        let local = draws_something(&with_image(&path.display().to_string()));
        std::fs::remove_file(&path).unwrap();
        assert!(!local, "an SVG read a local file");
        const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        assert!(draws_something(&with_image(PIXEL)));
    }

    // Reading this file never ends, so the href must not even be tried.
    #[cfg(unix)]
    #[test]
    fn device_files_are_not_read() {
        assert!(!draws_something(&with_image("/dev/zero")));
    }

    #[test]
    fn gzip_is_refused() {
        // `<svg xmlns="http://www.w3.org/2000/svg" width="3" height="2"/>`, gzipped.
        const SVGZ: &[u8] = &[
            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0xb3, 0x29, 0x2e, 0x4b, 0x57, 0xa8, 0xc8, 0xcd, 0xc9, 0x2b, 0xb6, 0x55, 0xca, 0x28, 0x29, 0x29, 0xb0, 0xd2, 0xd7, 0x2f, 0x2f, 0x2f,
            0xd7, 0x2b, 0x37, 0xd6, 0xcb, 0x2f, 0x4a, 0xd7, 0x37, 0x32, 0x30, 0x30, 0xd0, 0x07, 0xaa, 0x50, 0x52, 0x28, 0xcf, 0x4c, 0x29, 0xc9, 0xb0, 0x55, 0x32, 0x56, 0x52, 0xc8, 0x48, 0xcd, 0x4c, 0xcf,
            0x28, 0xb1, 0x55, 0x32, 0x52, 0xd2, 0xb7, 0x03, 0x00, 0x25, 0x08, 0xdc, 0x66, 0x3e, 0x00, 0x00, 0x00,
        ];
        assert!(load(SVGZ).is_err());
    }

    /// Just under usvg's limit of 1024 levels, so it's all parsed and drawn; a stack overflow
    /// can't be caught and would close the app.
    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        const HEAD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4">"#;
        const RECT: &str = r##"<rect id="u0" width="4" height="4" fill="#f00"/>"##;
        let shapes: [fn(usize) -> String; 3] = [
            |depth| format!("{HEAD}{}{RECT}{}</svg>", "<g>".repeat(depth), "</g>".repeat(depth)),
            |depth| format!("{HEAD}{}{RECT}{}</svg>", r#"<g opacity="0.9">"#.repeat(depth), "</g>".repeat(depth)),
            |depth| {
                let links: String = (1..depth).map(|i| format!(r##"<use id="u{i}" xlink:href="#u{}"/>"##, i - 1)).collect();
                format!("{HEAD}<defs>{RECT}{links}</defs><use xlink:href=\"#u{}\"/></svg>", depth - 1)
            },
        ];
        for shape in shapes {
            // The deepest that usvg accepts, on a thread with the decode pool's stack size.
            let drawn = [1000, 500, 340, 250, 200, 100].into_iter().find(|&depth| {
                let svg = shape(depth);
                let worker = std::thread::spawn(move || load(svg.as_bytes()).ok().and_then(|tree| render(&tree, 1)).is_some());
                worker.join().unwrap()
            });
            eprintln!("drawn at depth {drawn:?}");
            assert!(drawn.is_some());
        }
    }

    #[test]
    fn bad_files_are_errors() {
        assert!(load(b"not an svg").is_err());
        assert!(load(b"").is_err());
        assert!(load(br#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"#).is_err());
    }
}
