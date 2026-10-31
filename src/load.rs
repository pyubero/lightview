use crate::edit::Doc;
use crate::settings::SortOrder;
use crate::{svg, turbo};
use eframe::egui;
use image::codecs::{gif::GifDecoder, webp::WebPDecoder};
use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::Duration;

pub const EXTENSIONS: [&str; 9] = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "svg"];

#[derive(Clone)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    // RGB unless the image has alpha: resizing RGBA is ~7× slower.
    pub pixels: Vec<u8>,
    pub alpha: bool,
    pub delay: Duration,
}

/// A decoded file. A still image is one frame.
pub struct Loaded {
    pub frames: Vec<Frame>,
    pub file_size: u64,
    /// For an SVG, which can be drawn again sharper when zoomed in.
    pub svg: Option<Arc<resvg::usvg::Tree>>,
}

pub enum Msg {
    /// A file picked in the Open dialog.
    Open(PathBuf),
    Folder(Vec<PathBuf>),
    Preview(PathBuf, Frame),
    Full(PathBuf, Loaded),
    /// An SVG drawn this many times bigger than its 1× frame.
    Sharp(PathBuf, u32, Frame),
    Error(PathBuf, String),
    /// A save finished (successfully or not) writing this path. Save As also says which image
    /// and edits it wrote; an EXIF copy wrote no edits.
    Saved(PathBuf, Result<(), String>, Option<(PathBuf, Doc)>),
    /// Background removal finished for this image: the cut-out, and the subject's bounds
    /// when cropping to it.
    Cutout(PathBuf, Result<(image::RgbaImage, Option<egui::Rect>), String>),
    Status(String),
    /// Denoise / Enhance run `id`: how far it is (0–1).
    Progress(u64, f32),
    /// Denoise / Enhance run `id` finished `region` (output px) of this image: the result and how
    /// long it took per megapixel. `whole` is false for the preview.
    Restored(u64, PathBuf, egui::Rect, bool, Result<(image::RgbaImage, f64), String>),
    /// A model download finished.
    Downloaded(String, Result<(), String>),
    /// A model's input size was read.
    Detected(String, Result<u32, String>),
}

/// Sends messages to the UI and wakes it up. The context is set once eframe has created it;
/// until then the first frame picks the messages up anyway.
#[derive(Clone)]
pub struct Notify {
    pub tx: mpsc::Sender<Msg>,
    pub ctx: Arc<OnceLock<egui::Context>>,
}

impl Notify {
    pub fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        if let Some(ctx) = self.ctx.get() {
            ctx.request_repaint();
        }
    }
}

/// What the workers should decode next, and what they're decoding now.
#[derive(Default)]
struct Queue {
    wanted: Vec<PathBuf>,
    busy: HashSet<PathBuf>,
}

/// A fixed pool of decode threads working through the newest wanted list.
pub struct Loader {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    // The viewport in physical pixels, which sets the reduced-size JPEG decode.
    target: Arc<Mutex<[u32; 2]>>,
}

impl Loader {
    pub fn new(notify: Notify, workers: usize, target: [u32; 2]) -> Self {
        let queue: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        let target = Arc::new(Mutex::new(target));
        for _ in 0..workers {
            let (queue, notify, target) = (queue.clone(), notify.clone(), target.clone());
            std::thread::spawn(move || loop {
                let path = {
                    let mut q = queue.1.wait_while(queue.0.lock().unwrap(), |q| q.wanted.is_empty()).unwrap();
                    let path = q.wanted.remove(0);
                    q.busy.insert(path.clone());
                    path
                };
                let target = *target.lock().unwrap();
                let msg = match load(&path, target, |frame| notify.send(Msg::Preview(path.clone(), frame))) {
                    Ok(loaded) => Msg::Full(path.clone(), loaded),
                    Err(e) => Msg::Error(path.clone(), e),
                };
                queue.0.lock().unwrap().busy.remove(&path);
                notify.send(msg);
            });
        }
        Loader { queue, target }
    }

    pub fn set_target(&self, size: [u32; 2]) {
        *self.target.lock().unwrap() = size;
    }

