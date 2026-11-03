use crate::i18n::trf;
use crate::edit::{self, Doc};
use crate::load::Frame;
use crate::tiff::{drop_exif_thumbnail, set_exif_dpi};
use image::codecs::{bmp::BmpEncoder, gif::GifEncoder, jpeg::{JpegEncoder, PixelDensity}, png::PngEncoder, tiff::TiffEncoder, webp::WebPEncoder};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageReader};
use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::{Path, PathBuf};

const JPEG_QUALITY: u8 = 90;

/// The formats Save As and Export write: name and extensions (the first is the default).
pub const FORMATS: [(&str, &[&str]); 6] = [
    ("JPEG", &["jpg", "jpeg"]),
    ("PNG", &["png"]),
    ("WebP", &["webp"]),
    ("BMP", &["bmp"]),
    ("TIFF", &["tif", "tiff"]),
    ("GIF", &["gif"]),
];

/// Formats that open but can't be written. Save As offers PNG for them, which keeps transparency.
const READ_ONLY: [&str; 25] = ["svg", "ico", "qoi", "tga", "pnm", "pbm", "pgm", "ppm", "pam", "dds", "hdr", "exr", "jxl", "heic", "heif", "hif", "avif", "cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2"];

/// `FORMATS` with the format of `source` first: file dialogs start on the first filter.
pub fn formats_for(source: &Path) -> [(&'static str, &'static [&'static str]); 6] {
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let ext = if READ_ONLY.contains(&ext.as_str()) { "png".to_string() } else { ext };
    let mut formats = FORMATS;
    formats.sort_by_key(|(_, exts)| !exts.contains(&ext.as_str()));
    formats
}

/// `source`, as PNG if its format can't be written.
fn writable(source: &Path) -> PathBuf {
    let read_only = source.extension().is_some_and(|e| READ_ONLY.iter().any(|r| e.eq_ignore_ascii_case(r)));
    if read_only { source.with_extension("png") } else { source.to_path_buf() }
}

/// The name Save As starts with: the source's, unless its format can't be written.
pub fn suggested_name(source: &Path) -> String {
    writable(source).file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// Claims the first free name for a copy next to `source` (`photo-edited.jpg`, then
/// `photo-edited-2.jpg`…) with an empty file, so that no two saves pick the same name and nothing
/// is overwritten. It has the source's permissions, which `write_atomic` keeps.
pub fn reserve_copy(source: &Path) -> Result<PathBuf, String> {
    let writable = writable(source);
    let (stem, ext) = (writable.file_stem().unwrap_or_default(), writable.extension());
    let mut n = 1;
    loop {
        let mut name = stem.to_os_string();
        name.push(if n == 1 { "-edited".to_string() } else { format!("-edited-{n}") });
        if let Some(ext) = ext {
            name.push(".");
            name.push(ext);
        }
        let dest = source.with_file_name(name);
        match File::options().write(true).create_new(true).open(&dest) {
            Ok(file) => {
                if let Ok(original) = std::fs::metadata(source) {
                    let _ = file.set_permissions(original.permissions());
                }
                return Ok(dest);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Saves `frame` with the edits in `doc` to `dest`, in the format of its extension.
/// EXIF from `source` is kept, with the orientation reset (the pixels are already upright)
/// and the old thumbnail unlinked. `keep_icc` is for pixels that are still in the source's colour
/// space (colour management off): its profile goes along. Converted pixels are sRGB, which needs
/// none. Writes to a temporary file first, so a failed save never damages an existing file
/// (including the original).
pub fn save(frame: &Frame, doc: &Doc, source: &Path, dest: &Path, keep_icc: bool) -> Result<(), String> {
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if !FORMATS.iter().any(|(_, exts)| exts.contains(&ext.as_str())) {
        return Err(trf("can't save as .{ext}: use .jpg, .png, .webp, .bmp, .tif or .gif", &[("ext", &ext)]));
    }
    let img = edit::render(frame, doc)?;
    let mut exif = source_exif(source);
    if let (Some(exif), Some(dpi)) = (&mut exif, doc.dpi) {
        set_exif_dpi(exif, dpi);
    }

    let icc = if keep_icc { source_icc(source) } else { None };
    write_atomic(dest, |out| encode(&img, &ext, exif, icc, doc.dpi, out).map_err(|e| e.to_string()))
}

/// Writes `dest` through a temporary file next to it and renames it into place,
/// so a failure never leaves `dest` half-written.
pub fn write_atomic(dest: &Path, write: impl FnOnce(&mut BufWriter<File>) -> Result<(), String>) -> Result<(), String> {
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or_default();
    let tmp = dest.with_extension(format!("{ext}.lightview-tmp"));
    let result = (|| {
        // A leftover goes first, and `create_new` then won't follow a link planted at this name.
        let _ = std::fs::remove_file(&tmp);
        let file = File::options().write(true).create_new(true).open(&tmp).map_err(|e| e.to_string())?;
        // Overwriting a private file keeps it private. Some file systems can't, which needn't fail the save.
        if let Ok(existing) = std::fs::metadata(dest) {
            let _ = file.set_permissions(existing.permissions());
        }
        let mut out = BufWriter::new(file);
        write(&mut out)?;
        out.into_inner().map_err(|e| e.to_string())?.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn encode(img: &DynamicImage, ext: &str, exif: Option<Vec<u8>>, icc: Option<Vec<u8>>, dpi: Option<u16>, out: &mut (impl Write + Seek)) -> image::ImageResult<()> {
    // Formats that can't hold EXIF or a profile just skip them.
    fn tagged<E: ImageEncoder>(mut enc: E, exif: Option<Vec<u8>>, icc: Option<Vec<u8>>) -> E {
        if let Some(exif) = exif {
            let _ = enc.set_exif_metadata(exif);
        }
        if let Some(icc) = icc {
            let _ = enc.set_icc_profile(icc);
        }
        enc
    }
    match ext {
        // JPEG has no alpha channel.
        "jpg" | "jpeg" => {
            let mut enc = JpegEncoder::new_with_quality(out, JPEG_QUALITY);
            if let Some(dpi) = dpi {
                enc.set_pixel_density(PixelDensity::dpi(dpi));
            }
            DynamicImage::ImageRgb8(img.to_rgb8()).write_with_encoder(tagged(enc, exif, icc))
        }
        "png" => img.write_with_encoder(tagged(PngEncoder::new(out), exif, icc)),
        "webp" => img.write_with_encoder(tagged(WebPEncoder::new_lossless(out), exif, icc)),
        "tif" | "tiff" => img.write_with_encoder(tagged(TiffEncoder::new(out), None, icc)),
        "gif" => DynamicImage::ImageRgba8(img.to_rgba8()).write_with_encoder(GifEncoder::new(out)),
        _ => img.write_with_encoder(BmpEncoder::new(out)),
    }
}

fn source_icc(source: &Path) -> Option<Vec<u8>> {
    ImageReader::open(source).ok()?.with_guessed_format().ok()?.into_decoder().ok()?.icc_profile().ok()?
}

/// The source's EXIF, ready to copy: orientation reset and thumbnail unlinked.
fn source_exif(source: &Path) -> Option<Vec<u8>> {
    let mut exif = ImageReader::open(source).ok()?.with_guessed_format().ok()?.into_decoder().ok()?.exif_metadata().ok()??;
    let _ = Orientation::remove_from_exif_chunk(&mut exif);
    drop_exif_thumbnail(&mut exif);
    Some(exif)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_takes_the_first_free_name() {
        let dir = std::env::temp_dir().join(format!("lightview-copy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("photo.jpg");
        std::fs::write(&source, b"original").unwrap();
        let names: Vec<PathBuf> = (0..3).map(|_| reserve_copy(&source).unwrap()).collect();
        assert_eq!(names, [dir.join("photo-edited.jpg"), dir.join("photo-edited-2.jpg"), dir.join("photo-edited-3.jpg")]);
        std::fs::write(dir.join("photo-edited-4.jpg"), b"someone else's").unwrap();
        assert_eq!(reserve_copy(&source).unwrap(), dir.join("photo-edited-5.jpg"));
        assert_eq!(std::fs::read(dir.join("photo-edited-4.jpg")).unwrap(), b"someone else's");
        assert_eq!(std::fs::read(source).unwrap(), b"original");
        std::fs::write(dir.join("logo.svg"), b"").unwrap();
        assert_eq!(reserve_copy(&dir.join("logo.svg")).unwrap(), dir.join("logo-edited.png"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// With colour management off the pixels are still in the source's space, so a copy without
    /// its profile would show the wrong colours.
    #[test]
    fn the_profile_goes_along_only_when_the_pixels_need_it() {
        let dir = std::env::temp_dir().join(format!("lightview-icc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let icc = moxcms::ColorProfile::new_display_p3().encode().unwrap();
        let source = dir.join("wide.png");
        let mut enc = PngEncoder::new(File::create(&source).unwrap());
        enc.set_icc_profile(icc.clone()).unwrap();
        DynamicImage::new_rgb8(2, 2).write_with_encoder(enc).unwrap();
        // `image` 0.25.10 reads no profile from a small TIFF through `ImageReader` (64×64 fails, 12 MP works), so photo-sized.
        let frame = Frame { w: 1000, h: 800, pixels: vec![0; 1000 * 800 * 3], alpha: false, delay: Duration::ZERO };
        let profile_of = |p: &Path| source_icc(p);
        for ext in ["png", "jpg", "webp", "tif"] {
            let dest = dir.join(format!("copy.{ext}"));
            save(&frame, &Doc::new(1000, 800), &source, &dest, true).unwrap();
            assert_eq!(profile_of(&dest).as_ref(), Some(&icc), "{ext}");
            save(&frame, &Doc::new(1000, 800), &source, &dest, false).unwrap();
            assert_eq!(profile_of(&dest), None, "{ext}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_copy_of_a_private_file_stays_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("lightview-private-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("photo.png");
        std::fs::write(&source, b"").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
        let dest = reserve_copy(&source).unwrap();
        let frame = Frame { w: 1, h: 1, pixels: vec![0; 3], alpha: false, delay: Duration::ZERO };
        save(&frame, &Doc::new(1, 1), &source, &dest, false).unwrap();
        assert_eq!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }
    use std::time::Duration;

    #[test]
    fn source_format_comes_first() {
        let first = |name: &str| formats_for(Path::new(name)).map(|(n, _)| n);
        assert_eq!(first("a.PNG"), ["PNG", "JPEG", "WebP", "BMP", "TIFF", "GIF"]);
        assert_eq!(first("a.jpeg")[0], "JPEG");
        assert_eq!(first("a.tiff")[0], "TIFF");
        assert_eq!(first("a.xyz"), FORMATS.map(|(n, _)| n));
    }

    #[test]
    fn view_only_formats_save_as_png() {
        for ext in crate::load::EXTENSIONS.iter().filter(|e| !FORMATS.iter().any(|(_, exts)| exts.contains(e))) {
            let name = format!("a.{ext}");
            assert_eq!(suggested_name(Path::new(&name)), "a.png");
            assert_eq!(formats_for(Path::new(&name))[0].0, "PNG");
        }
    }

    #[test]
    fn an_svg_saves_as_png_and_never_as_svg() {
        assert_eq!(formats_for(Path::new("a.SVG"))[0].0, "PNG");
        assert_eq!(formats_for(Path::new("a.ico"))[0].0, "PNG");
        assert_eq!(suggested_name(Path::new("/d/a.ICO")), "a.png");
        assert_eq!(suggested_name(Path::new("/d/a.b.svg")), "a.b.png");
        assert_eq!(suggested_name(Path::new("/d/a.jpg")), "a.jpg");
        let dest = temp_dir("svg").join("a.svg");
        std::fs::write(&dest, "original").unwrap();
        let frame = Frame { w: 1, h: 1, pixels: vec![0; 4], alpha: true, delay: Duration::ZERO };
        assert!(save(&frame, &Doc::new(1, 1), &dest, &dest, false).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"original");
        std::fs::remove_dir_all(dest.parent().unwrap()).unwrap();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lightview-save-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 3×2 RGB frame, red channel = pixel index.
    fn frame() -> Frame {
        Frame { w: 3, h: 2, pixels: (0..6u8).flat_map(|i| [i * 40, 0, 0]).collect(), alpha: false, delay: Duration::ZERO }
    }

    #[test]
    fn lossless_formats_round_trip_with_edits() {
        let dir = temp_dir("roundtrip");
        let source = dir.join("source.png");
        std::fs::write(&source, b"not an image").unwrap();
        let mut doc = Doc::new(3, 2);
        doc.rotate_right();
        doc.flip_horizontal();
        let expected = edit::render(&frame(), &doc).unwrap().to_rgb8();
        assert_eq!(expected, DynamicImage::ImageRgb8(image::RgbImage::from_raw(3, 2, frame().pixels).unwrap()).rotate90().fliph().to_rgb8());
        for ext in ["png", "webp", "bmp", "tif"] {
            let dest = dir.join(format!("out.{ext}"));
            save(&frame(), &doc, &source, &dest, false).unwrap();
            assert_eq!(image::open(&dest).unwrap().to_rgb8(), expected, "{ext}");
        }
        // No temporary files left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().contains("lightview-tmp")).collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn jpeg_keeps_exif_without_orientation() {
        let dir = temp_dir("exif");
        // A source JPEG with EXIF orientation 6 (rotate 90°) and 72 dpi, like a phone photo.
        let mut tiff = b"II\x2a\0\x08\0\0\0\x04\0".to_vec();
        tiff.extend([0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0]); // Orientation, SHORT, 1, value 6
        tiff.extend([0x1A, 0x01, 5, 0, 1, 0, 0, 0, 62, 0, 0, 0]); // XResolution, RATIONAL, at 62
        tiff.extend([0x1B, 0x01, 5, 0, 1, 0, 0, 0, 70, 0, 0, 0]); // YResolution, RATIONAL, at 70
        tiff.extend([0x28, 0x01, 3, 0, 1, 0, 0, 0, 3, 0, 0, 0]); // ResolutionUnit, SHORT, cm
        tiff.extend([0, 0, 0, 0]); // no IFD1
        tiff.extend([72, 0, 0, 0, 1, 0, 0, 0, 72, 0, 0, 0, 1, 0, 0, 0]); // 72/1, 72/1
        let source = dir.join("source.jpg");
        let mut enc = JpegEncoder::new(std::fs::File::create(&source).unwrap());
        enc.set_exif_metadata(tiff).unwrap();
        enc.write_image(&[0u8; 3 * 2 * 3], 3, 2, image::ExtendedColorType::Rgb8).unwrap();

        let dest = dir.join("saved.JPG");
        let doc = Doc { dpi: Some(300), ..Doc::new(3, 2) };
        save(&frame(), &doc, &source, &dest, false).unwrap();
        let mut decoder = ImageReader::open(&dest).unwrap().with_guessed_format().unwrap().into_decoder().unwrap();
        let exif = decoder.exif_metadata().unwrap();
        let jpeg = std::fs::read(&dest).unwrap();
        let m = little_exif::metadata::Metadata::new_from_path(&dest).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(Orientation::from_exif_chunk(&exif.unwrap()), Some(Orientation::NoTransforms));
        // The copied EXIF says 300 dpi too, not the source's 72 (per cm).
        use little_exif::exif_tag::ExifTag;
        for tag in [ExifTag::XResolution(Vec::new()), ExifTag::YResolution(Vec::new())] {
            match m.get_tag(&tag).next() {
                Some(ExifTag::XResolution(v) | ExifTag::YResolution(v)) => assert_eq!((v[0].nominator, v[0].denominator), (300, 1)),
                other => panic!("{other:?}"),
            }
        }
        assert!(matches!(m.get_tag(&ExifTag::ResolutionUnit(Vec::new())).next(), Some(ExifTag::ResolutionUnit(v)) if v == &[2]));
        // JFIF density: units = 1 (dpi), then 300 × 300.
        let jfif = jpeg.windows(5).position(|w| w == b"JFIF\0").unwrap();
        assert_eq!(&jpeg[jfif + 7..jfif + 12], &[1, 1, 44, 1, 44]);
    }

    #[test]
    fn gif_export() {
        let dir = temp_dir("gif");
        let dest = dir.join("out.gif");
        save(&frame(), &Doc::new(3, 2), &dir.join("missing.png"), &dest, false).unwrap();
        let back = image::open(&dest).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!((back.width(), back.height()), (3, 2));
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_symlink_is_not_followed() {
        let dir = temp_dir("symlink");
        let (victim, dest) = (dir.join("victim.txt"), dir.join("out.png"));
        std::fs::write(&victim, "precious").unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("out.png.lightview-tmp")).unwrap();
        write_atomic(&dest, |out| out.write_all(b"saved").map_err(|e| e.to_string())).unwrap();
        let (kept, saved) = (std::fs::read(&victim).unwrap(), std::fs::read(&dest).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!((&kept[..], &saved[..]), (&b"precious"[..], &b"saved"[..]));
    }

    #[cfg(unix)]
    #[test]
    fn overwriting_keeps_a_private_file_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let dest = dir.join("private.png");
        std::fs::write(&dest, "old").unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_atomic(&dest, |out| out.write_all(b"new").map_err(|e| e.to_string())).unwrap();
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn failed_save_keeps_existing_file() {
        let dir = temp_dir("fail");
        let dest = dir.join("keep.png");
        std::fs::write(&dest, b"original").unwrap();
        // A frame whose pixel buffer is too short fails before anything is written.
        let bad = Frame { pixels: vec![0; 3], ..frame() };
        assert!(save(&bad, &Doc::new(3, 2), &dest, &dest, false).is_err());
        assert!(save(&frame(), &Doc::new(3, 2), &dest, &dir.join("x.heic"), false).is_err());
        let kept = std::fs::read(&dest).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(kept, b"original");
    }
}
