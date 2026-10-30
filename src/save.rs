use crate::i18n::trf;
use crate::edit::{self, Doc};
use crate::load::{Frame, drop_exif_thumbnail, set_exif_dpi};
use image::codecs::{bmp::BmpEncoder, gif::GifEncoder, jpeg::{JpegEncoder, PixelDensity}, png::PngEncoder, tiff::TiffEncoder, webp::WebPEncoder};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageReader};
use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::Path;

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

/// Saves `frame` with the edits in `doc` to `dest`, in the format of its extension.
/// EXIF from `source` is kept, with the orientation reset (the pixels are already upright)
/// and the old thumbnail unlinked. Writes to a temporary file first, so a failed save never
/// damages an existing file (including the original).
pub fn save(frame: &Frame, doc: &Doc, source: &Path, dest: &Path) -> Result<(), String> {
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if !FORMATS.iter().any(|(_, exts)| exts.contains(&ext.as_str())) {
        return Err(trf("can't save as .{ext}: use .jpg, .png, .webp, .bmp, .tif or .gif", &[("ext", &ext)]));
    }
    let img = edit::render(frame, doc)?;
    let mut exif = source_exif(source);
    if let (Some(exif), Some(dpi)) = (&mut exif, doc.dpi) {
        set_exif_dpi(exif, dpi);
    }

    write_atomic(dest, |out| encode(&img, &ext, exif, doc.dpi, out).map_err(|e| e.to_string()))
}

/// Writes `dest` through a temporary file next to it and renames it into place,
/// so a failure never leaves `dest` half-written.
pub fn write_atomic(dest: &Path, write: impl FnOnce(&mut BufWriter<File>) -> Result<(), String>) -> Result<(), String> {
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or_default();
    let tmp = dest.with_extension(format!("{ext}.lightview-tmp"));
    let result = (|| {
        let mut out = BufWriter::new(File::create(&tmp).map_err(|e| e.to_string())?);
        write(&mut out)?;
        out.into_inner().map_err(|e| e.to_string())?.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn encode(img: &DynamicImage, ext: &str, exif: Option<Vec<u8>>, dpi: Option<u16>, out: &mut (impl Write + Seek)) -> image::ImageResult<()> {
    // Formats that can't hold EXIF just skip it.
    fn with_exif<E: ImageEncoder>(mut enc: E, exif: Option<Vec<u8>>) -> E {
        if let Some(exif) = exif {
            let _ = enc.set_exif_metadata(exif);
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
            DynamicImage::ImageRgb8(img.to_rgb8()).write_with_encoder(with_exif(enc, exif))
        }
        "png" => img.write_with_encoder(with_exif(PngEncoder::new(out), exif)),
        "webp" => img.write_with_encoder(with_exif(WebPEncoder::new_lossless(out), exif)),
        "tif" | "tiff" => img.write_with_encoder(TiffEncoder::new(out)),
        "gif" => DynamicImage::ImageRgba8(img.to_rgba8()).write_with_encoder(GifEncoder::new(out)),
        _ => img.write_with_encoder(BmpEncoder::new(out)),
    }
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
    use std::time::Duration;

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
            save(&frame(), &doc, &source, &dest).unwrap();
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
        save(&frame(), &doc, &source, &dest).unwrap();
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
        save(&frame(), &Doc::new(3, 2), &dir.join("missing.png"), &dest).unwrap();
        let back = image::open(&dest).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!((back.width(), back.height()), (3, 2));
    }

    #[test]
    fn failed_save_keeps_existing_file() {
        let dir = temp_dir("fail");
        let dest = dir.join("keep.png");
        std::fs::write(&dest, b"original").unwrap();
        // A frame whose pixel buffer is too short fails before anything is written.
        let bad = Frame { pixels: vec![0; 3], ..frame() };
        assert!(save(&bad, &Doc::new(3, 2), &dest, &dest).is_err());
        assert!(save(&frame(), &Doc::new(3, 2), &dest, &dir.join("x.heic")).is_err());
        let kept = std::fs::read(&dest).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(kept, b"original");
    }
}