    /// Replaces the wanted list (most important first). Stale requests are dropped;
    /// paths already being decoded aren't queued twice.
    pub fn want(&self, paths: Vec<PathBuf>) {
        let mut q = self.queue.0.lock().unwrap();
        q.wanted = paths.into_iter().filter(|p| !q.busy.contains(p)).collect();
        self.queue.1.notify_all();
    }
}

/// Starts decoding `path` and listing its folder.
pub fn start(path: &Path, sort: SortOrder, loader: &Loader, notify: &Notify) {
    loader.want(vec![path.to_path_buf()]);
    relist(path, sort, notify);
}

/// Lists `path`'s folder on another thread and sends it as `Msg::Folder`.
pub fn relist(path: &Path, sort: SortOrder, notify: &Notify) {
    let (path, notify) = (path.to_path_buf(), notify.clone());
    std::thread::spawn(move || notify.send(Msg::Folder(list_folder(&path, sort))));
}

/// Decodes a file. For JPEGs, `on_preview` first gets the EXIF thumbnail (if any), then,
/// when libjpeg-turbo is installed, a reduced-size decode just big enough for `target`.
fn load(path: &Path, target: [u32; 2], on_preview: impl Fn(Frame) + Sync) -> Result<Loaded, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let file_size = bytes.len() as u64;
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        let tree = svg::load(&bytes)?;
        let frame = svg::render(&tree, 1).ok_or("the SVG renderer failed")?;
        return Ok(Loaded { frames: vec![frame], file_size, svg: Some(tree) });
    }
    let reader = ImageReader::new(Cursor::new(&bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let format = reader.format();

    let animated = match format {
        Some(ImageFormat::Gif) => Some(GifDecoder::new(Cursor::new(&bytes)).and_then(|d| d.into_frames().collect_frames())),
        Some(ImageFormat::WebP) => {
            let d = WebPDecoder::new(Cursor::new(&bytes)).map_err(|e| e.to_string())?;
            d.has_animation().then(|| d.into_frames().collect_frames())
        }
        _ => None,
    };
    if let Some(frames) = animated {
        // shortcut: all frames are decoded before showing; stream them if huge GIFs are slow.
        let frames = frames.map_err(|e| e.to_string())?;
        let frames: Vec<Frame> = frames.into_iter().map(|f| {
            let (n, d) = f.delay().numer_denom_ms();
            // Like browsers: delays under 20 ms mean "unspecified" and play at 100 ms.
            let ms = if d == 0 || n / d < 20 { 100 } else { n / d };
            to_frame(DynamicImage::ImageRgba8(f.into_buffer()), Duration::from_millis(ms.into()))
        }).collect();
        if frames.is_empty() {
            return Err(crate::i18n::tr("no frames").into());
        }
        return Ok(Loaded { frames, file_size, svg: None });
    }

    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let exif = decoder.exif_metadata().ok().flatten();
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    if format == Some(ImageFormat::Jpeg)
        && let Some(thumb) = exif.as_deref().and_then(exif_thumbnail)
        && let Ok(mut img) = image::load_from_memory(thumb)
    {
        img.apply_orientation(orientation);
        on_preview(to_frame(img, Duration::ZERO));
    }
    let turbo = (format == Some(ImageFormat::Jpeg) && turbo::available()).then(|| {
        let (w, h) = decoder.dimensions();
        let rotated = matches!(orientation, Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH);
        let den = scale_denominator(if rotated { (h, w) } else { (w, h) }, target);
        // The scope joins the reduced-size decode, so its preview always arrives before the full image.
        let (bytes, on_preview) = (&bytes, &on_preview);
        std::thread::scope(|s| {
            if let Some(den) = den {
                s.spawn(move || {
                    if let Some(small) = turbo::decode(bytes, den) {
                        let mut small = DynamicImage::ImageRgb8(small);
                        small.apply_orientation(orientation);
                        on_preview(to_frame(small, Duration::ZERO));
                    }
                });
            }
            turbo::decode(bytes, 1)
        })
    });
    let mut img = match turbo.flatten() {
        Some(img) => DynamicImage::ImageRgb8(img),
        None => DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?,
    };
    img.apply_orientation(orientation);
    Ok(Loaded { frames: vec![to_frame(img, Duration::ZERO)], file_size, svg: None })
}

/// The largest JPEG scale-down (8, 4 or 2) whose output still covers `img` (upright) when
/// fitted into `target`, or None when only 1:1 is big enough.
fn scale_denominator(img: (u32, u32), target: [u32; 2]) -> Option<u32> {
    let fit = (target[0] as f64 / img.0 as f64).min(target[1] as f64 / img.1 as f64);
    [8, 4, 2].into_iter().find(|&d| img.0.div_ceil(d) as f64 >= img.0 as f64 * fit && img.1.div_ceil(d) as f64 >= img.1 as f64 * fit)
}

fn to_frame(img: DynamicImage, delay: Duration) -> Frame {
    let (w, h, alpha) = (img.width(), img.height(), img.color().has_alpha());
    let pixels = if alpha { img.into_rgba8().into_raw() } else { img.into_rgb8().into_raw() };
    Frame { w, h, pixels, alpha, delay }
}

/// The JPEG thumbnail in IFD1 of an EXIF (TIFF) block, if any. The file is untrusted:
/// every read is bounds-checked and anything malformed returns `None`.
fn exif_thumbnail(tiff: &[u8]) -> Option<&[u8]> {
    let (u16_at, u32_at) = tiff_readers(tiff)?;
    let ifd1 = u32_at(next_ifd_ptr(tiff)?)?;
    if ifd1 == 0 {
        return None;
    }
    let (mut offset, mut len) = (None, None);
    for i in 0..u16_at(ifd1)? {
        let entry = ifd1 + 2 + i * 12;
        match u16_at(entry)? {
            0x0201 => offset = u32_at(entry + 8),
            0x0202 => len = u32_at(entry + 8),
            _ => {}
        }
    }
    let offset = offset?;
    tiff.get(offset..offset.checked_add(len?)?)
}

type Reader<'a> = Box<dyn Fn(usize) -> Option<usize> + 'a>;

/// Bounds-checked u16 and u32 readers in the TIFF block's byte order.
fn tiff_readers(tiff: &[u8]) -> Option<(Reader<'_>, Reader<'_>)> {
    let le = match tiff.get(0..4)? {
        [b'I', b'I', 42, 0] => true,
        [b'M', b'M', 0, 42] => false,
        _ => return None,
    };
    let u16_at = move |o: usize| {
        let b = tiff.get(o..o.checked_add(2)?)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) } as usize)
    };
    let u32_at = move |o: usize| {
        let b: [u8; 4] = tiff.get(o..o.checked_add(4)?)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) } as usize)
    };
    Some((Box::new(u16_at), Box::new(u32_at)))
}

