use crate::edit::Doc;
use crate::i18n::{tr, trf};
use crate::settings::SortOrder;
use crate::{color, heif, jpegxl, raw, svg, turbo};
use eframe::egui;
use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};
use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, Frames, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::{Duration, SystemTime};

pub const EXTENSIONS: [&str; 33] = [
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "svg", "ico", "qoi", "tga", "pnm", "pbm", "pgm", "ppm", "pam", "dds", "hdr", "exr", "jxl", "heic", "heif", "hif", "avif", "cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2",
];
/// The ones only native decoders read: see `heif`.
pub const NATIVE_EXTENSIONS: [&str; 4] = ["heic", "heif", "hif", "avif"];

/// Bigger files are refused before they're read.
const MAX_FILE_BYTES: u64 = 1 << 30;
/// The memory an animation's frames may hold: what a tiny file can ask for has no other limit.
pub const MAX_ANIMATION_BYTES: usize = 1 << 30;
/// The `image` crate's default allocation limit, which many of its decoders don't apply themselves.
pub const MAX_CANVAS_BYTES: u64 = 512 << 20;
/// A TIFF's pages, including the thumbnails that are skipped.
const MAX_TIFF_IFDS: usize = 4096;

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

impl Loaded {
    /// Whether the frames are pages to step through, not an animation to play.
    pub fn paged(&self) -> bool {
        self.frames.len() > 1 && self.frames[0].delay.is_zero()
    }
}

pub enum Msg {
    /// A file picked in the Open dialog.
    Open(PathBuf),
    Folder(Vec<PathBuf>),
    /// The watched file changed on disk and has settled.
    Changed(PathBuf, Stamp),
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
    color_managed: Arc<AtomicBool>,
}

impl Loader {
    pub fn new(notify: Notify, workers: usize, target: [u32; 2], color_managed: bool) -> Self {
        let queue: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        let target = Arc::new(Mutex::new(target));
        let color_managed = Arc::new(AtomicBool::new(color_managed));
        for _ in 0..workers {
            let (queue, notify, target, color_managed) = (queue.clone(), notify.clone(), target.clone(), color_managed.clone());
            std::thread::spawn(move || loop {
                let path = {
                    let mut q = queue.1.wait_while(queue.0.lock().unwrap(), |q| q.wanted.is_empty()).unwrap();
                    let path = q.wanted.remove(0);
                    q.busy.insert(path.clone());
                    path
                };
                let target = *target.lock().unwrap();
                let managed = color_managed.load(AtomicOrdering::Relaxed);
                let msg = match catching(|| load(&path, target, managed, |frame| notify.send(Msg::Preview(path.clone(), frame)))) {
                    Ok(loaded) => Msg::Full(path.clone(), loaded),
                    Err(e) => Msg::Error(path.clone(), e),
                };
                queue.0.lock().unwrap().busy.remove(&path);
                notify.send(msg);
            });
        }
        Loader { queue, target, color_managed }
    }

    /// Whether images decoded from now on are converted to sRGB.
    pub fn set_color_managed(&self, on: bool) {
        self.color_managed.store(on, AtomicOrdering::Relaxed);
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

/// Runs `f`, which reads an untrusted file. A panic in a decoder becomes an error; on a worker it
/// would lose the thread and leave its path marked busy for good.
pub fn catching<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(tr("this file crashed its decoder").into()))
}

/// What tells two versions of a file apart: when it was modified, and its length.
pub type Stamp = (SystemTime, u64);

pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Checks the path put in the returned slot every `interval`, on a thread of its own so that it
/// takes no repaints and a slow network folder can't hold up the UI. A change is sent once it has
/// held for a whole interval, not to load a half-written file. A file that's gone sends nothing.
pub fn watch(notify: Notify, interval: Duration) -> Arc<Mutex<Option<PathBuf>>> {
    let slot = Arc::new(Mutex::new(None));
    let shared = Arc::clone(&slot);
    std::thread::spawn(move || {
        let (mut path, mut known, mut last) = (None, None, None);
        while Arc::strong_count(&shared) > 1 {
            std::thread::sleep(interval);
            let now: Option<PathBuf> = shared.lock().ok().and_then(|p| p.clone());
            let current = now.as_deref().and_then(stamp);
            if now != path {
                (path, known, last) = (now, current, current);
                continue;
            }
            if let (Some(p), Some(new)) = (&path, current) {
                if known.is_none() {
                    known = current;
                } else if current != known && current == last {
                    known = current;
                    notify.send(Msg::Changed(p.clone(), new));
                }
            }
            last = current;
        }
    });
    slot
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
fn load(path: &Path, target: [u32; 2], color_managed: bool, on_preview: impl Fn(Frame) + Sync) -> Result<Loaded, String> {
    // The folder's neighbours get here unopened, and any of them may be a FIFO, which would block
    // this worker for ever, or a link to a device that never ends.
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(tr("this isn't a regular file").into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(tr("this image is too large to open").into());
    }
    let all = std::fs::read(path).map_err(|e| e.to_string())?;
    let file_size = all.len() as u64;
    // A RAW file is shown by its JPEG preview, which goes through the usual path from here.
    let (bytes, raw_orientation): (&[u8], _) = if raw::is_raw(path) {
        (raw::embedded_jpeg(&all).ok_or(tr("this RAW file has no embedded preview"))?, raw::orientation(&all))
    } else {
        (&all, None)
    };
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        let tree = svg::load(bytes)?;
        let frame = svg::render(&tree, 1).ok_or("the SVG renderer failed")?;
        return Ok(Loaded { frames: vec![frame], file_size, svg: Some(tree) });
    }
    // Goes by extension, so a file named anything else can't reach native code.
    // Converts frames from `profile` to sRGB, with colour management on.
    let manage = |frames: &mut [Frame], profile: Option<moxcms::ColorProfile>| {
        for frame in frames.iter_mut() {
            if let Some(profile) = profile.as_ref().filter(|_| color_managed) {
                color::convert(profile, frame);
            }
        }
    };
    if heif::is_native(path) {
        let mut frames = vec![to_frame(heif::decode(bytes)?, Duration::ZERO)];
        manage(&mut frames, catching(|| Ok(heif::color(bytes))).ok().flatten());
        return Ok(Loaded { frames, file_size, svg: None });
    }
    if jpegxl::is_jxl(bytes) {
        let mut frames = jpegxl::decode(bytes)?;
        manage(&mut frames, catching(|| Ok(jpegxl::color(bytes))).ok().flatten());
        return Ok(Loaded { frames, file_size, svg: None });
    }
    let mut reader = ImageReader::new(Cursor::new(bytes));
    // TGA has no magic bytes; the guess below keeps this when it finds nothing.
    if let Ok(format) = ImageFormat::from_path(path) {
        reader.set_format(format);
    }
    let reader = reader.with_guessed_format().map_err(|e| e.to_string())?;
    let format = reader.format();

    // An animation's profile, read before its frames are.
    let mut animation_icc = None;
    let animation: Option<Frames> = match format {
        Some(ImageFormat::Gif) => {
            let mut d = GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            // Unlike with `ImageReader`, it starts without any.
            d.set_limits(Limits::default()).map_err(|e| e.to_string())?;
            Some(d.into_frames())
        }
        Some(ImageFormat::Png) => {
            let mut d = PngDecoder::with_limits(Cursor::new(bytes), Limits::default()).map_err(|e| e.to_string())?;
            animation_icc = d.icc_profile().ok().flatten();
            if d.is_apng().map_err(|e| e.to_string())? { Some(d.apng().map_err(|e| e.to_string())?.into_frames()) } else { None }
        }
        Some(ImageFormat::WebP) => {
            let mut d = WebPDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            animation_icc = d.icc_profile().ok().flatten();
            let (w, h) = d.dimensions();
            if d.has_animation() && w as u64 * h as u64 * 4 > MAX_CANVAS_BYTES {
                return Err(tr("this image is too large to open").into());
            }
            d.has_animation().then(|| d.into_frames())
        }
        _ => None,
    };
    if let Some(animation) = animation {
        // shortcut: all frames are decoded before showing; stream them if huge GIFs are slow.
        let (mut frames, mut held) = (Vec::new(), 0);
        for f in animation {
            let f = f.map_err(|e| e.to_string())?;
            let (n, d) = f.delay().numer_denom_ms();
            let frame = to_frame(DynamicImage::ImageRgba8(f.into_buffer()), frame_delay(n.checked_div(d).unwrap_or(0)));
            held += frame.pixels.len() + size_of::<Frame>();
            frames.push(frame);
            // A long animation plays as far as it fits.
            if held >= MAX_ANIMATION_BYTES {
                break;
            }
        }
        if frames.is_empty() {
            return Err(tr("no frames").into());
        }
        manage(&mut frames, animation_icc.and_then(|icc| catching(|| Ok(color::to_srgb(&icc))).ok().flatten()));
        return Ok(Loaded { frames, file_size, svg: None });
    }

    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    // shortcut: PNG's gAMA/cHRM/cICP without an ICC profile are ignored; handle them if they're reported.
    let icc = if color_managed { decoder.icc_profile().ok().flatten() } else { None };
    // A profile that upsets the library costs the conversion, not the image.
    let profile = icc.and_then(|icc| catching(|| Ok(color::to_srgb(&icc))).ok().flatten());
    let finish = |img: DynamicImage| {
        let mut frame = to_frame(img, Duration::ZERO);
        if let Some(profile) = &profile {
            color::convert(profile, &mut frame);
        }
        frame
    };
    let exif = decoder.exif_metadata().ok().flatten();
    let orientation = match raw_orientation {
        Some(o) => o,
        None => decoder.orientation().map_err(|e| e.to_string())?,
    };
    if format == Some(ImageFormat::Jpeg)
        && let Some(thumb) = exif.as_deref().and_then(exif_thumbnail)
        && let Ok(mut img) = image::load_from_memory(thumb)
    {
        img.apply_orientation(orientation);
        on_preview(finish(img));
    }
    let turbo = (format == Some(ImageFormat::Jpeg) && turbo::available()).then(|| {
        let (w, h) = decoder.dimensions();
        let rotated = matches!(orientation, Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH);
        let den = scale_denominator(if rotated { (h, w) } else { (w, h) }, target);
        // The scope joins the reduced-size decode, so its preview always arrives before the full image.
        let (on_preview, finish) = (&on_preview, &finish);
        std::thread::scope(|s| {
            if let Some(den) = den {
                s.spawn(move || {
                    if let Some(small) = turbo::decode(bytes, den) {
                        let mut small = DynamicImage::ImageRgb8(small);
                        small.apply_orientation(orientation);
                        on_preview(finish(small));
                    }
                });
            }
            turbo::decode(bytes, 1)
        })
    });
    let mut img = match turbo.flatten() {
        Some(img) => DynamicImage::ImageRgb8(img),
        None => {
            // Most decoders ignore the allocation limit, and this would allocate what a tiny header asks for.
            if decoder.total_bytes() > MAX_CANVAS_BYTES {
                return Err(tr("this image is too large to open").into());
            }
            DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?
        }
    };
    img.apply_orientation(orientation);
    let img = if matches!(format, Some(ImageFormat::Hdr | ImageFormat::OpenExr)) { tone_map(img) } else { img };
    let first = finish(img);
    let pages = if format == Some(ImageFormat::Tiff) { tiff_pages(bytes) } else { Vec::new() };
    if pages.is_empty() {
        return Ok(Loaded { frames: vec![first], file_size, svg: None });
    }
    on_preview(first.clone());
    Ok(Loaded { frames: decode_pages(bytes.to_vec(), first, &pages, profile.as_ref()), file_size, svg: None })
}

