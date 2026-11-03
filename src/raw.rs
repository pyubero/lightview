//! Camera RAW files, shown by the JPEG preview the camera stores in them: instant, and no
//! demosaicing, which is a viewer's job. Every file here is untrusted, and neighbours are read
//! unopened, so each read is bounds-checked and each search is capped.
use crate::tiff::tiff_readers;
use image::metadata::Orientation;
use std::path::Path;

pub const EXTENSIONS: [&str; 8] = ["cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2"];

/// JPEG starts looked at. Real files have a handful, plus chance matches in compressed data.
const MAX_CANDIDATES: usize = 256;
/// Markers walked in one JPEG before its frame header.
const MAX_MARKERS: usize = 64;

/// By extension: most of these look like any TIFF from the inside.
pub fn is_raw(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The pixel count of the colour JPEG starting at `at`, from its frame header. Lossless raw data
/// (SOF3), greyscale strips and chance matches give `None`.
fn jpeg_pixels(b: &[u8], at: usize) -> Option<u64> {
    let mut j = at.checked_add(2)?;
    for _ in 0..MAX_MARKERS {
        if *b.get(j)? != 0xFF {
            return None;
        }
        match *b.get(j + 1)? {
            0xFF => j += 1,
            0xC0..=0xC2 => {
                let s = b.get(j + 5..j + 10)?;
                let (h, w) = (u16::from_be_bytes([s[0], s[1]]) as u64, u16::from_be_bytes([s[2], s[3]]) as u64);
                return (s[4] == 3 && w > 0 && h > 0).then_some(w * h);
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF | 0xD9 | 0xDA => return None,
            _ => {
                let len = u16::from_be_bytes([*b.get(j + 2)?, *b.get(j + 3)?]) as usize;
                if len < 2 {
                    return None;
                }
                j += 2 + len;
            }
        }
    }
    None
}

/// The biggest JPEG preview in `b`, to the end of the file: decoders stop at its end marker.
// shortcut: only the biggest is tried, and uncompressed TIFF previews (some old NEFs and DNGs) aren't read; try the next one if a sample needs it.
pub fn embedded_jpeg(b: &[u8]) -> Option<&[u8]> {
    let mut best = None::<(u64, usize)>;
    for at in memchr::memmem::find_iter(b, &[0xFF, 0xD8, 0xFF]).take(MAX_CANDIDATES) {
        if let Some(px) = jpeg_pixels(b, at)
            && best.is_none_or(|(p, _)| px > p)
        {
            best = Some((px, at));
        }
    }
    best.map(|(_, at)| &b[at..])
}

/// The preview's header segments as a JPEG of their own (no image data), if one is EXIF: all an
/// EXIF reader needs, without handing it the tens of megabytes behind.
pub fn exif_header(jpeg: &[u8]) -> Option<Vec<u8>> {
    let (mut j, mut exif) = (2, false);
    for _ in 0..MAX_MARKERS {
        if *jpeg.get(j)? != 0xFF {
            return None;
        }
        let marker = *jpeg.get(j + 1)?;
        if marker == 0xDA || (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            break;
        }
        let len = u16::from_be_bytes([*jpeg.get(j + 2)?, *jpeg.get(j + 3)?]) as usize;
        exif |= marker == 0xE1 && jpeg.get(j + 4..j + 10) == Some(b"Exif\0\0");
        j += 2 + len.max(2);
    }
    exif.then_some(())?;
    Some([jpeg.get(..j)?, &[0xFF, 0xD9]].concat())
}

/// The orientation the camera recorded in the container, which most previews lack: IFD0 of the
/// TIFF structure (CR2, NEF, ARW, DNG, ORF, RW2) or of CR3's `CMT1` box. `None` for RAF, whose
/// preview carries its own EXIF.
pub fn orientation(b: &[u8]) -> Option<Orientation> {
    let tiff = if is_cr3(b) { cr3_box(b, b"CMT1")? } else { b };
    let (u16_at, u32_at) = tiff_readers(tiff)?;
    let ifd0 = u32_at(4)?;
    // u16 entries at most, each read bounds-checked.
    for i in 0..u16_at(ifd0)? {
        let entry = ifd0 + 2 + i * 12;
        if u16_at(entry)? == 0x0112 {
            return Orientation::from_exif(u16_at(entry + 8)? as u8);
        }
    }
    None
}

/// An ISO box file, which among RAW formats means CR3.
pub fn is_cr3(b: &[u8]) -> bool {
    b.get(4..8) == Some(b"ftyp")
}

/// The TIFF structure in a CR3's metadata box, found near the start of the file: `CMT1` holds
/// IFD0, `CMT2` the Exif tags.
pub fn cr3_box<'a>(b: &'a [u8], name: &[u8; 4]) -> Option<&'a [u8]> {
    let head = &b[..b.len().min(1 << 16)];
    let at = head.windows(4).position(|w| w == name)?;
    let size = u32::from_be_bytes(b.get(at.checked_sub(4)?..at)?.try_into().ok()?) as usize;
    b.get(at + 4..(at - 4).checked_add(size)?)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use image::ImageEncoder;

    /// A real colour JPEG of `w` × `h`.
    pub fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut out).write_image(&vec![90; (w * h * 3) as usize], w, h, image::ExtendedColorType::Rgb8).unwrap();
        out
    }

    /// A little-endian TIFF whose IFD0 holds an orientation, with `parts` after it.
    pub fn tiff(magic: &[u8; 4], orientation: u16, parts: &[&[u8]]) -> Vec<u8> {
        let mut t = magic.to_vec();
        t.extend(8u32.to_le_bytes());
        t.extend(1u16.to_le_bytes());
        t.extend(0x0112u16.to_le_bytes());
        t.extend(3u16.to_le_bytes());
        t.extend(1u32.to_le_bytes());
        t.extend((orientation as u32).to_le_bytes());
        t.extend(0u32.to_le_bytes());
        for p in parts {
            t.extend(*p);
        }
        t
    }

    fn size_of_found(b: &[u8]) -> Option<(u32, u32)> {
        image::load_from_memory(embedded_jpeg(b)?).ok().map(|i| (i.width(), i.height()))
    }

    #[test]
    fn the_biggest_colour_preview_wins() {
        // A lossless strip (SOF3) bigger than everything: what CR2 and DNG keep their raw data in.
        let mut sof3 = jpeg(64, 64);
        let i = sof3.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        sof3[i + 1] = 0xC3;
        let b = tiff(b"II*\0", 1, &[&jpeg(16, 12), &jpeg(40, 30), b"\x00\x01", &sof3, &jpeg(20, 10)]);
        assert_eq!(size_of_found(&b), Some((40, 30)));
    }

    #[test]
    fn orientation_from_each_container() {
        for magic in [b"II*\0", b"IIRO", b"IIU\0"] {
            assert_eq!(orientation(&tiff(magic, 6, &[])), Some(Orientation::Rotate90), "{magic:?}");
        }
        assert_eq!(orientation(&tiff(b"II*\0", 1, &[])), Some(Orientation::NoTransforms));
        // CR3: an ISO box file with the TIFF in a CMT1 box.
        let cmt1 = tiff(b"II*\0", 8, &[]);
        let mut cr3 = b"\0\0\0\x18ftypcrx \0\0\0\x01crx isom".to_vec();
        cr3.extend(((cmt1.len() + 8) as u32).to_be_bytes());
        cr3.extend(b"CMT1");
        cr3.extend(&cmt1);
        cr3.extend(jpeg(8, 8));
        assert_eq!(orientation(&cr3), Some(Orientation::Rotate270));
        assert_eq!(size_of_found(&cr3), Some((8, 8)));
        // RAF has none in the container.
        let mut raf = b"FUJIFILMCCD-RAW 0201FF383501".to_vec();
        raf.extend(jpeg(8, 8));
        assert_eq!(orientation(&raf), None);
        assert_eq!(size_of_found(&raf), Some((8, 8)));
    }

    #[test]
    fn only_the_header_of_a_preview_with_exif_is_kept() {
        assert!(exif_header(&jpeg(8, 8)).is_none(), "no EXIF");
        let mut with = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new(&mut with);
        enc.set_exif_metadata(tiff(b"II*\0", 6, &[])).unwrap();
        enc.write_image(&[90; 64 * 64 * 3], 64, 64, image::ExtendedColorType::Rgb8).unwrap();
        let head = exif_header(&with).unwrap();
        assert!(head.len() < with.len() && head.ends_with(&[0xFF, 0xD9]));
        assert!(head.windows(6).any(|w| w == b"Exif\0\0"));
        for len in 0..with.len().min(400) {
            let _ = exif_header(&with[..len]);
        }
    }

    #[test]
    fn broken_and_hostile_files_find_nothing_quickly() {
        assert!(embedded_jpeg(b"").is_none() && orientation(b"").is_none());
        let good = tiff(b"II*\0", 3, &[&jpeg(8, 8)]);
        for len in 0..good.len() {
            let _ = (embedded_jpeg(&good[..len]), orientation(&good[..len]));
        }
        // IFD0 pointing past the end, and a CMT1 box claiming 4 GiB.
        let mut far = good.clone();
        far[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(orientation(&far).is_none());
        let mut cr3 = b"\0\0\0\x18ftypcrx \0\0\0\x01crx isom\xff\xff\xff\xffCMT1".to_vec();
        cr3.extend(&good);
        assert!(orientation(&cr3).is_none());
        // 64 MiB of JPEG starts, each with a long chain of markers: linear, and capped.
        let mut flood = Vec::with_capacity(64 << 20);
        while flood.len() < 64 << 20 {
            flood.extend([0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x02]);
        }
        flood.extend(jpeg(8, 8));
        let t = std::time::Instant::now();
        let _ = embedded_jpeg(&flood);
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
        // A frame header claiming 65535 × 65535 is only a size here; the decoder's limits refuse it.
        let mut huge = jpeg(8, 8);
        let i = huge.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        huge[i + 5..i + 9].copy_from_slice(&[0xFF; 4]);
        assert!(embedded_jpeg(&huge).is_some());
    }

    /// One real file of each kind: `LIGHTVIEW_RAW=<folder> cargo test --release raw_samples -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn raw_samples() {
        let dir = std::env::var("LIGHTVIEW_RAW").unwrap();
        let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| is_raw(p)).collect();
        paths.sort();
        for path in paths {
            let b = std::fs::read(&path).unwrap();
            let t = std::time::Instant::now();
            let found = embedded_jpeg(&b).map(|j| j.len());
            let scan = t.elapsed();
            let o = orientation(&b);
            let t = std::time::Instant::now();
            let loaded = crate::load::load_for_test(&path);
            let load = t.elapsed();
            let t = std::time::Instant::now();
            let ins = crate::inspector::Inspector::new(&path);
            let inspect = t.elapsed();
            let exif = ins.exif.as_ref().map(|_| "EXIF ok").map_err(String::as_str);
            eprintln!("{:<14} preview {:>9?} B  orientation {o:?}  scan {scan:?}  load {load:?} → {loaded:?}  inspector {inspect:?} {exif:?}", path.file_name().unwrap().to_string_lossy(), found);
        }
    }
}