/// Offset of IFD0's "next IFD" pointer, which points at IFD1 (the thumbnail), if it's in bounds.
fn next_ifd_ptr(tiff: &[u8]) -> Option<usize> {
    let (u16_at, u32_at) = tiff_readers(tiff)?;
    let ifd0 = u32_at(4)?;
    let ptr = ifd0 + 2 + u16_at(ifd0)? * 12;
    u32_at(ptr).map(|_| ptr)
}

/// Unlinks the EXIF thumbnail, for when the pixels no longer match it (e.g. after rotating).
pub fn drop_exif_thumbnail(tiff: &mut [u8]) {
    if let Some(ptr) = next_ifd_ptr(tiff) {
        tiff[ptr..ptr + 4].fill(0);
    }
}

/// Sets IFD0's XResolution and YResolution to `dpi` and ResolutionUnit to inches, so the
/// copied EXIF agrees with the DPI written into the saved file. Tags that aren't there are
/// left out (the file's own header still has the DPI); malformed input is left alone.
pub fn set_exif_dpi(tiff: &mut [u8], dpi: u16) {
    let mut writes: Vec<(usize, [u32; 2], bool)> = Vec::new();
    {
        let Some((u16_at, u32_at)) = tiff_readers(tiff) else { return };
        let Some(ifd0) = u32_at(4) else { return };
        for i in 0..u16_at(ifd0).unwrap_or(0) {
            let entry = ifd0 + 2 + i * 12;
            match (u16_at(entry), u16_at(entry + 2)) {
                // RATIONAL: the value is 8 bytes elsewhere.
                (Some(0x011A | 0x011B), Some(5)) => {
                    if let Some(at) = u32_at(entry + 8).filter(|&at| u32_at(at + 4).is_some()) {
                        writes.push((at, [dpi as u32, 1], true));
                    }
                }
                // SHORT: the value sits in the entry.
                (Some(0x0128), Some(3)) if u16_at(entry + 8).is_some() => writes.push((entry + 8, [2, 0], false)),
                (Some(_), Some(_)) => {}
                _ => return,
            }
        }
    }
    let le = tiff[0] == b'I';
    for (at, [a, b], rational) in writes {
        if rational {
            tiff[at..at + 4].copy_from_slice(&if le { a.to_le_bytes() } else { a.to_be_bytes() });
            tiff[at + 4..at + 8].copy_from_slice(&if le { b.to_le_bytes() } else { b.to_be_bytes() });
        } else {
            tiff[at..at + 2].copy_from_slice(&if le { (a as u16).to_le_bytes() } else { (a as u16).to_be_bytes() });
        }
    }
}

