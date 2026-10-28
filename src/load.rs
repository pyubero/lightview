use eframe::egui;
use image::codecs::{gif::GifDecoder, webp::WebPDecoder};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::Duration;

const EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "webp", "bmp"];

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
}

pub enum Msg {
    Folder(Vec<PathBuf>),
    Preview(PathBuf, Frame),
    Full(PathBuf, Loaded),
    Error(PathBuf, String),
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
}

impl Loader {
    pub fn new(notify: Notify, workers: usize) -> Self {
        let queue: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        for _ in 0..workers {
            let (queue, notify) = (queue.clone(), notify.clone());
            std::thread::spawn(move || loop {
                let path = {
                    let mut q = queue.1.wait_while(queue.0.lock().unwrap(), |q| q.wanted.is_empty()).unwrap();
                    let path = q.wanted.remove(0);
                    q.busy.insert(path.clone());
                    path
                };
                let msg = match load(&path, |frame| notify.send(Msg::Preview(path.clone(), frame))) {
                    Ok(loaded) => Msg::Full(path.clone(), loaded),
                    Err(e) => Msg::Error(path.clone(), e),
                };
                queue.0.lock().unwrap().busy.remove(&path);
                notify.send(msg);
            });
        }
        Loader { queue }
    }

    /// Replaces the wanted list (most important first). Stale requests are dropped;
    /// paths already being decoded aren't queued twice.
    pub fn want(&self, paths: Vec<PathBuf>) {
        let mut q = self.queue.0.lock().unwrap();
        q.wanted = paths.into_iter().filter(|p| !q.busy.contains(p)).collect();
        self.queue.1.notify_all();
    }
}

/// Decodes a file. For JPEGs with an EXIF thumbnail, `on_preview` gets it first.
fn load(path: &Path, on_preview: impl Fn(Frame)) -> Result<Loaded, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let file_size = bytes.len() as u64;
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
            return Err("no frames".into());
        }
        return Ok(Loaded { frames, file_size });
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
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    Ok(Loaded { frames: vec![to_frame(img, Duration::ZERO)], file_size })
}

fn to_frame(img: DynamicImage, delay: Duration) -> Frame {
    let (w, h, alpha) = (img.width(), img.height(), img.color().has_alpha());
    let pixels = if alpha { img.into_rgba8().into_raw() } else { img.into_rgb8().into_raw() };
    Frame { w, h, pixels, alpha, delay }
}

/// The JPEG thumbnail in IFD1 of an EXIF (TIFF) block, if any. The file is untrusted:
/// every read is bounds-checked and anything malformed returns `None`.
fn exif_thumbnail(tiff: &[u8]) -> Option<&[u8]> {
    let le = match tiff.get(0..4)? {
        [b'I', b'I', 42, 0] => true,
        [b'M', b'M', 0, 42] => false,
        _ => return None,
    };
    let u16_at = |o: usize| {
        let b = tiff.get(o..o.checked_add(2)?)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) } as usize)
    };
    let u32_at = |o: usize| {
        let b: [u8; 4] = tiff.get(o..o.checked_add(4)?)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) } as usize)
    };
    let ifd0 = u32_at(4)?;
    let ifd1 = u32_at(ifd0 + 2 + u16_at(ifd0)? * 12)?;
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

/// The supported images in `path`'s folder, in file-manager order.
pub fn list_folder(path: &Path) -> Vec<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![path.to_path_buf()] };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())))
        .collect();
    let name = |p: &PathBuf| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
    files.sort_by(|a, b| {
        let (a, b) = (name(a), name(b));
        natural_cmp(&a, &b).then_with(|| a.cmp(&b))
    });
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
    fn natural_order() {
        assert_eq!(natural_cmp("img2.jpg", "img10.jpg"), Ordering::Less);
        assert_eq!(natural_cmp("IMG_b.jpg", "img_A.jpg"), Ordering::Greater);
        assert_eq!(natural_cmp("a007", "a7"), Ordering::Equal);
        assert_eq!(natural_cmp("photo", "photo1"), Ordering::Less);
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
        let loaded = load(&path, |_| panic!("GIFs have no preview")).unwrap();
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
        let names: Vec<String> = list_folder(&dir.join("a.webp")).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["a.webp", "b2.png", "b10.JPG"]);
    }
}
