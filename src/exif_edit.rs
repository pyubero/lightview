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
    ("Description", ExifTag::ImageDescription),
    ("Artist", ExifTag::Artist),
    ("Copyright", ExifTag::Copyright),
    ("Date taken", ExifTag::DateTimeOriginal),
    ("Camera make", ExifTag::Make),
    ("Camera model", ExifTag::Model),
];
const DATE: usize = 3;

pub type Values = [String; 6];

/// The Edit EXIF dialog's state.
pub struct Dialog {
    pub path: PathBuf,
    pub original: Values,
    pub values: Values,
    /// "40.4168° N, 3.7038° W", if the file has a location.
    gps: Option<String>,
    pub remove_gps: bool,
    /// Read-only listing of every other tag.
    others: Vec<String>,
}

pub enum Choice {
    Save,
    Cancel,
}

/// little_exif can panic on input it doesn't expect; turn that into an error.
fn guarded<T>(f: impl FnOnce() -> Result<T, std::io::Error>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(_) => Err("this file's EXIF couldn't be processed".into()),
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
        return Err("couldn't read this file's EXIF safely".into());
    }
    Ok(m)
}

fn string_value(m: &Metadata, tag: &ExifTag) -> String {
    m.get_tag(tag).next().map(|t| {
        let bytes = t.value_as_u8_vec(&m.get_endian());
        String::from_utf8_lossy(&bytes).trim_end_matches('\0').trim().to_string()
    }).unwrap_or_default()
}

fn location(m: &Metadata) -> Option<String> {
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
    let lat_ref = string_value(m, &ExifTag::GPSLatitudeRef(String::new()));
    let lon_ref = string_value(m, &ExifTag::GPSLongitudeRef(String::new()));
    Some(format!("{lat:.4}° {lat_ref}, {lon:.4}° {lon_ref}"))
}

/// Reads `path` (a JPEG) for the dialog.
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
    Ok(Dialog { path: path.to_path_buf(), values: original.clone(), original, gps: location(&m), remove_gps: false, others })
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
    let check = metadata(jpeg).map_err(|e| format!("the written file didn't check out: {e}"))?;
    for (i, (label, tag)) in FIELDS.iter().enumerate() {
        if string_value(&check, &tag(String::new())) != values[i].trim() {
            return Err(format!("the written file didn't check out: {label} wasn't saved"));
        }
    }
    if remove_gps && (&check).into_iter().any(|t| t.get_group() == ExifTagGroup::GPS) {
        return Err("the written file didn't check out: the location wasn't removed".into());
    }
    image::load_from_memory(jpeg).map_err(|e| format!("the written file didn't check out: {e}"))?;
    Ok(())
}

/// Writes a copy of `source` with the edited EXIF to `dest`. The source is never modified.
pub fn write_copy(source: &Path, dest: &Path, original: &Values, values: &Values, remove_gps: bool) -> Result<(), String> {
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if ext != "jpg" && ext != "jpeg" {
        return Err("the copy must be a .jpg file".into());
    }
    let dest_canonical = dest.parent().and_then(|d| std::fs::canonicalize(d).ok()).map(|d| d.join(dest.file_name().unwrap_or_default()));
    if dest_canonical == std::fs::canonicalize(source).ok() {
        return Err("choose a new name: the original is never modified".into());
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

/// The dialog body. Returns what the user chose, if anything.
pub fn ui(ui: &mut egui::Ui, d: &mut Dialog) -> Option<Choice> {
    let name = d.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    ui.heading(format!("Edit EXIF — {name}"));
    ui.add_space(4.0);
    egui::Grid::new("exif-grid").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
        for (i, (label, _)) in FIELDS.iter().enumerate() {
            ui.label(*label);
            ui.vertical(|ui| {
                let hint = if i == DATE { "YYYY:MM:DD HH:MM:SS" } else { "" };
                ui.add(egui::TextEdit::singleline(&mut d.values[i]).hint_text(hint).desired_width(260.0));
                if i == DATE && !valid_date(&d.values[i]) {
                    ui.colored_label(ui.visuals().error_fg_color, "Use YYYY:MM:DD HH:MM:SS");
                }
            });
            ui.end_row();
        }
        ui.label("Location");
        match &d.gps {
            Some(gps) => {
                ui.horizontal(|ui| {
                    ui.label(gps);
                    ui.checkbox(&mut d.remove_gps, "Remove location");
                });
            }
            None => {
                ui.weak("None");
            }
        }
        ui.end_row();
    });
    egui::CollapsingHeader::new(format!("Other tags ({})", d.others.len())).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            for tag in &d.others {
                ui.monospace(tag);
            }
        });
    });
    ui.weak("Camera maker notes are removed in the saved copy.");
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let save = ui.add_enabled(valid_date(&d.values[DATE]), egui::Button::new("Save copy…")).clicked();
        let cancel = ui.button("Cancel").clicked();
        if save { Some(Choice::Save) } else if cancel { Some(Choice::Cancel) } else { None }
    }).inner
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
        assert_eq!(d.gps.as_deref(), Some("40.4167° N, 3.7000° W"));
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