/// How long an animation frame stays up. Like browsers: under 20 ms means "unspecified" and plays at 100 ms.
pub fn frame_delay(ms: u32) -> Duration {
    Duration::from_millis(if ms < 20 { 100 } else { ms.into() })
}

/// Linear HDR values to sRGB bytes. Extended Reinhard with the peak as white keeps everything
/// from 0 to 1 as it is, and only compresses what's brighter.
fn tone_map(img: DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let (px, channels): (&[f32], usize) = match &img {
        DynamicImage::ImageRgb32F(b) => (b.as_raw(), 3),
        DynamicImage::ImageRgba32F(b) => (b.as_raw(), 4),
        _ => return img,
    };
    let peak = px.chunks_exact(channels).flat_map(|p| &p[..3]).copied().filter(|v| v.is_finite()).fold(1.0, f32::max);
    const STEPS: usize = 1 << 14;
    let curve: Vec<u8> = (0..=STEPS)
        .map(|i| {
            let v = i as f32 / STEPS as f32;
            ((if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }) * 255.0 + 0.5) as u8
        })
        .collect();
    let map = |v: f32| {
        let v = if v.is_finite() { v.max(0.0) } else { 0.0 };
        curve[((v * (1.0 + v / (peak * peak)) / (1.0 + v)).min(1.0) * STEPS as f32 + 0.5) as usize]
    };
    let mut pixels = Vec::with_capacity(px.len());
    for p in px.chunks_exact(channels) {
        pixels.extend(p[..3].iter().map(|&v| map(v)));
        if channels == 4 {
            pixels.push((p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
    }
    let out = if channels == 4 { image::RgbaImage::from_raw(w, h, pixels).map(DynamicImage::ImageRgba8) } else { image::RgbImage::from_raw(w, h, pixels).map(DynamicImage::ImageRgb8) };
    out.expect("one pixel out for each one in")
}

/// Where a TIFF's pages after the first start. Thumbnails ("reduced-resolution" subfiles) aren't pages.
// shortcut: BigTIFF shows its first page only, because `tiff_readers` reads classic TIFF; extend it if that's reported.
fn tiff_pages(tiff: &[u8]) -> Vec<u32> {
    let Some((u16_at, u32_at)) = tiff_readers(tiff) else { return Vec::new() };
    let (mut pages, mut seen, mut next) = (Vec::new(), HashSet::new(), u32_at(4));
    while let Some(ifd) = next.filter(|&at| at != 0 && seen.len() < MAX_TIFF_IFDS && seen.insert(at)) {
        let Some(entries) = u16_at(ifd) else { break };
        let reduced = (0..entries).map(|i| ifd + 2 + i * 12).any(|entry| u16_at(entry) == Some(0x00FE) && u32_at(entry + 8).is_some_and(|flags| flags & 1 == 1));
        if !reduced && seen.len() > 1 {
            pages.push(ifd as u32);
        }
        next = u32_at(ifd + 2 + entries * 12);
    }
    pages
}

/// `first` and the pages at `pages`, on one canvas: `Doc` and the texture cache take every frame
/// to be the same size. Each page goes through `image` by pointing the header at it, which keeps
/// all its pixel formats. A page that won't decode is left out, and pages stop at the memory budget.
// shortcut: a small page looks small next to a big one; give pages their own size if that's reported.
fn decode_pages(mut tiff: Vec<u8>, first: Frame, pages: &[u32], profile: Option<&moxcms::ColorProfile>) -> Vec<Frame> {
    let mut frames = vec![first];
    let le = tiff[0] == b'I';
    let mut canvas = (frames[0].w, frames[0].h);
    for &ifd in pages {
        tiff[4..8].copy_from_slice(&if le { ifd.to_le_bytes() } else { ifd.to_be_bytes() });
        let Ok(mut page) = catching(|| decode_page(&tiff)) else { continue };
        // shortcut: every page takes the first page's profile; read each page's if mixed ones are reported.
        if let Some(profile) = profile {
            color::convert(profile, &mut page);
        }
        let grown = (canvas.0.max(page.w), canvas.1.max(page.h));
        if (frames.len() + 1) as u64 * grown.0 as u64 * grown.1 as u64 * 4 > MAX_ANIMATION_BYTES as u64 {
            break;
        }
        canvas = grown;
        frames.push(page);
    }
    for frame in &mut frames {
        pad(frame, canvas);
    }
    frames
}

fn decode_page(tiff: &[u8]) -> Result<Frame, String> {
    let mut decoder = ImageReader::with_format(Cursor::new(tiff), ImageFormat::Tiff).into_decoder().map_err(|e| e.to_string())?;
    if decoder.total_bytes() > MAX_CANVAS_BYTES {
        return Err(tr("this image is too large to open").into());
    }
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    Ok(to_frame(img, Duration::ZERO))
}

/// Centres `frame` on a black (or clear) canvas.
fn pad(frame: &mut Frame, (w, h): (u32, u32)) {
    if (frame.w, frame.h) == (w, h) {
        return;
    }
    let channels = if frame.alpha { 4 } else { 3 };
    let (x, y) = (((w - frame.w) / 2) as usize, ((h - frame.h) / 2) as usize);
    let mut pixels = vec![0; w as usize * h as usize * channels];
    for (i, row) in frame.pixels.chunks_exact(frame.w as usize * channels).enumerate() {
        let at = ((y + i) * w as usize + x) * channels;
        pixels[at..at + row.len()].copy_from_slice(row);
    }
    (frame.w, frame.h, frame.pixels) = (w, h, pixels);
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
    tiff.get(thumbnail_range(tiff)?)
}

/// Where `exif_thumbnail` is, if that's inside the block.
fn thumbnail_range(tiff: &[u8]) -> Option<std::ops::Range<usize>> {
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
    let range = offset..offset.checked_add(len?)?;
    tiff.get(range.clone()).map(|_| range)
}

pub type Reader<'a> = Box<dyn Fn(usize) -> Option<usize> + 'a>;

/// Bounds-checked u16 and u32 readers in the TIFF block's byte order.
/// Readers for a TIFF structure's integers. Olympus (`IIRO`, `MMOR`) and Panasonic (`IIU\0`)
/// RAW files change only the magic number.
pub fn tiff_readers(tiff: &[u8]) -> Option<(Reader<'_>, Reader<'_>)> {
    let le = match tiff.get(0..4)? {
        [b'I', b'I', 42, 0] | [b'I', b'I', b'R', b'O'] | [b'I', b'I', b'U', 0] => true,
        [b'M', b'M', 0, 42] | [b'M', b'M', b'O', b'R'] => false,
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

/// Whether the chain of IFDs in this TIFF block ends: a parser that follows every link never
/// stops on one that loops. Whatever can't be read counts as the end.
pub fn ifd_chain_ends(tiff: &[u8]) -> bool {
    let Some((u16_at, u32_at)) = tiff_readers(tiff) else { return true };
    // A multi-page TIFF has an IFD per page, so this goes by repeats, not length.
    let (mut ifd, mut seen) = (u32_at(4), HashSet::new());
    while seen.len() < MAX_TIFF_IFDS {
        match ifd {
            None | Some(0) => return true,
            Some(at) if !seen.insert(at) => return false,
            Some(at) => ifd = u16_at(at).and_then(|entries| u32_at(at + 2 + entries * 12)),
        }
    }
    false
}

/// Erases the EXIF thumbnail, for when the pixels no longer match it (e.g. after rotating), and
/// unlinks it. Only unlinking would leave the old picture in the file, cropped-out parts included.
// shortcut: a preview in the MakerNote survives; clear that too if Save As should drop all of them.
pub fn drop_exif_thumbnail(tiff: &mut [u8]) {
    let ptr = next_ifd_ptr(tiff);
    if let Some(range) = thumbnail_range(tiff) {
        tiff[range].fill(0);
    }
    if let Some(ptr) = ptr {
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
    let item = newest.ok_or_else(|| tr("It's no longer in the Trash").to_string())?;
    // The full name, or the file would come back without its extension.
    let name = path.file_name().unwrap_or_default().to_owned();
    trash::os_limited::restore_all([trash::TrashItem { name, ..item }]).map_err(|e| e.to_string())
}

/// Renames the file `path` to `name` in the same folder. Never replaces another file.
// shortcut: changing only the case on a case-insensitive file system counts as "already exists"; handle it if it's reported.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(tr("That isn't a valid file name").into());
    }
    let new = path.with_file_name(name);
    // `fs::rename` would silently replace it. This also sees a link to nowhere.
    if new.symlink_metadata().is_ok() {
        return Err(trf("{name} already exists", &[("name", &name)]));
    }
    std::fs::rename(path, &new).map_err(|e| e.to_string())?;
    Ok(new)
}

/// Whether `path` has the extension of a format lightview opens.
pub fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The first image in folder `dir`, in `sort` order.
pub fn first_image(dir: &Path, sort: SortOrder) -> Option<PathBuf> {
    // `list_folder` lists a file's siblings, and gives the file back when it can't read the folder.
    let child = dir.join("-");
    list_folder(&child, sort).into_iter().find(|p| *p != child)
}

/// The supported images in `path`'s folder: by name in file-manager order, or oldest first.
fn list_folder(path: &Path, sort: SortOrder) -> Vec<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![path.to_path_buf()] };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| is_image(p))
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

    /// Writes a 64×48 image of one colour, tagged with Display P3, as `name` in a new folder.
    fn wide_gamut(name: &str) -> PathBuf {
        use image::ImageEncoder;
        let dir = std::env::temp_dir().join(format!("lightview-wide-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 48, image::Rgb([200, 100, 50])));
        let icc = moxcms::ColorProfile::new_display_p3().encode().unwrap();
        let out = std::fs::File::create(&path).unwrap();
        if name.ends_with("png") {
            let mut enc = image::codecs::png::PngEncoder::new(out);
            enc.set_icc_profile(icc).unwrap();
            img.write_with_encoder(enc).unwrap();
        } else {
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(out, 100);
            enc.set_icc_profile(icc).unwrap();
            img.write_with_encoder(enc).unwrap();
        }
        path
    }

    #[test]
    fn a_wide_gamut_image_is_converted_unless_the_setting_is_off() {
        let path = wide_gamut("a.png");
        let on = load(&path, [100, 100], true, |_| {}).unwrap();
        assert!(on.frames[0].pixels[0].abs_diff(215) <= 2, "{:?}", &on.frames[0].pixels[..3]);
        let off = load(&path, [100, 100], false, |_| {}).unwrap();
        assert_eq!(off.frames[0].pixels[..3], [200, 100, 50]);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_preview_and_the_full_image_get_the_same_colours() {
        let path = wide_gamut("a.jpg");
        let previews = Mutex::new(Vec::new());
        let loaded = load(&path, [8, 8], true, |frame| previews.lock().unwrap().push(frame.pixels[0])).unwrap();
        let red = loaded.frames[0].pixels[0];
        assert!(red.abs_diff(215) <= 3, "{red}");
        if turbo::available() {
            let previews = previews.into_inner().unwrap();
            assert!(!previews.is_empty() && previews.iter().all(|r| r.abs_diff(red) <= 3), "{previews:?} vs {red}");
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn an_animation_is_converted_too() {
        use image::ImageEncoder;
        let dir = std::env::temp_dir().join(format!("lightview-wideanim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.webp");
        // A lossless WebP with a profile: the still path; then the same through the animation path is
        // checked by an APNG below.
        let mut enc = image::codecs::webp::WebPEncoder::new_lossless(std::fs::File::create(&path).unwrap());
        enc.set_icc_profile(moxcms::ColorProfile::new_display_p3().encode().unwrap()).unwrap();
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([200, 100, 50]))).write_with_encoder(enc).unwrap();
        assert!(load(&path, [100, 100], true, |_| {}).unwrap().frames[0].pixels[0].abs_diff(215) <= 2);
        // APNG with an iCCP chunk, written by hand: two frames.
        let apng = dir.join("b.png");
        std::fs::write(&apng, apng_with_icc(&moxcms::ColorProfile::new_display_p3().encode().unwrap())).unwrap();
        let l = load(&apng, [100, 100], true, |_| {}).unwrap();
        assert_eq!(l.frames.len(), 2);
        assert!(l.frames.iter().all(|f| f.pixels[0].abs_diff(215) <= 2), "{:?}", l.frames.iter().map(|f| f.pixels[0]).collect::<Vec<_>>());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A 2-frame 1×1 APNG of (200, 100, 50) with `icc` in an iCCP chunk.
    fn apng_with_icc(icc: &[u8]) -> Vec<u8> {
        let mut png = Vec::new();
        {
            let mut info = png::Info::with_size(1, 1);
            info.color_type = png::ColorType::Rgb;
            info.icc_profile = Some(std::borrow::Cow::Owned(icc.to_vec()));
            let mut enc = png::Encoder::with_info(&mut png, info).unwrap();
            enc.set_animated(2, 0).unwrap();
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[200, 100, 50]).unwrap();
            w.write_image_data(&[200, 100, 50]).unwrap();
            w.finish().unwrap();
        }
        png
    }

    #[test]
    fn a_broken_profile_costs_the_conversion_not_the_image() {
        use image::ImageEncoder;
        let dir = std::env::temp_dir().join(format!("lightview-badicc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        let mut enc = image::codecs::png::PngEncoder::new(std::fs::File::create(&path).unwrap());
        enc.set_icc_profile(vec![0xFF; 4096]).unwrap();
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([200, 100, 50]))).write_with_encoder(enc).unwrap();
        assert_eq!(load(&path, [100, 100], true, |_| {}).unwrap().frames[0].pixels[..3], [200, 100, 50]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_watcher_reports_a_settled_change_once() {
        let dir = std::env::temp_dir().join(format!("lightview-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        std::fs::write(&path, b"1").unwrap();
        let (tx, rx) = mpsc::channel();
        let every = Duration::from_millis(20);
        let slot = watch(Notify { tx, ctx: Arc::default() }, every);
        *slot.lock().unwrap() = Some(path.clone());
        let wait = Duration::from_millis(150);
        std::thread::sleep(wait);
        assert!(rx.try_recv().is_err(), "just watching isn't a change");
        std::fs::write(&path, b"22").unwrap();
        let Ok(Msg::Changed(changed, stamp_seen)) = rx.recv_timeout(Duration::from_secs(5)) else { panic!("no change seen") };
        assert_eq!((changed, Some(stamp_seen)), (path.clone(), stamp(&path)));
        std::thread::sleep(wait);
        assert!(rx.try_recv().is_err(), "one message per change");
        std::fs::remove_file(&path).unwrap();
        std::thread::sleep(wait);
        assert!(rx.try_recv().is_err(), "gone isn't a change");
        // Replaced, as an editor saving through a temporary file does.
        std::fs::write(&path, b"333").unwrap();
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)), Ok(Msg::Changed(..))));
        // Another file starts clean, and so does none.
        let other = dir.join("b.png");
        std::fs::write(&other, b"4").unwrap();
        *slot.lock().unwrap() = Some(other);
        std::thread::sleep(wait);
        assert!(rx.try_recv().is_err());
        *slot.lock().unwrap() = None;
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rename_never_replaces_a_file() {
        let dir = std::env::temp_dir().join(format!("lightview-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.png"), dir.join("b.png"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        assert!(rename(&a, "b.png").is_err());
        assert_eq!((std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()), (b"a".to_vec(), b"b".to_vec()));
        for bad in ["", ".", "..", "../c.png", "sub/c.png", "sub\\c.png"] {
            assert!(rename(&a, bad).is_err(), "{bad}");
        }
        assert!(a.exists());
        assert_eq!(rename(&a, "c d.png").unwrap(), dir.join("c d.png"));
        assert!(!a.exists() && dir.join("c d.png").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

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
        let loaded = load(&path, [100, 100], true, |_| {}).unwrap();
        std::fs::write(&path, "broken").unwrap();
        let broken = load(&path, [100, 100], true, |_| {});
        std::fs::remove_file(&path).unwrap();
        assert_eq!((loaded.frames[0].w, loaded.frames[0].h), (3, 2));
        assert!(loaded.svg.is_some() && broken.is_err());
    }

    #[test]
    fn opens_ico_with_its_transparency() {
        let path = std::env::temp_dir().join(format!("lightview-{}.ico", std::process::id()));
        image::RgbaImage::from_fn(3, 2, |x, _| image::Rgba([200, 100, 50, x as u8 * 100])).save(&path).unwrap();
        let loaded = load(&path, [100, 100], true, |_| {});
        std::fs::remove_file(&path).unwrap();
        let f = &loaded.unwrap().frames[0];
        assert_eq!((f.w, f.h, f.alpha), (3, 2, true));
        assert_eq!(&f.pixels[f.pixels.len() - 4..], [200, 100, 50, 200]);
    }

    /// Encodes through the `image` crate, then opens the file as `name` says.
    fn round_trip(name: &str, img: DynamicImage) -> Loaded {
        let mut bytes = Vec::new();
        let format = ImageFormat::from_path(name).unwrap();
        img.write_to(&mut Cursor::new(&mut bytes), format).unwrap();
        load_bytes(name, &bytes).unwrap()
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("tests/data/{name}")).unwrap()
    }

    #[test]
    fn opens_the_cheap_formats() {
        let img = image::RgbImage::from_fn(3, 2, |x, y| image::Rgb([x as u8 * 40, y as u8 * 40, 200]));
        for name in ["a.qoi", "a.tga", "a.ppm"] {
            let f = &round_trip(name, DynamicImage::ImageRgb8(img.clone())).frames[0];
            assert_eq!((f.w, f.h), (3, 2), "{name}");
            assert_eq!(&f.pixels[f.pixels.len() - 3..], [80, 40, 200], "{name}");
        }
        let f = &load_bytes("a.dds", &fixture("tiny.dds")).unwrap().frames[0];
        assert_eq!((f.w, f.h, &f.pixels[..3]), (4, 4, &[255, 0, 0][..]));
    }

    #[test]
    fn hdr_formats_are_tone_mapped() {
        let img = image::Rgb32FImage::from_fn(2, 1, |x, _| image::Rgb(if x == 0 { [0.5, 0.5, 0.5] } else { [100.0, 100.0, 100.0] }));
        for name in ["a.hdr", "a.exr"] {
            let f = &round_trip(name, DynamicImage::ImageRgb32F(img.clone())).frames[0];
            assert_eq!(f.pixels[3], 255, "{name}: the peak is white");
            assert!(f.pixels[0] > 100 && f.pixels[0] < 188, "{name}: {}", f.pixels[0]);
        }
    }

    #[test]
    fn tone_mapping() {
        let map = |v: Vec<f32>| tone_map(DynamicImage::ImageRgb32F(image::Rgb32FImage::from_raw(v.len() as u32 / 3, 1, v).unwrap())).into_bytes();
        // Within 0–1, only the sRGB curve.
        assert_eq!(map(vec![0.0, 0.214, 1.0]), [0, 127, 255]);
        assert_eq!(map(vec![f32::NAN, f32::INFINITY, -1.0, 100.0, 1.0, 0.0])[..4], [0, 0, 0, 255]);
        let rgba = tone_map(DynamicImage::ImageRgba32F(image::Rgba32FImage::from_raw(1, 1, vec![1.0, 1.0, 1.0, 0.5]).unwrap()));
        assert_eq!(rgba.into_bytes(), [255, 255, 255, 128]);
    }

    #[test]
    fn plays_apng_and_leaves_still_pngs_alone() {
        let loaded = load_bytes("anim.png", &fixture("anim.apng")).unwrap();
        assert_eq!(loaded.frames.iter().map(|f| f.delay.as_millis()).collect::<Vec<_>>(), [50, 200]);
        assert_eq!((loaded.frames[0].pixels[0], loaded.frames[1].pixels[0]), (0, 255));
        assert_eq!(round_trip("still.png", DynamicImage::ImageRgb8(image::RgbImage::new(2, 2))).frames[0].delay, Duration::ZERO);
    }

    #[test]
    fn opens_jpeg_xl() {
        for name in ["tiny.jxl", "lossless.jxl"] {
            let f = &load_bytes("a.jxl", &fixture(name)).unwrap().frames[0];
            assert_eq!((f.w, f.h, f.alpha), (64, 48, false), "{name}");
            let px = &f.pixels[(24 * 64 + 8) * 3..][..3];
            assert!(px[0] > 180 && px[2] < 80, "{name}: {px:?}");
        }
        let f = &load_bytes("a.jxl", &fixture("alpha.jxl")).unwrap().frames[0];
        assert_eq!((f.alpha, f.pixels[3]), (true, 128));
        // By content, like the other formats.
        assert_eq!(load_bytes("misnamed.png", &fixture("tiny.jxl")).unwrap().frames[0].w, 64);
    }

    /// A little-endian TIFF of 8-bit grey pages: width, height, shade, whether it's a thumbnail.
    fn tiff_file(pages: &[(u32, u32, u8, bool)]) -> Vec<u8> {
        let mut t = b"II\x2a\0\x08\0\0\0".to_vec();
        for (i, &(w, h, shade, reduced)) in pages.iter().enumerate() {
            let tags: [(u16, u16, u32); 9] = [(0xFE, 4, reduced as u32), (0x100, 4, w), (0x101, 4, h), (0x102, 3, 8), (0x103, 3, 1), (0x106, 3, 1), (0x111, 4, 0), (0x116, 4, h), (0x117, 4, w * h)];
            let start = t.len() as u32;
            let data = start + 2 + 9 * 12 + 4;
            let next = data + w * h;
            t.extend(9u16.to_le_bytes());
            for (tag, kind, value) in tags {
                t.extend(tag.to_le_bytes());
                t.extend(kind.to_le_bytes());
                t.extend(1u32.to_le_bytes());
                t.extend((if tag == 0x111 { data } else { value }).to_le_bytes());
            }
            t.extend((if i + 1 == pages.len() { 0 } else { next }).to_le_bytes());
            t.resize(next as usize, shade);
        }
        t
    }

    #[test]
    fn tiff_pages_share_one_canvas_and_skip_thumbnails() {
        let previews = Mutex::new(0);
        let path = std::env::temp_dir().join(format!("lightview-{}-pages.tif", std::process::id()));
        std::fs::write(&path, tiff_file(&[(2, 2, 10, false), (1, 1, 99, true), (4, 2, 200, false)])).unwrap();
        let loaded = load(&path, [100, 100], true, |_| *previews.lock().unwrap() += 1).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(loaded.paged());
        assert_eq!(*previews.lock().unwrap(), 1, "the first page shows while the rest decode");
        assert_eq!(loaded.frames.iter().map(|f| (f.w, f.h)).collect::<Vec<_>>(), [(4, 2), (4, 2)]);
        // Centred on black.
        assert_eq!(&loaded.frames[0].pixels[..9], [0, 0, 0, 10, 10, 10, 10, 10, 10]);
        assert_eq!(loaded.frames[1].pixels[0], 200);
        // One page, as before.
        assert!(!load_bytes("one.tif", &tiff_file(&[(2, 2, 10, false)])).unwrap().paged());
    }

    #[test]
    fn tiff_page_lists_end() {
        let mut t = tiff_file(&[(1, 1, 0, false), (1, 1, 0, false)]);
        // The second page's "next" points back at itself, or at the first.
        let second = u32::from_le_bytes(t[8 + 2 + 9 * 12..][..4].try_into().unwrap());
        let link = second as usize + 2 + 9 * 12;
        assert_eq!(tiff_pages(&t), [second]);
        for target in [8, second] {
            t[link..link + 4].copy_from_slice(&target.to_le_bytes());
            assert_eq!(tiff_pages(&t), [second]);
        }
        t[link..link + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(tiff_pages(&t), [second]);
        let good = tiff_file(&[(1, 1, 0, false), (1, 1, 0, false)]);
        for len in 0..good.len() {
            let _ = tiff_pages(&good[..len]);
        }
    }

    #[test]
    fn many_small_pages_cant_grow_to_a_giant_canvas() {
        let first = Frame { w: 30000, h: 30000, pixels: Vec::new(), alpha: false, delay: Duration::ZERO };
        let tiff = tiff_file(&[(1, 1, 0, false), (1, 1, 0, false)]);
        let frames = decode_pages(tiff.clone(), Frame { w: 1, h: 1, ..first }, &tiff_pages(&tiff), None);
        assert_eq!(frames.len(), 2);
    }

    #[test]
    fn a_native_format_with_another_name_never_reaches_its_decoder() {
        for name in ["tiny.heic", "tiny.avif"] {
            assert!(load_bytes("photo.jpg", &fixture(name)).is_err(), "{name}");
            assert!(load_bytes("photo.png", &fixture(name)).is_err(), "{name}");
        }
    }

    #[test]
    fn saying_too_much_in_a_header_is_refused() {
        let be = 60000u32.to_be_bytes();
        let mut qoi = b"qoif".to_vec();
        qoi.extend(be);
        qoi.extend(be);
        qoi.extend([4, 0, 0, 0, 0, 0, 0, 0, 1]);
        let mut tga = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 32, 0];
        tga.extend([0; 64]);
        let mut dds = fixture("tiny.dds");
        dds[12..20].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF, 0x7F]);
        let mut exr = Vec::new();
        DynamicImage::ImageRgb32F(image::Rgb32FImage::new(2, 2)).write_to(&mut Cursor::new(&mut exr), ImageFormat::OpenExr).unwrap();
        let window = exr.windows(10).position(|w| w == b"dataWindow").unwrap() + b"dataWindow\0box2i\0".len() + 4;
        exr[window + 8..window + 16].copy_from_slice(&[0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0]);
        let cases: [(&str, Vec<u8>); 6] = [
            ("a.qoi", qoi),
            ("a.tga", tga),
            ("a.pnm", b"P6\n60000 60000\n255\n\0\0\0".to_vec()),
            ("a.hdr", b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 60000 +X 60000\n\0\0".to_vec()),
            ("a.dds", dds),
            ("a.exr", exr),
        ];
        for (name, bytes) in cases {
            assert!(load_bytes(name, &bytes).is_err(), "{name}");
        }
    }

    #[test]
    fn a_giant_jpeg_xl_is_refused() {
        // 16384 × 16384 in 35 KB, which would take 1 GiB. `jxl`'s Level 5 limit refuses it first.
        assert!(load_bytes("a.jxl", &fixture("giant.jxl")).is_err());
    }

    #[test]
    fn cut_short_files_fail_cleanly() {
        let mut files: Vec<(&str, Vec<u8>)> = vec![("a.jxl", fixture("tiny.jxl")), ("a.jxl", fixture("alpha.jxl")), ("a.dds", fixture("tiny.dds")), ("a.png", fixture("anim.apng"))];
        let img = DynamicImage::ImageRgb32F(image::Rgb32FImage::new(4, 4));
        for (name, format) in [("a.exr", ImageFormat::OpenExr), ("a.hdr", ImageFormat::Hdr)] {
            let mut bytes = Vec::new();
            img.write_to(&mut Cursor::new(&mut bytes), format).unwrap();
            files.push((name, bytes));
        }
        let small = DynamicImage::ImageRgb8(image::RgbImage::new(4, 4));
        for (name, format) in [("a.qoi", ImageFormat::Qoi), ("a.tga", ImageFormat::Tga), ("a.ppm", ImageFormat::Pnm)] {
            let mut bytes = Vec::new();
            small.write_to(&mut Cursor::new(&mut bytes), format).unwrap();
            files.push((name, bytes));
        }
        files.push(("a.tif", tiff_file(&[(2, 2, 1, false), (2, 2, 2, false)])));
        for (name, bytes) in files {
            for len in 0..bytes.len() {
                let _ = load_bytes(name, &bytes[..len]);
            }
        }
    }

    #[test]
    fn opens_tiff() {
        let path = std::env::temp_dir().join(format!("lightview-{}.tif", std::process::id()));
        let img = image::RgbImage::from_fn(3, 2, |x, y| image::Rgb([x as u8 * 40, y as u8 * 40, 200]));
        img.save(&path).unwrap();
        let loaded = load(&path, [100, 100], true, |_| {});
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
        let loaded = load(&path, [1280, 720], true, |_| panic!("GIFs have no preview")).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(loaded.frames.len(), 2);
        assert_eq!((loaded.frames[0].w, loaded.frames[0].h, loaded.frames[0].alpha), (4, 2, true));
        // 50 ms kept; 0 ms ("unspecified") plays at 100 ms.
        assert_eq!(loaded.frames.iter().map(|f| f.delay.as_millis()).collect::<Vec<_>>(), [50, 100]);
        assert_eq!(loaded.frames[1].pixels[0], 255);
    }

    /// A GIF with a `w`×`h` canvas and `frames` one-pixel frames: tiny on disk, huge in memory.
    fn gif_bomb(w: u16, h: u16, frames: usize) -> Vec<u8> {
        let mut gif = b"GIF89a".to_vec();
        gif.extend(w.to_le_bytes());
        gif.extend(h.to_le_bytes());
        gif.extend([0x80, 0, 0, 0, 0, 0, 255, 255, 255]); // A two-colour table.
        for _ in 0..frames {
            gif.extend([0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 1, 0]);
        }
        gif.push(0x3B);
        gif
    }

    fn load_bytes(name: &str, bytes: &[u8]) -> Result<Loaded, String> {
        // Tests run in parallel, and many use the same names.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("lightview-{}-{n}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let loaded = load(&path, [1280, 720], true, |_| {});
        std::fs::remove_file(&path).unwrap();
        loaded
    }

    #[test]
    fn animations_stop_at_the_memory_budget() {
        let (canvas, frames) = (4000 * 4000 * 4, 20);
        assert!(canvas * frames > MAX_ANIMATION_BYTES);
        let loaded = load_bytes("bomb.gif", &gif_bomb(4000, 4000, frames)).unwrap();
        assert!(loaded.frames.len() > 1 && (loaded.frames.len() - 1) * canvas < MAX_ANIMATION_BYTES, "{} frames", loaded.frames.len());
    }

    #[test]
    fn giant_gif_canvases_are_refused() {
        assert!(load_bytes("giant.gif", &gif_bomb(u16::MAX, u16::MAX, 2)).is_err());
    }

    #[test]
    fn giant_animated_webp_canvases_are_refused() {
        let mut body = b"WEBP".to_vec();
        // VP8X: animated, canvas 16384×16384 (sizes are stored minus one, in 24 bits).
        body.extend(b"VP8X\x0a\0\0\0\x02\0\0\0\xff\x3f\0\xff\x3f\0");
        body.extend(b"ANIM\x06\0\0\0\0\0\0\0\0\0");
        body.extend(b"ANMF\x18\0\0\0");
        body.extend([0; 16]);
        body.extend(b"VP8L\0\0\0\0");
        let mut webp = b"RIFF".to_vec();
        webp.extend((body.len() as u32).to_le_bytes());
        webp.extend(body);
        assert_eq!(load_bytes("giant.webp", &webp).err().as_deref(), Some(tr("this image is too large to open")));
    }

    #[cfg(unix)]
    #[test]
    fn only_regular_files_are_read() {
        let dir = std::env::temp_dir().join(format!("lightview-special-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (fifo, link) = (dir.join("pipe.jpg"), dir.join("zero.jpg"));
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        std::os::unix::fs::symlink("/dev/zero", &link).unwrap();
        let results = [&fifo, &link].map(|p| load(p, [100, 100], true, |_| {}).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(results, [true, true]);
    }

    #[test]
    fn oversized_files_are_refused() {
        let path = std::env::temp_dir().join(format!("lightview-huge-{}.png", std::process::id()));
        std::fs::File::create(&path).unwrap().set_len(MAX_FILE_BYTES + 1).unwrap();
        let loaded = load(&path, [100, 100], true, |_| {});
        std::fs::remove_file(&path).unwrap();
        assert_eq!(loaded.err().as_deref(), Some(tr("this image is too large to open")));
    }

    #[test]
    fn panics_become_errors() {
        let result: Result<(), String> = catching(|| panic!("a decoder bug"));
        assert!(result.is_err());
    }

    #[test]
    fn dropped_thumbnail_leaves_no_trace() {
        for le in [true, false] {
            let mut t = tiff(le, b"JPEGDATA");
            drop_exif_thumbnail(&mut t);
            assert!(!t.windows(8).any(|w| w == b"JPEGDATA"), "the old picture is still in the file");
        }
    }

    #[test]
    fn ifd_chains() {
        let ifd = |at: usize, next: u32| {
            let mut t = b"II\x2a\0".to_vec();
            t.extend((at as u32).to_le_bytes());
            t.resize(at, 0);
            t.extend([0, 0]);
            t.extend(next.to_le_bytes());
            t
        };
        assert!(ifd_chain_ends(&ifd(8, 0)));
        assert!(ifd_chain_ends(&ifd(8, 0xFFFF_FFFF)), "a link out of bounds is the end");
        assert!(!ifd_chain_ends(&ifd(8, 8)), "links to itself");
        assert!(ifd_chain_ends(b"II"));
    }

    #[test]
    fn folder_filter_and_order() {
        let dir = std::env::temp_dir().join(format!("lightview-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["b10.JPG", "b2.png", "a.webp", "notes.txt", "clip.mp4", "layers.psd"] {
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

/// `load` for tests elsewhere: the frame's size, or the error.
#[cfg(test)]
pub fn load_for_test(path: &Path) -> Result<(u32, u32), String> {
    load(path, [1280, 720], true, |_| {}).map(|l| (l.frames[0].w, l.frames[0].h))
}
