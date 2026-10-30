use crate::i18n::{tr, tr_key, trf};
use crate::save::write_atomic;
use eframe::egui;
use image::{ImageDecoder, ImageReader};
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::ifd::ExifTagGroup;
use little_exif::metadata::Metadata;
use std::io::{Cursor, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

/// The editable fields: label and the tag they map to.
const FIELDS: [(&str, fn(String) -> ExifTag); 6] = [
    (tr_key("Description"), ExifTag::ImageDescription),
    (tr_key("Artist"), ExifTag::Artist),
    (tr_key("Copyright"), ExifTag::Copyright),
    (tr_key("Date taken"), ExifTag::DateTimeOriginal),
    (tr_key("Camera make"), ExifTag::Make),
    (tr_key("Camera model"), ExifTag::Model),
];
const DESCRIPTION: usize = 0;
const DATE: usize = 3;

pub type Values = [String; 6];

/// The EXIF shown and edited in the inspector.
#[derive(Debug)]
pub struct Dialog {
    pub path: PathBuf,
    pub original: Values,
    pub values: Values,
    /// Latitude and longitude in degrees (negative is south/west), if the file has a location.
    gps: Option<(f64, f64)>,
    pub remove_gps: bool,
    /// Camera, lens, exposure: read-only.
    summary: Vec<(&'static str, String)>,
    /// "300 dpi", from XResolution.
    pub resolution: Option<String>,
    /// Read-only listing of every other tag.
    others: Vec<String>,
}

/// little_exif can panic on input it doesn't expect; turn that into an error.
fn guarded<T>(f: impl FnOnce() -> Result<T, std::io::Error>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(_) => Err(tr("this file's EXIF couldn't be processed").into()),
    }
}

/// Parses the EXIF of a JPEG buffer. Refuses when the file has EXIF that little_exif reads
/// as empty, since writing that back would silently wipe it.
fn metadata(jpeg: &Vec<u8>) -> Result<Metadata, String> {
    let m = guarded(|| Metadata::new_from_vec(jpeg, FileExtension::JPEG))?;
    let has_exif = ImageReader::new(Cursor::new(jpeg))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_decoder().ok()?.exif_metadata().ok()?)
        .is_some_and(|e| !e.is_empty());
    if has_exif && (&m).into_iter().next().is_none() {
        return Err(tr("couldn't read this file's EXIF safely").into());
    }
    Ok(m)
}

fn string_value(m: &Metadata, tag: &ExifTag) -> String {
    m.get_tag(tag).next().map(|t| {
        let bytes = t.value_as_u8_vec(&m.get_endian());
        String::from_utf8_lossy(&bytes).trim_end_matches('\0').trim().to_string()
    }).unwrap_or_default()
}

fn rationals(m: &Metadata, tag: &ExifTag) -> Option<f64> {
    match m.get_tag(tag).next()? {
        ExifTag::ExposureTime(v) | ExifTag::FNumber(v) | ExifTag::FocalLength(v) | ExifTag::XResolution(v) => {
            v.first().filter(|r| r.denominator != 0).map(|r| r.nominator as f64 / r.denominator as f64)
        }
        _ => None,
    }
}