/// Whether a Trash item is `path`. When Windows hides known extensions, `item.name` has none,
/// but `item.id` (like `$RAB12CD.jpg`) keeps it.
fn is_trashed_copy(item: &trash::TrashItem, path: &Path) -> bool {
    let same_name = Some(item.name.as_os_str()) == path.file_name() || (Some(item.name.as_os_str()) == path.file_stem() && Path::new(&item.id).extension() == path.extension());
    // Compared canonically: on Windows `path` starts with `\\?\`, the Trash's paths don't.
    same_name && item.original_parent.canonicalize().is_ok_and(|p| Some(p.as_path()) == path.parent())
}

/// Puts a file back from the Trash, to the place it was deleted from.
pub fn restore_trashed(path: &Path) -> Result<(), String> {
    let items = trash::os_limited::list().map_err(|e| e.to_string())?;
    let newest = items.into_iter().filter(|i| is_trashed_copy(i, path)).max_by_key(|i| i.time_deleted);
    let item = newest.ok_or_else(|| crate::i18n::tr("It's no longer in the Trash").to_string())?;
    // The full name, or the file would come back without its extension.
    let name = path.file_name().unwrap_or_default().to_owned();
    trash::os_limited::restore_all([trash::TrashItem { name, ..item }]).map_err(|e| e.to_string())
}

/// The supported images in `path`'s folder: by name in file-manager order, or oldest first.
fn list_folder(path: &Path, sort: SortOrder) -> Vec<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![path.to_path_buf()] };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())))
        .collect();
    let name = |p: &PathBuf| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let by_name = |a: &PathBuf, b: &PathBuf| {
        let (a, b) = (name(a), name(b));
        natural_cmp(&a, &b).then_with(|| a.cmp(&b))
    };
    match sort {
        SortOrder::Name => files.sort_by(by_name),
        SortOrder::DateModified => {
            let modified = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
            let mut dated: Vec<_> = files.into_iter().map(|p| (modified(&p), p)).collect();
            dated.sort_by(|(ta, a), (tb, b)| ta.cmp(tb).then_with(|| by_name(a, b)));
            files = dated.into_iter().map(|(_, p)| p).collect();
        }
    }
    files
}

