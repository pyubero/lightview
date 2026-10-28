use crate::load::{Frame, drop_exif_thumbnail};
use crate::view::Transform;
use image::codecs::{bmp::BmpEncoder, jpeg::JpegEncoder, png::PngEncoder, webp::WebPEncoder};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageReader};
use std::io::{BufWriter, Write};
use std::path::Path;

const JPEG_QUALITY: u8 = 90;

/// Saves `frame` with `transform` applied to `dest`, in the format of its extension.
/// EXIF from `source` is kept, with the orientation reset (the pixels are already upright)
/// and the old thumbnail unlinked. Writes to a temporary file first, so a failed save never
/// damages an existing file (including the original).
pub fn save(frame: &Frame, transform: Transform, source: &Path, dest: &Path) -> Result<(), String> {
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if !["jpg", "jpeg", "png", "webp", "bmp"].contains(&ext.as_str()) {
        return Err(format!("can't save as .{ext}: use .jpg, .png, .webp or .bmp"));
    }
    let img = if frame.alpha {
        DynamicImage::ImageRgba8(image::RgbaImage::from_raw(frame.w, frame.h, frame.pixels.clone()).ok_or("bad frame")?)
    } else {
        DynamicImage::ImageRgb8(image::RgbImage::from_raw(frame.w, frame.h, frame.pixels.clone()).ok_or("bad frame")?)
    };
    let img = transform.apply(img);
    let exif = source_exif(source);

    let tmp = dest.with_extension(format!("{ext}.lightview-tmp"));
    let result = (|| {
        let mut out = BufWriter::new(std::fs::File::create(&tmp).map_err(|e| e.to_string())?);
        encode(&img, &ext, exif, &mut out).map_err(|e| e.to_string())?;
        out.into_inner().map_err(|e| e.to_string())?.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn encode(img: &DynamicImage, ext: &str, exif: Option<Vec<u8>>, out: &mut impl Write) -> image::ImageResult<()> {
    // Formats that can't hold EXIF just skip it.
    fn with_exif<E: ImageEncoder>(mut enc: E, exif: Option<Vec<u8>>) -> E {
        if let Some(exif) = exif {
            let _ = enc.set_exif_metadata(exif);
        }
        enc
    }
    match ext {
        // JPEG has no alpha channel.
        "jpg" | "jpeg" => DynamicImage::ImageRgb8(img.to_rgb8()).write_with_encoder(with_exif(JpegEncoder::new_with_quality(out, JPEG_QUALITY), exif)),
        "png" => img.write_with_encoder(with_exif(PngEncoder::new(out), exif)),
        "webp" => img.write_with_encoder(with_exif(WebPEncoder::new_lossless(out), exif)),
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
    fn lossless_formats_round_trip_with_transform() {
        let dir = temp_dir("roundtrip");
        let source = dir.join("source.png");
        std::fs::write(&source, b"not an image").unwrap();
        let t = Transform::default().rotate_right().flip_horizontal();
        let expected = t.apply(DynamicImage::ImageRgb8(image::RgbImage::from_raw(3, 2, frame().pixels).unwrap())).to_rgb8();
        for ext in ["png", "webp", "bmp"] {
            let dest = dir.join(format!("out.{ext}"));
            save(&frame(), t, &source, &dest).unwrap();
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
        // A source JPEG with EXIF orientation 6 (rotate 90°).
        let mut tiff = b"II\x2a\0\x08\0\0\0\x01\0".to_vec();
        tiff.extend([0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0]); // Orientation, SHORT, 1, value 6
        tiff.extend([0, 0, 0, 0]); // no IFD1
        let source = dir.join("source.jpg");
        let mut enc = JpegEncoder::new(std::fs::File::create(&source).unwrap());
        enc.set_exif_metadata(tiff).unwrap();
        enc.write_image(&[0u8; 3 * 2 * 3], 3, 2, image::ExtendedColorType::Rgb8).unwrap();

        let dest = dir.join("saved.JPG");
        save(&frame(), Transform::default(), &source, &dest).unwrap();
        let mut decoder = ImageReader::open(&dest).unwrap().with_guessed_format().unwrap().into_decoder().unwrap();
        let exif = decoder.exif_metadata().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(Orientation::from_exif_chunk(&exif.unwrap()), Some(Orientation::NoTransforms));
    }

    #[test]
    fn failed_save_keeps_existing_file() {
        let dir = temp_dir("fail");
        let dest = dir.join("keep.png");
        std::fs::write(&dest, b"original").unwrap();
        // A frame whose pixel buffer is too short fails before anything is written.
        let bad = Frame { pixels: vec![0; 3], ..frame() };
        assert!(save(&bad, Transform::default(), &dest, &dest).is_err());
        assert!(save(&frame(), Transform::default(), &dest, &dir.join("x.gif")).is_err());
        let kept = std::fs::read(&dest).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(kept, b"original");
    }
}