/// Camera, lens and exposure, for the rows that the file has.
fn summary(m: &Metadata) -> Vec<(&'static str, String)> {
    let make = string_value(m, &ExifTag::Make(String::new()));
    let model = string_value(m, &ExifTag::Model(String::new()));
    let camera = if model.to_lowercase().starts_with(&make.to_lowercase()) { model } else { format!("{make} {model}").trim().to_string() };
    let mut exposure = Vec::new();
    if let Some(t) = rationals(m, &ExifTag::ExposureTime(Vec::new())) {
        exposure.push(if t < 1.0 && t > 0.0 { format!("1/{} s", (1.0 / t).round()) } else { format!("{t} s") });
    }
    if let Some(f) = rationals(m, &ExifTag::FNumber(Vec::new())) {
        exposure.push(format!("f/{f:.1}"));
    }
    if let Some(ExifTag::ISO(v)) = m.get_tag(&ExifTag::ISO(Vec::new())).next()
        && let Some(iso) = v.first()
    {
        exposure.push(format!("ISO {iso}"));
    }
    if let Some(mm) = rationals(m, &ExifTag::FocalLength(Vec::new())) {
        exposure.push(format!("{mm:.1} mm"));
    }
    [(tr_key("Camera"), camera), (tr_key("Lens"), string_value(m, &ExifTag::LensModel(String::new()))), (tr_key("Exposure"), exposure.join(" · "))]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

fn resolution(m: &Metadata) -> Option<String> {
    let x = rationals(m, &ExifTag::XResolution(Vec::new()))?;
    let unit = match m.get_tag(&ExifTag::ResolutionUnit(Vec::new())).next() {
        Some(ExifTag::ResolutionUnit(v)) if v.first() == Some(&3) => "dpcm",
        _ => "dpi",
    };
    Some(format!("{} {unit}", x.round()))
}

/// "40.4168° N, 3.7038° W".
pub fn format_location((lat, lon): (f64, f64)) -> String {
    format!("{:.4}° {}, {:.4}° {}", lat.abs(), if lat < 0.0 { "S" } else { "N" }, lon.abs(), if lon < 0.0 { "W" } else { "E" })
}

fn location(m: &Metadata) -> Option<(f64, f64)> {
    let degrees = |tag: &ExifTag| -> Option<f64> {
        match m.get_tag(tag).next()? {
            ExifTag::GPSLatitude(v) | ExifTag::GPSLongitude(v) if v.len() == 3 => {
                let part = |i: usize| v[i].nominator as f64 / v[i].denominator.max(1) as f64;
                Some(part(0) + part(1) / 60.0 + part(2) / 3600.0)
            }
            _ => None,
        }
    };
    let lat = degrees(&ExifTag::GPSLatitude(Vec::new()))?;
    let lon = degrees(&ExifTag::GPSLongitude(Vec::new()))?;
    let south = string_value(m, &ExifTag::GPSLatitudeRef(String::new())) == "S";
    let west = string_value(m, &ExifTag::GPSLongitudeRef(String::new())) == "W";
    Some((if south { -lat } else { lat }, if west { -lon } else { lon }))
}

/// Reads `path` (a JPEG) for the inspector.
pub fn read(path: &Path) -> Result<Dialog, String> {
    let jpeg = std::fs::read(path).map_err(|e| e.to_string())?;
    let m = metadata(&jpeg)?;
    let original: Values = FIELDS.map(|(_, tag)| string_value(&m, &tag(String::new())));
    let key = |t: &ExifTag| (t.get_group(), t.as_u16());
    let shown: Vec<_> = FIELDS.iter().map(|(_, tag)| key(&tag(String::new()))).collect();
    let others = (&m)
        .into_iter()
        .filter(|t| !shown.contains(&key(t)))
        .map(|t| {
            let s = format!("{t:?}");
            if s.chars().count() > 90 { format!("{}…", s.chars().take(90).collect::<String>()) } else { s }
        })
        .collect();
    Ok(Dialog { path: path.to_path_buf(), values: original.clone(), original, gps: location(&m), remove_gps: false, summary: summary(&m), resolution: resolution(&m), others })
}

/// Applies the edits to a JPEG buffer. Only changed fields are touched; MakerNote is always
/// dropped. The result is checked before it's returned, so a bad write never reaches disk.
fn apply(jpeg: &mut Vec<u8>, original: &Values, values: &Values, remove_gps: bool) -> Result<(), String> {
    let mut m = metadata(jpeg)?;
    for (i, (_, tag)) in FIELDS.iter().enumerate() {
        let value = values[i].trim();
        if value == original[i] {
            continue;
        }
        if value.is_empty() {
            m.remove_tag(tag(String::new()));
        } else {
            m.set_tag(tag(value.to_string()));
        }
    }
    m.remove_tag(ExifTag::MakerNote(Vec::new()));
    if remove_gps {
        let gps: Vec<u16> = (&m).into_iter().filter(|t| t.get_group() == ExifTagGroup::GPS).map(|t| t.as_u16()).collect();
        for hex in gps {
            m.remove_tag_by_hex_group(hex, ExifTagGroup::GPS);
        }
        m.remove_tag(ExifTag::GPSInfo(Vec::new()));
    }
    guarded(|| m.write_to_vec(jpeg, FileExtension::JPEG))?;

    // Check the written file: values as asked, location gone if asked, image still decodes.
    let check = metadata(jpeg).map_err(|e| trf("the written file didn't check out: {e}", &[("e", &e)]))?;
    for (i, (label, tag)) in FIELDS.iter().enumerate() {
        if string_value(&check, &tag(String::new())) != values[i].trim() {
            return Err(trf("the written file didn't check out: {label} wasn't saved", &[("label", &tr(label))]));
        }
    }
    if remove_gps && (&check).into_iter().any(|t| t.get_group() == ExifTagGroup::GPS) {
        return Err(tr("the written file didn't check out: the location wasn't removed").into());
    }
    image::load_from_memory(jpeg).map_err(|e| trf("the written file didn't check out: {e}", &[("e", &e)]))?;
    Ok(())
}

/// Writes a copy of `source` with the edited EXIF to `dest`. The source is never modified.
pub fn write_copy(source: &Path, dest: &Path, original: &Values, values: &Values, remove_gps: bool) -> Result<(), String> {
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if ext != "jpg" && ext != "jpeg" {
        return Err(tr("the copy must be a .jpg file").into());
    }
    let dest_canonical = dest.parent().and_then(|d| std::fs::canonicalize(d).ok()).map(|d| d.join(dest.file_name().unwrap_or_default()));
    if dest_canonical == std::fs::canonicalize(source).ok() {
        return Err(tr("choose a new name: the original is never modified").into());
    }
    let mut jpeg = std::fs::read(source).map_err(|e| e.to_string())?;
    apply(&mut jpeg, original, values, remove_gps)?;
    write_atomic(dest, |out| out.write_all(&jpeg).map_err(|e| e.to_string()))
}

/// `YYYY:MM:DD HH:MM:SS` (the EXIF date format), or empty.
pub fn valid_date(s: &str) -> bool {
    let s = s.trim();
    s.is_empty()
        || s.len() == 19
            && s.bytes().enumerate().all(|(i, b)| match i {
                4 | 7 => b == b':',
                10 => b == b' ',
                13 | 16 => b == b':',
                _ => b.is_ascii_digit(),
            })
}

impl Dialog {
    fn changed(&self) -> bool {
        self.remove_gps || self.values.iter().map(|v| v.trim()).ne(self.original.iter().map(String::as_str))
    }
}

/// The inspector's EXIF, Location and Accessibility sections. Returns true when the user
/// asks to save a copy with the edits.
pub fn sections_ui(ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    egui::CollapsingHeader::new("EXIF").default_open(true).show(ui, |ui| {
        egui::Grid::new("exif-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            for (label, value) in &d.summary {
                ui.label(tr(label));
                ui.add(egui::Label::new(value).wrap());
                ui.end_row();
            }
            for (i, (label, _)) in FIELDS.iter().enumerate().filter(|(i, _)| *i != DESCRIPTION) {
                ui.label(tr(label));
                ui.vertical(|ui| {
                    let hint = if i == DATE { "YYYY:MM:DD HH:MM:SS" } else { "" };
                    ui.add(egui::TextEdit::singleline(&mut d.values[i]).hint_text(hint).desired_width(f32::INFINITY));
                    if i == DATE && !valid_date(&d.values[i]) {
                        ui.colored_label(ui.visuals().error_fg_color, tr("Use YYYY:MM:DD HH:MM:SS"));
                    }
                });
                ui.end_row();
            }
        });
        egui::CollapsingHeader::new(trf("Other tags ({count})", &[("count", &d.others.len())])).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                for tag in &d.others {
                    ui.monospace(tag);
                }
            });
        });
    });
    egui::CollapsingHeader::new(tr("Location")).default_open(true).show(ui, |ui| match d.gps {
        Some((lat, lon)) => {
            ui.label(format_location((lat, lon)));
            ui.horizontal_wrapped(|ui| {
                // shortcut: xdg-open is Linux only; use the platform's opener on Windows.
                let open = |url: String| drop(std::process::Command::new("xdg-open").arg(url).spawn());
                if ui.button(tr("Open in Google Maps")).clicked() {
                    open(format!("https://www.google.com/maps?q={lat},{lon}"));
                }
                if ui.button("OpenStreetMap").clicked() {
                    open(format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=16/{lat}/{lon}"));
                }
            });
            ui.checkbox(&mut d.remove_gps, tr("Remove location from the copy"));
        }
        None => {
            ui.weak(tr("No location in this file"));
        }
    });
    egui::CollapsingHeader::new(tr("Accessibility")).default_open(true).show(ui, |ui| {
        ui.label(tr("Image description"));
        ui.add(egui::TextEdit::multiline(&mut d.values[DESCRIPTION]).hint_text(tr("Describe the image for people who can't see it")).desired_rows(3).desired_width(f32::INFINITY));
    });
    ui.add_space(6.0);
    let changed = d.changed();
    let save = ui.horizontal_wrapped(|ui| {
        let save = ui.add_enabled(changed && valid_date(&d.values[DATE]), egui::Button::new(tr("Save copy…"))).clicked();
        if ui.add_enabled(changed, egui::Button::new(tr("Revert"))).clicked() {
            (d.values, d.remove_gps) = (d.original.clone(), false);
        }
        save
    }).inner;
    ui.weak(tr("Saves a copy of the file with this metadata (without other edits). Camera maker notes are removed."));
    save
}