/// Case-insensitive comparison where runs of digits compare as numbers ("img2" < "img10").
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let digits = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.next_if(|c| c.is_ascii_digit()) {
                        s.push(c);
                    }
                    s.trim_start_matches('0').to_string()
                };
                let (x, y) = (digits(&mut a), digits(&mut b));
                let ord = x.len().cmp(&y.len()).then_with(|| x.cmp(&y));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_trashed_file_with_hidden_extension() {
        let dir = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let path = dir.join("photo.jpg");
        let item = |name: &str, id: &str| trash::TrashItem { id: id.into(), name: name.into(), original_parent: dir.clone(), time_deleted: 0 };
        assert!(is_trashed_copy(&item("photo.jpg", "/trash/info/photo.jpg.trashinfo"), &path));
        assert!(is_trashed_copy(&item("photo", r"C:\$Recycle.Bin\S-1\$RAB12CD.jpg"), &path));
        assert!(!is_trashed_copy(&item("photo", r"C:\$Recycle.Bin\S-1\$RAB12CD.png"), &path));
        assert!(!is_trashed_copy(&item("other.jpg", "/trash/info/other.jpg.trashinfo"), &path));
        assert!(!is_trashed_copy(&trash::TrashItem { original_parent: dir.join("elsewhere"), ..item("photo.jpg", "x") }, &path));
    }

    // Uses the real Trash: `cargo test trash_round_trip -- --ignored`.
    #[test]
    #[ignore]
    fn trash_round_trip() {
        let path = std::env::temp_dir().join(format!("lightview-trash-{}.txt", std::process::id()));
        std::fs::write(&path, "image").unwrap();
        let path = std::fs::canonicalize(&path).unwrap();
        trash::delete(&path).unwrap();
        assert!(!path.exists());
        restore_trashed(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "image");
        std::fs::remove_file(&path).unwrap();
        assert!(restore_trashed(&path).is_err());
    }

    #[test]
    fn opens_svg() {
        let path = std::env::temp_dir().join(format!("lightview-{}.SVG", std::process::id()));
        std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="3" height="2"/>"#).unwrap();
        let loaded = load(&path, [100, 100], |_| {}).unwrap();
        std::fs::write(&path, "broken").unwrap();
        let broken = load(&path, [100, 100], |_| {});
        std::fs::remove_file(&path).unwrap();
        assert_eq!((loaded.frames[0].w, loaded.frames[0].h), (3, 2));
        assert!(loaded.svg.is_some() && broken.is_err());
    }

    #[test]
    fn opens_tiff() {
        let path = std::env::temp_dir().join(format!("lightview-{}.tif", std::process::id()));
        let img = image::RgbImage::from_fn(3, 2, |x, y| image::Rgb([x as u8 * 40, y as u8 * 40, 200]));
        img.save(&path).unwrap();
        let loaded = load(&path, [100, 100], |_| {});
        std::fs::remove_file(&path).unwrap();
        let f = &loaded.unwrap().frames[0];
        assert_eq!((f.w, f.h), (3, 2));
        assert_eq!(&f.pixels[f.pixels.len() - 3..], [80, 40, 200]);
    }

    #[test]
    fn natural_order() {
        assert_eq!(natural_cmp("img2.jpg", "img10.jpg"), Ordering::Less);
        assert_eq!(natural_cmp("IMG_b.jpg", "img_A.jpg"), Ordering::Greater);
        assert_eq!(natural_cmp("a007", "a7"), Ordering::Equal);
        assert_eq!(natural_cmp("photo", "photo1"), Ordering::Less);
    }

    #[test]
    fn scale_down_covers_the_fitted_size() {
        assert_eq!(scale_denominator((4000, 2252), [1280, 720]), Some(2));
        assert_eq!(scale_denominator((4000, 2252), [500, 282]), Some(8));
        assert_eq!(scale_denominator((4000, 2252), [1000, 563]), Some(4));
        assert_eq!(scale_denominator((1000, 563), [1280, 720]), None);
        assert_eq!(scale_denominator((4000, 2252), [3840, 2160]), None);
        // A portrait photo (already swapped to upright) in a landscape window is limited by height.
        assert_eq!(scale_denominator((2252, 4000), [1280, 720]), Some(4));
    }

    /// A TIFF block with an empty IFD0 and an IFD1 pointing at `thumb`.
    fn tiff(le: bool, thumb: &[u8]) -> Vec<u8> {
        let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut t = Vec::new();
        t.extend(if le { *b"II\x2a\0" } else { *b"MM\0\x2a" });
        t.extend(u32b(8)); // IFD0 at 8
        t.extend(u16b(0)); // IFD0: no entries
        t.extend(u32b(14)); // IFD1 at 14
        t.extend(u16b(2)); // IFD1: two entries
        let thumb_at = 14 + 2 + 2 * 12 + 4;
        for (tag, value) in [(0x0201, thumb_at), (0x0202, thumb.len() as u32)] {
            t.extend(u16b(tag));
            t.extend(u16b(4)); // LONG
            t.extend(u32b(1));
            t.extend(u32b(value));
        }
        t.extend(u32b(0)); // no IFD2
        t.extend(thumb);
        t
    }

    #[test]
    fn exif_thumbnail_found() {
        assert_eq!(exif_thumbnail(&tiff(true, b"JPEGDATA")), Some(&b"JPEGDATA"[..]));
        assert_eq!(exif_thumbnail(&tiff(false, b"JPEGDATA")), Some(&b"JPEGDATA"[..]));
    }

    #[test]
    fn drop_thumbnail() {
        for le in [true, false] {
            let mut t = tiff(le, b"JPEGDATA");
            drop_exif_thumbnail(&mut t);
            assert_eq!(exif_thumbnail(&t), None);
        }
        // Malformed input is left alone without panicking.
        let mut junk = b"II\x2a\0\xff\xff\xff\xff".to_vec();
        drop_exif_thumbnail(&mut junk);
        assert_eq!(junk, b"II\x2a\0\xff\xff\xff\xff");
    }

    #[test]
    fn exif_dpi_malformed() {
        // IFD0 with XResolution (RATIONAL at 26) and ResolutionUnit, then 72/1.
        let mut good = b"II\x2a\0\x08\0\0\0\x02\0".to_vec();
        good.extend([0x1A, 0x01, 5, 0, 1, 0, 0, 0, 34, 0, 0, 0]);
        good.extend([0x28, 0x01, 3, 0, 1, 0, 0, 0, 3, 0, 0, 0]);
        good.extend([0, 0, 0, 0, 72, 0, 0, 0, 1, 0, 0, 0]);
        let mut t = good.clone();
        set_exif_dpi(&mut t, 300);
        assert_eq!(&t[34..42], &[44, 1, 0, 0, 1, 0, 0, 0]);
        assert_eq!(&t[30..32], &[2, 0]);
        // Truncated anywhere: no panic.
        for len in 0..good.len() {
            set_exif_dpi(&mut good[..len].to_vec(), 300);
        }
    }

    #[test]
    fn exif_thumbnail_malformed() {
        let good = tiff(true, b"JPEGDATA");
        // Every truncation of a valid block fails cleanly.
        for len in 0..good.len() {
            assert_eq!(exif_thumbnail(&good[..len]), None);
        }
        assert_eq!(exif_thumbnail(b"garbage garbage garbage"), None);
        // Offsets pointing far out of bounds.
        let mut bad = good.clone();
        bad[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(exif_thumbnail(&bad), None);
    }

    #[test]
    fn animated_gif_frames() {
        let path = std::env::temp_dir().join(format!("lightview-test-{}.gif", std::process::id()));
        {
            let mut enc = image::codecs::gif::GifEncoder::new(std::fs::File::create(&path).unwrap());
            for (shade, ms) in [(0u8, 50u32), (255, 0)] {
                let buf = image::RgbaImage::from_pixel(4, 2, image::Rgba([shade, shade, shade, 255]));
                enc.encode_frame(image::Frame::from_parts(buf, 0, 0, image::Delay::from_numer_denom_ms(ms, 1))).unwrap();
            }
        }
        let loaded = load(&path, [1280, 720], |_| panic!("GIFs have no preview")).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(loaded.frames.len(), 2);
        assert_eq!((loaded.frames[0].w, loaded.frames[0].h, loaded.frames[0].alpha), (4, 2, true));
        // 50 ms kept; 0 ms ("unspecified") plays at 100 ms.
        assert_eq!(loaded.frames.iter().map(|f| f.delay.as_millis()).collect::<Vec<_>>(), [50, 100]);
        assert_eq!(loaded.frames[1].pixels[0], 255);
    }

    #[test]
    fn folder_filter_and_order() {
        let dir = std::env::temp_dir().join(format!("lightview-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["b10.JPG", "b2.png", "a.webp", "notes.txt", "clip.mp4", "photo.heic"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        // Oldest: b2.png, then a.webp, then b10.JPG.
        let base = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for (f, secs) in [("b2.png", 0), ("a.webp", 10), ("b10.JPG", 20)] {
            std::fs::File::options().write(true).open(dir.join(f)).unwrap().set_modified(base + Duration::from_secs(secs)).unwrap();
        }
        let names = |sort| -> Vec<String> {
            list_folder(&dir.join("a.webp"), sort).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
        };
        let (by_name, by_date) = (names(SortOrder::Name), names(SortOrder::DateModified));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(by_name, ["a.webp", "b2.png", "b10.JPG"]);
        assert_eq!(by_date, ["b2.png", "a.webp", "b10.JPG"]);
    }
}