#[cfg(test)]
mod tests {
    use super::*;
    use little_exif::rational::uR64;

    #[test]
    fn dates() {
        assert!(valid_date("2025:05:30 19:26:02"));
        assert!(valid_date(""));
        assert!(!valid_date("2025-05-30 19:26:02"));
        assert!(!valid_date("2025:05:30"));
        assert!(!valid_date("2025:05:30 19:26:0x"));
    }

    /// A small JPEG with Make, Artist, a location and a MakerNote.
    fn sample_jpeg() -> Vec<u8> {
        let mut jpeg = Vec::new();
        let pixels: Vec<u8> = (0..16 * 8 * 3).map(|i| (i * 7) as u8).collect();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode(&pixels, 16, 8, image::ExtendedColorType::Rgb8).unwrap();
        let mut m = Metadata::new();
        m.set_tag(ExifTag::Make("Acme".into()));
        m.set_tag(ExifTag::Artist("Ana".into()));
        m.set_tag(ExifTag::DateTimeOriginal("2020:01:02 03:04:05".into()));
        m.set_tag(ExifTag::GPSLatitudeRef("N".into()));
        m.set_tag(ExifTag::GPSLatitude(vec![uR64 { nominator: 40, denominator: 1 }, uR64 { nominator: 25, denominator: 1 }, uR64 { nominator: 0, denominator: 1 }]));
        m.set_tag(ExifTag::GPSLongitudeRef("W".into()));
        m.set_tag(ExifTag::GPSLongitude(vec![uR64 { nominator: 3, denominator: 1 }, uR64 { nominator: 42, denominator: 1 }, uR64 { nominator: 0, denominator: 1 }]));
        m.set_tag(ExifTag::MakerNote(vec![1, 2, 3, 4]));
        m.write_to_vec(&mut jpeg, FileExtension::JPEG).unwrap();
        jpeg
    }

    fn read_vec(jpeg: &Vec<u8>, name: &str) -> Dialog {
        let path = std::env::temp_dir().join(format!("lightview-exif-{name}-{}.jpg", std::process::id()));
        std::fs::write(&path, jpeg).unwrap();
        let d = read(&path);
        std::fs::remove_file(&path).unwrap();
        d.unwrap()
    }

    #[test]
    fn reads_fields_and_location() {
        let d = read_vec(&sample_jpeg(), "reads");
        assert_eq!(d.original[1], "Ana");
        assert_eq!(d.original[3], "2020:01:02 03:04:05");
        assert_eq!(d.original[4], "Acme");
        assert_eq!(d.gps.map(format_location).as_deref(), Some("40.4167° N, 3.7000° W"));
        assert!(d.gps.unwrap().1 < 0.0, "west is negative");
        assert_eq!(d.summary, [("Camera", "Acme".to_string())]);
    }

    // Content wider than a right panel moves the panel's left edge, past the expander arrows.
    #[test]
    fn sections_keep_the_panel_width() {
        fn image_area_right(d: &mut Dialog, width: f32) -> f32 {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::markup::font_definitions());
            let mut right = 0.0;
            for _ in 0..4 {
                let mut input = egui::RawInput::default();
                input.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0)));
                let mut output = ctx.run_ui(input, |ui| {
                    egui::Panel::right("inspector").size_range(240.0..=320.0).default_size(width).resizable(true).show(ui, |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| sections_ui(ui, d));
                    });
                    right = ui.available_rect_before_wrap().right();
                });
                output.textures_delta.clear();
            }
            right
        }
        let mut d = read_vec(&sample_jpeg(), "panel");
        d.summary = vec![
            ("Lens", "EF24-70mm f/2.8L II USM with a rather long lens description".to_string()),
            ("Exposure", "1/250 s · f/2.8 · ISO 100 · 24.0 mm".to_string()),
        ];
        for width in [240.0, 320.0] {
            assert_eq!(image_area_right(&mut d, width), 1280.0 - width);
        }
    }

    #[test]
    fn apply_edits() {
        let before = sample_jpeg();
        let d = read_vec(&before, "apply");
        let mut values = d.original.clone();
        values[0] = "A walk".into(); // set
        values[1] = String::new(); // clear Artist
        values[3] = "2021:02:03 04:05:06".into(); // change date

        for remove_gps in [false, true] {
            let mut jpeg = before.clone();
            apply(&mut jpeg, &d.original, &values, remove_gps).unwrap();
            let m = Metadata::new_from_vec(&jpeg, FileExtension::JPEG).unwrap();
            assert_eq!(string_value(&m, &ExifTag::ImageDescription(String::new())), "A walk");
            assert!(m.get_tag(&ExifTag::Artist(String::new())).next().is_none());
            assert_eq!(string_value(&m, &ExifTag::DateTimeOriginal(String::new())), "2021:02:03 04:05:06");
            assert_eq!(string_value(&m, &ExifTag::Make(String::new())), "Acme", "untouched tag kept");
            assert!(m.get_tag(&ExifTag::MakerNote(Vec::new())).next().is_none(), "MakerNote dropped");
            assert_eq!(location(&m).is_some(), !remove_gps);
            // Pixels are never re-encoded.
            assert_eq!(image::load_from_memory(&jpeg).unwrap().to_rgb8(), image::load_from_memory(&before).unwrap().to_rgb8());
        }
    }

    #[test]
    fn write_copy_never_touches_source() {
        let dir = std::env::temp_dir().join(format!("lightview-exif-copy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("photo.jpg");
        std::fs::write(&source, sample_jpeg()).unwrap();
        let original = std::fs::read(&source).unwrap();
        let d = read(&source).unwrap();
        let mut values = d.original.clone();
        values[2] = "© Ana".into();

        assert!(write_copy(&source, &source, &d.original, &values, true).is_err());
        assert!(write_copy(&source, &dir.join("./photo.jpg"), &d.original, &values, true).is_err());
        assert!(write_copy(&source, &dir.join("copy.png"), &d.original, &values, true).is_err());
        write_copy(&source, &dir.join("copy.jpg"), &d.original, &values, true).unwrap();
        let copy = read(&dir.join("copy.jpg")).unwrap();
        let source_after = std::fs::read(&source).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(source_after, original);
        assert_eq!(copy.original[2], "© Ana");
        assert!(copy.gps.is_none());
    }
}
