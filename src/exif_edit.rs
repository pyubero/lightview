use crate::i18n::{tr, tr_key, trf};
use crate::load::ifd_chain_ends;
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
    /// Why this file's EXIF is shown but not edited, if it isn't.
    read_only: Option<&'static str>,
}

/// The file types whose EXIF is shown, by extension. RAW files are read through `raw` instead.
pub fn kind(path: &Path) -> Option<FileExtension> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some(FileExtension::JPEG),
        // The flag only changes `as_u8_vec`: writing always makes a zTXt chunk, which `standard_png_exif` replaces.
        "png" => Some(FileExtension::PNG { as_zTXt_chunk: false }),
        "webp" => Some(FileExtension::WEBP),
        "tif" | "tiff" => Some(FileExtension::TIFF),
        _ => None,
    }
}

/// little_exif can panic on input it doesn't expect; turn that into an error.
fn guarded<T>(f: impl FnOnce() -> Result<T, std::io::Error>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(_) => Err(tr("this file's EXIF couldn't be processed").into()),
    }
}

/// Whether every TIFF block in the file has an IFD chain that ends. little_exif follows a looping
/// chain for ever, which `guarded` can't stop, and it finds its block by scanning, with or without
/// the `Exif` prefix and even behind the image data, so all of them are checked.
fn ifd_chains_end(jpeg: &[u8]) -> bool {
    jpeg.windows(4).enumerate().all(|(i, w)| !matches!(w, b"II\x2a\0" | b"MM\0\x2a") || ifd_chain_ends(&jpeg[i..]))
}

/// Parses the EXIF of a file of type `kind`. Refuses when the file has EXIF that little_exif reads
/// as empty, since writing that back would silently wipe it.
fn metadata(jpeg: &Vec<u8>, kind: FileExtension) -> Result<Metadata, String> {
    if !ifd_chains_end(jpeg) {
        return Err(tr("this file's EXIF couldn't be processed").into());
    }
    let m = guarded(|| Metadata::new_from_vec(jpeg, kind))?;
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

/// `value`'s debug text, cut at `max` characters without writing the rest: a tag can hold
/// megabytes (a RAW file's maker notes), and formatting them all took a second.
fn shortened(value: &impl std::fmt::Debug, max: usize) -> String {
    struct Upto(String, usize);
    impl std::fmt::Write for Upto {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for c in s.chars() {
                if self.1 == 0 {
                    return Err(std::fmt::Error);
                }
                self.0.push(c);
                self.1 -= 1;
            }
            Ok(())
        }
    }
    let mut out = Upto(String::new(), max);
    match std::fmt::write(&mut out, format_args!("{value:?}")) {
        Ok(()) => out.0,
        Err(_) => out.0 + "…",
    }
}

/// A RAW file's EXIF: its preview's own when it has some (RAF, RW2, iPhone DNG), else the camera's
/// TIFF structure (CR2, NEF, ARW, DNG, ORF), or CR3's two TIFF blocks.
fn raw_metadata(all: &[u8]) -> Result<Metadata, String> {
    if crate::raw::is_cr3(all) {
        return cr3_metadata(all);
    }
    if let Some(head) = crate::raw::embedded_jpeg(all).and_then(crate::raw::exif_header)
        && let Ok(m) = metadata(&head, FileExtension::JPEG)
        && (&m).into_iter().next().is_some()
    {
        return Ok(m);
    }
    let mut tiff = all.to_vec();
    // Olympus changes only the magic number, which little_exif checks.
    match tiff.get(..4) {
        Some(b"IIRO") => tiff[2..4].copy_from_slice(&[42, 0]),
        Some(b"MMOR") => tiff[2..4].copy_from_slice(&[0, 42]),
        _ => {}
    }
    if !ifd_chain_ends(&tiff) {
        return Err(tr("this file's EXIF couldn't be processed").into());
    }
    guarded(|| Metadata::new_from_vec(&tiff, FileExtension::TIFF)).map_err(|_| tr("EXIF isn't read from this kind of RAW file yet").into())
}

/// CR3's `CMT1` (IFD0), with `CMT2`'s Exif tags added.
fn cr3_metadata(all: &[u8]) -> Result<Metadata, String> {
    let parse = |tiff: Vec<u8>| -> Result<Metadata, String> {
        if !ifd_chain_ends(&tiff) {
            return Err(tr("this file's EXIF couldn't be processed").into());
        }
        guarded(|| Metadata::new_from_vec(&tiff, FileExtension::TIFF))
    };
    let mut m = parse(crate::raw::cr3_box(all, b"CMT1").ok_or(tr("this file's EXIF couldn't be processed"))?.to_vec())?;
    if let Some(exif) = crate::raw::cr3_box(all, b"CMT2").and_then(as_exif_ifd)
        && let Ok(e) = parse(exif)
    {
        for tag in &e {
            if tag.get_group() == ExifTagGroup::EXIF {
                m.set_tag(tag.clone());
            }
        }
    }
    Ok(m)
}

/// `tiff` with a new IFD0 whose only entry points at the old one as the Exif IFD: CMT2 holds Exif
/// tags at the top level, where a reader doesn't know them. Nothing moves, so offsets stay valid.
fn as_exif_ifd(tiff: &[u8]) -> Option<Vec<u8>> {
    let (_, u32_at) = crate::load::tiff_readers(tiff)?;
    let (exif_ifd, le) = (u32::try_from(u32_at(4)?).ok()?, tiff[0] == b'I');
    let u16_bytes = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let u32_bytes = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let mut out = tiff.to_vec();
    if out.len() % 2 == 1 {
        out.push(0);
    }
    let ifd0 = u32::try_from(out.len()).ok()?;
    out.extend(u16_bytes(1));
    // Tag 0x8769 (Exif IFD), type 4 (LONG), one value.
    out.extend(u16_bytes(0x8769));
    out.extend(u16_bytes(4));
    out.extend(u32_bytes(1));
    out.extend(u32_bytes(exif_ifd));
    out.extend(u32_bytes(0));
    out[4..8].copy_from_slice(&u32_bytes(ifd0));
    Some(out)
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

/// Reads `path` for the inspector: a file with a `kind`, or a RAW file.
pub fn read(path: &Path) -> Result<Dialog, String> {
    read_bytes(path, &std::fs::read(path).map_err(|e| e.to_string())?)
}

/// `read`, for a file already in memory.
pub fn read_bytes(path: &Path, bytes: &[u8]) -> Result<Dialog, String> {
    let raw = crate::raw::is_raw(path);
    let kind = if raw { FileExtension::TIFF } else { self::kind(path).ok_or(tr("EXIF is shown for JPEG, PNG, WebP, TIFF and RAW files"))? };
    let m = if raw { raw_metadata(bytes)? } else { metadata(&bytes.to_vec(), kind)? };
    let read_only = if raw {
        Some(tr_key("A RAW file's metadata is shown, not edited."))
    } else if kind == FileExtension::WEBP {
        // little_exif 0.6.23 refuses lossy WebP and corrupts the image data of the rest.
        Some(tr_key("A WebP file's metadata is shown, not edited."))
    } else {
        None
    };
    let original: Values = FIELDS.map(|(_, tag)| string_value(&m, &tag(String::new())));
    let key = |t: &ExifTag| (t.get_group(), t.as_u16());
    let shown: Vec<_> = FIELDS.iter().map(|(_, tag)| key(&tag(String::new()))).collect();
    let others = (&m)
        .into_iter()
        .filter(|t| !shown.contains(&key(t)))
        .map(|t| shortened(&t, 90))
        .collect();
    Ok(Dialog { path: path.to_path_buf(), values: original.clone(), original, gps: location(&m), remove_gps: false, summary: summary(&m), resolution: resolution(&m), others, read_only })
}

/// Drops the XMP that mentions a location: little_exif only rewrites the EXIF.
fn drop_xmp_location(bytes: &mut Vec<u8>, kind: FileExtension) {
    match kind {
        FileExtension::JPEG => drop_jpeg_xmp_location(bytes),
        FileExtension::PNG { .. } => drop_png_xmp_location(bytes),
        // TIFF keeps XMP in tag 0x02BC, which `apply` removes.
        _ => {}
    }
}

/// The PNG's chunks: where each starts, its type and its data. Stops at anything malformed.
fn png_chunks(png: &[u8]) -> Vec<(usize, [u8; 4], &[u8])> {
    let mut chunks = Vec::new();
    let mut at = 8;
    while let Some(head) = png.get(at..at + 8) {
        let len = u32::from_be_bytes(head[..4].try_into().unwrap()) as usize;
        let Some(data) = png.get(at + 8..).and_then(|rest| rest.get(..len)) else { break };
        if png.len() < at + 12 + len {
            break;
        }
        chunks.push((at, head[4..8].try_into().unwrap(), data));
        at += 12 + len;
    }
    chunks
}

/// Drops `iTXt` XMP that names GPS, or is compressed and so can't be checked.
fn drop_png_xmp_location(png: &mut Vec<u8>) {
    const XMP: &[u8] = b"XML:com.adobe.xmp\0";
    let doomed: Vec<(usize, usize)> = png_chunks(png)
        .into_iter()
        .filter(|(_, kind, data)| kind == b"iTXt" && data.starts_with(XMP) && (data.get(XMP.len()) != Some(&0) || data.windows(3).any(|w| w == b"GPS")))
        .map(|(at, _, data)| (at, at + 12 + data.len()))
        .collect();
    for (start, end) in doomed.into_iter().rev() {
        png.drain(start..end);
    }
}

/// Replaces the zTXt "Raw profile type exif" chunk that little_exif writes with a standard
/// `eXIf` chunk holding `m`: the `image` crate, browsers and Windows read only that one.
fn standard_png_exif(png: &mut Vec<u8>, m: &Metadata) -> Result<(), String> {
    let mut tiff = m.encode().map_err(|e| e.to_string())?;
    if tiff.starts_with(b"Exif\0\0") {
        tiff.drain(..6);
    }
    let chunks = png_chunks(png);
    let legacy = chunks.iter().find(|(_, kind, data)| kind == b"zTXt" && data.starts_with(b"Raw profile type exif\0")).map(|&(at, _, data)| (at, at + 12 + data.len()));
    let Some((start, end)) = legacy else { return Err(tr("this file's EXIF couldn't be processed").into()) };
    let mut chunk = (tiff.len() as u32).to_be_bytes().to_vec();
    chunk.extend(b"eXIf");
    chunk.extend(&tiff);
    chunk.extend(crc32fast::hash(&chunk[4..]).to_be_bytes());
    png.splice(start..end, chunk);
    Ok(())
}

/// Drops the XMP segments that mention a location.
fn drop_jpeg_xmp_location(jpeg: &mut Vec<u8>) {
    const XMP: [&[u8]; 2] = [b"http://ns.adobe.com/xap/1.0/\0", b"http://ns.adobe.com/xmp/extension/\0"];
    let mut at = 2;
    while let Some(&[0xFF, marker, high, low]) = jpeg.get(at..).and_then(|rest| rest.first_chunk::<4>()) {
        let end = at + 2 + u16::from_be_bytes([high, low]) as usize;
        let Some(body) = jpeg.get(at + 4..end).filter(|_| marker != 0xDA) else { break };
        if marker == 0xE1 && XMP.iter().any(|x| body.starts_with(x)) && body.windows(3).any(|w| w == b"GPS") {
            jpeg.drain(at..end);
        } else {
            at = end;
        }
    }
}

/// Applies the edits to a file of type `kind`. Only changed fields are touched; MakerNote is always
/// dropped. The result is checked before it's returned, so a bad write never reaches disk.
fn apply(jpeg: &mut Vec<u8>, kind: FileExtension, original: &Values, values: &Values, remove_gps: bool) -> Result<(), String> {
    if kind == FileExtension::WEBP {
        return Err(tr("A WebP file's metadata is shown, not edited.").into());
    }
    let pixels = image::load_from_memory(jpeg).map_err(|e| e.to_string())?;
    let mut m = metadata(jpeg, kind)?;
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
        // A TIFF's XMP packet.
        if m.get_tag_by_hex(0x02BC, Some(ExifTagGroup::GENERIC)).any(|t| t.value_as_u8_vec(&m.get_endian()).windows(3).any(|w| w == b"GPS")) {
            m.remove_tag_by_hex_group(0x02BC, ExifTagGroup::GENERIC);
        }
    }
    guarded(|| m.write_to_vec(jpeg, kind))?;
    if matches!(kind, FileExtension::PNG { .. }) {
        standard_png_exif(jpeg, &m)?;
    }
    if remove_gps {
        drop_xmp_location(jpeg, kind);
    }

    // Check the written file: values as asked, location gone if asked, image still decodes.
    let check = metadata(jpeg, kind).map_err(|e| trf("the written file didn't check out: {e}", &[("e", &e)]))?;
    for (i, (label, tag)) in FIELDS.iter().enumerate() {
        if string_value(&check, &tag(String::new())) != values[i].trim() {
            return Err(trf("the written file didn't check out: {label} wasn't saved", &[("label", &tr(label))]));
        }
    }
    if remove_gps && (&check).into_iter().any(|t| t.get_group() == ExifTagGroup::GPS) {
        return Err(tr("the written file didn't check out: the location wasn't removed").into());
    }
    // TIFF's EXIF is the file's own structure, which little_exif writes over.
    if image::load_from_memory(jpeg).map_err(|e| trf("the written file didn't check out: {e}", &[("e", &e)]))? != pixels {
        return Err(tr("the written file didn't check out: the image changed").into());
    }
    Ok(())
}

/// Writes a copy of `source` with the edited EXIF to `dest`. The source is never modified.
pub fn write_copy(source: &Path, dest: &Path, original: &Values, values: &Values, remove_gps: bool) -> Result<(), String> {
    let kind = self::kind(source).ok_or(tr("EXIF is shown for JPEG, PNG, WebP, TIFF and RAW files"))?;
    if self::kind(dest) != Some(kind) {
        let ext = source.extension().unwrap_or_default().to_string_lossy().to_lowercase();
        return Err(trf("the copy must be a .{ext} file", &[("ext", &ext)]));
    }
    let dest_canonical = dest.parent().and_then(|d| std::fs::canonicalize(d).ok()).map(|d| d.join(dest.file_name().unwrap_or_default()));
    if dest_canonical == std::fs::canonicalize(source).ok() {
        return Err(tr("choose a new name: the original is never modified").into());
    }
    let mut jpeg = std::fs::read(source).map_err(|e| e.to_string())?;
    apply(&mut jpeg, kind, original, values, remove_gps)?;
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
                    ui.add(egui::TextEdit::singleline(&mut d.values[i]).hint_text(hint).desired_width(f32::INFINITY).interactive(d.read_only.is_none()));
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
                let open = |url: String| crate::platform::open(url);
                if ui.button(tr("Open in Google Maps")).clicked() {
                    open(format!("https://www.google.com/maps?q={lat},{lon}"));
                }
                if ui.button("OpenStreetMap").clicked() {
                    open(format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=16/{lat}/{lon}"));
                }
            });
            if d.read_only.is_none() {
                ui.checkbox(&mut d.remove_gps, tr("Remove location from the copy"));
            }
        }
        None => {
            ui.weak(tr("No location in this file"));
        }
    });
    egui::CollapsingHeader::new(tr("Accessibility")).default_open(true).show(ui, |ui| {
        ui.label(tr("Image description"));
        ui.add(egui::TextEdit::multiline(&mut d.values[DESCRIPTION]).hint_text(tr("Describe the image for people who can't see it")).desired_rows(3).desired_width(f32::INFINITY).interactive(d.read_only.is_none()));
    });
    ui.add_space(6.0);
    if let Some(why) = d.read_only {
        ui.weak(tr(why));
        return false;
    }
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

    /// A CR3-like file: CMT1 names the camera, CMT2 holds the exposure time at its top level.
    #[test]
    fn a_cr3s_exif_comes_from_both_blocks() {
        let mut cmt1 = b"II*\0\x08\0\0\0\x01\0".to_vec();
        cmt1.extend([0x0F, 0x01, 2, 0, 6, 0, 0, 0, 26, 0, 0, 0, 0, 0, 0, 0]);
        cmt1.extend(b"Canon\0");
        let mut cmt2 = b"II*\0\x08\0\0\0\x01\0".to_vec();
        cmt2.extend([0x9A, 0x82, 5, 0, 1, 0, 0, 0, 26, 0, 0, 0, 0, 0, 0, 0]);
        cmt2.extend([1, 0, 0, 0, 125, 0, 0, 0]);
        let mut cr3 = b"\0\0\0\x18ftypcrx \0\0\0\x01crx isom".to_vec();
        for (name, block) in [(b"CMT1", &cmt1), (b"CMT2", &cmt2)] {
            cr3.extend(((block.len() + 8) as u32).to_be_bytes());
            cr3.extend(name);
            cr3.extend(block);
        }
        let m = raw_metadata(&cr3).unwrap();
        assert_eq!(string_value(&m, &ExifTag::Make(String::new())), "Canon");
        assert_eq!(rationals(&m, &ExifTag::ExposureTime(Vec::new())), Some(1.0 / 125.0));
        // Without the wrapping, the exposure would be an unknown tag.
        assert!(as_exif_ifd(b"II*\0").is_none());
    }

    #[test]
    fn long_tags_are_cut_without_being_written_out() {
        assert_eq!(shortened(&"abc", 90), "\"abc\"");
        assert_eq!(shortened(&vec![7u8; 10_000_000], 12), "[7, 7, 7, 7,…");
    }

    #[test]
    fn a_raw_files_exif_is_shown_read_only() {
        use image::ImageEncoder;
        let dir = std::env::temp_dir().join(format!("lightview-rawexif-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A RAF-like file whose preview carries EXIF with a camera model.
        let mut exif = crate::raw::tests::tiff(b"II*\0", 1, &[]);
        exif[8..10].copy_from_slice(&1u16.to_le_bytes());
        let mut preview = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new(&mut preview);
        enc.set_exif_metadata(exif).unwrap();
        enc.write_image(&[90; 8 * 8 * 3], 8, 8, image::ExtendedColorType::Rgb8).unwrap();
        let mut raf = b"FUJIFILMCCD-RAW 0201FF383501".to_vec();
        raf.extend(&preview);
        let path = dir.join("a.raf");
        std::fs::write(&path, &raf).unwrap();
        let d = read(&path).unwrap();
        assert!(d.read_only.is_some());
        // Its sidebar never offers a copy.
        let ctx = egui::Context::default();
        crate::markup::font_definitions();
        ctx.set_fonts(crate::markup::font_definitions());
        let mut d = d;
        d.values[0] = "changed".into();
        let mut save = true;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| save = sections_ui(ui, &mut d));
        output.textures_delta.clear();
        assert!(!save);
        // An Olympus file is a TIFF with its own magic number.
        let orf = crate::raw::tests::tiff(b"IIRO", 1, &[&crate::raw::tests::jpeg(8, 8)]);
        std::fs::write(dir.join("b.orf"), orf).unwrap();
        assert!(read(&dir.join("b.orf")).is_ok_and(|d| d.read_only.is_some()));
        std::fs::remove_dir_all(dir).unwrap();
    }
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
        fn image_area_right(d: &mut Dialog, width: f32, font_size: f32) -> f32 {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::markup::font_definitions());
            crate::settings::apply_font_size(&ctx, font_size);
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
            for font_size in [10.0, 13.0, 20.0] {
                assert_eq!(image_area_right(&mut d, width, font_size), 1280.0 - width, "{width} wide, font {font_size}");
            }
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
            apply(&mut jpeg, FileExtension::JPEG, &d.original, &values, remove_gps).unwrap();
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

    /// `jpeg` with a segment after its EXIF one: little_exif expects EXIF to come first.
    fn with_segment_after_exif(mut jpeg: Vec<u8>, payload: &[u8]) -> Vec<u8> {
        let exif = jpeg.windows(6).position(|w| w == b"Exif\0\0").unwrap();
        let end = exif - 2 + u16::from_be_bytes([jpeg[exif - 2], jpeg[exif - 1]]) as usize;
        let tail = jpeg.split_off(end);
        jpeg.extend([0xFF, 0xE1]);
        jpeg.extend((payload.len() as u16 + 2).to_be_bytes());
        jpeg.extend(payload);
        jpeg.extend(tail);
        jpeg
    }

    /// A JPEG with a segment right after SOI: `marker`, then `payload`.
    fn with_segment(mut jpeg: Vec<u8>, marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut segment = vec![0xFF, marker];
        segment.extend((payload.len() as u16 + 2).to_be_bytes());
        segment.extend(payload);
        jpeg.splice(2..2, segment);
        jpeg
    }

    /// Without a guard little_exif walks this IFD for ever, on the UI thread.
    #[test]
    fn looping_ifds_are_refused() {
        let plain = {
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode(&[0; 3], 1, 1, image::ExtendedColorType::Rgb8).unwrap();
            jpeg
        };
        const LOOPING: &[u8] = b"II\x2a\0\x08\0\0\0\0\0\x08\0\0\0";
        let with_prefix = [b"Exif\0\0".as_slice(), LOOPING].concat();
        assert!(metadata(&with_segment(plain.clone(), 0xE1, &with_prefix), FileExtension::JPEG).is_err());
        // little_exif doesn't need the prefix.
        assert!(metadata(&with_segment(plain.clone(), 0xE1, LOOPING), FileExtension::JPEG).is_err());
        // Behind another segment, and behind the image data.
        assert!(metadata(&with_segment(with_segment(plain.clone(), 0xE1, &with_prefix), 0xFE, b"comment"), FileExtension::JPEG).is_err());
        let mut after_image = plain.clone();
        after_image.truncate(after_image.len() - 2);
        let tail = with_segment(vec![0xFF, 0xD8], 0xE1, &with_prefix);
        after_image.extend(&tail[2..]);
        assert!(metadata(&after_image, FileExtension::JPEG).is_err());
    }

    #[test]
    fn remove_gps_also_clears_xmp() {
        let xmp = |text: &str| [b"http://ns.adobe.com/xap/1.0/\0".as_slice(), text.as_bytes()].concat();
        let jpeg = with_segment_after_exif(sample_jpeg(), &xmp(r#"<rdf:Description exif:GPSLatitude="40,25N"/>"#));
        let kept = with_segment_after_exif(sample_jpeg(), &xmp("<dc:title>Walk</dc:title>"));
        let d = read_vec(&sample_jpeg(), "xmp");
        let (mut a, mut b, mut c) = (jpeg.clone(), jpeg, kept);
        apply(&mut a, FileExtension::JPEG, &d.original, &d.original, true).unwrap();
        apply(&mut b, FileExtension::JPEG, &d.original, &d.original, false).unwrap();
        apply(&mut c, FileExtension::JPEG, &d.original, &d.original, true).unwrap();
        let has = |jpeg: &[u8], text: &[u8]| jpeg.windows(text.len()).any(|w| w == text);
        assert!(!has(&a, b"GPSLatitude"), "the location survived in XMP");
        assert!(has(&b, b"GPSLatitude"), "only removed on request");
        assert!(has(&c, b"Walk"), "XMP without a location stays");
    }

    /// `sample_jpeg`'s tags on a 16×8 image in `format`.
    fn sample(format: image::ImageFormat) -> Vec<u8> {
        let pixels: Vec<u8> = (0..16 * 8 * 3).map(|i| (i * 7) as u8).collect();
        let mut bytes = Vec::new();
        image::RgbImage::from_raw(16, 8, pixels).unwrap().write_to(&mut Cursor::new(&mut bytes), format).unwrap();
        seeded(bytes, kind(Path::new(&format!("a.{}", format.extensions_str()[0]))).unwrap())
    }

    /// `bytes` with `sample_jpeg`'s tags, written the way little_exif writes them.
    fn seeded(mut bytes: Vec<u8>, kind: FileExtension) -> Vec<u8> {
        let tagged = Metadata::new_from_vec(&sample_jpeg(), FileExtension::JPEG).unwrap();
        // A TIFF's own tags (size, strips) have to stay.
        let mut m = Metadata::new_from_vec(&bytes, kind).unwrap_or_else(|_| Metadata::new());
        for tag in &tagged {
            m.set_tag(tag.clone());
        }
        m.write_to_vec(&mut bytes, kind).unwrap();
        bytes
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lightview-exif-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Edits `bytes` (saved as `name`) through `write_copy` and reads the copy back.
    fn edit_copy(dir: &Path, name: &str, bytes: &[u8], remove_gps: bool) -> (Dialog, Vec<u8>) {
        let source = dir.join(name);
        std::fs::write(&source, bytes).unwrap();
        let d = read(&source).unwrap();
        assert_eq!((d.original[1].as_str(), d.original[4].as_str()), ("Ana", "Acme"), "{name}");
        assert!(d.gps.is_some() && d.read_only.is_none(), "{name}");
        let mut values = d.original.clone();
        values[0] = "A walk".into();
        values[1] = "Bea".into();
        let dest = dir.join(format!("copy-{name}"));
        write_copy(&source, &dest, &d.original, &values, remove_gps).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), bytes, "the source is never touched");
        (read(&dest).unwrap(), std::fs::read(&dest).unwrap())
    }

    #[test]
    fn png_and_tiff_edits_reopen() {
        let dir = temp_dir("formats");
        let pages = seeded(std::fs::read("tests/data/pages.tif").unwrap(), FileExtension::TIFF);
        let files = [("a.png", sample(image::ImageFormat::Png)), ("a.tif", sample(image::ImageFormat::Tiff)), ("pages.tif", pages)];
        for (name, bytes) in files {
            for remove_gps in [false, true] {
                let (copy, written) = edit_copy(&dir, name, &bytes, remove_gps);
                assert_eq!((copy.original[0].as_str(), copy.original[1].as_str(), copy.original[4].as_str()), ("A walk", "Bea", "Acme"), "{name}");
                assert_eq!(copy.gps.is_some(), !remove_gps, "{name}");
                assert_eq!(image::load_from_memory(&written).unwrap(), image::load_from_memory(&bytes).unwrap(), "{name}: the pixels");
                let m = Metadata::new_from_vec(&written, kind(Path::new(name)).unwrap()).unwrap();
                assert!(m.get_tag(&ExifTag::MakerNote(Vec::new())).next().is_none(), "{name}: MakerNote dropped");
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn png_exif_is_written_as_the_standard_chunk() {
        let dir = temp_dir("exif-chunk");
        let (_, written) = edit_copy(&dir, "a.png", &sample(image::ImageFormat::Png), false);
        std::fs::remove_dir_all(dir).unwrap();
        let types: Vec<[u8; 4]> = png_chunks(&written).into_iter().map(|(_, kind, _)| kind).collect();
        assert!(types.contains(b"eXIf") && !types.contains(b"zTXt"), "{types:?}");
        // So Save As and other programs see it too.
        let exif = ImageReader::new(Cursor::new(&written)).with_guessed_format().unwrap().into_decoder().unwrap().exif_metadata().unwrap();
        assert!(exif.is_some_and(|e| e.windows(3).any(|w| w == b"Bea")));
    }

    #[test]
    fn many_pages_and_animations_survive() {
        let dir = temp_dir("frames");
        // More pages than the old eight-IFD limit allowed.
        let page = image::RgbImage::from_pixel(2, 2, image::Rgb([9, 9, 9]));
        let mut pages = Vec::new();
        {
            let mut enc = tiff::encoder::TiffEncoder::new(Cursor::new(&mut pages)).unwrap();
            for _ in 0..12 {
                enc.write_image::<tiff::encoder::colortype::RGB8>(2, 2, page.as_raw()).unwrap();
            }
        }
        assert!(crate::load::ifd_chain_ends(&pages));
        let apng = seeded(std::fs::read("tests/data/anim.apng").unwrap(), FileExtension::PNG { as_zTXt_chunk: false });
        let (_, written) = edit_copy(&dir, "anim.png", &apng, true);
        std::fs::remove_dir_all(dir).unwrap();
        let frames = |png: &[u8]| image::codecs::png::PngDecoder::new(Cursor::new(png)).unwrap().apng().unwrap().into_frames().count();
        use image::AnimationDecoder;
        assert_eq!(frames(&written), 2);
    }

    /// little_exif also takes the `exif:` properties out of a PNG's XMP on every write, so the
    /// location there goes whether or not it's asked; the rest of the XMP stays.
    #[test]
    fn png_xmp_naming_a_location_goes() {
        let chunk = |kind: &[u8], data: &[u8]| {
            let mut c = (data.len() as u32).to_be_bytes().to_vec();
            c.extend(kind);
            c.extend(data);
            c.extend(crc32fast::hash(&c[4..]).to_be_bytes());
            c
        };
        // A zlib stream of one stored (uncompressed) block.
        let zlib = |text: &[u8]| {
            let mut z = vec![0x78, 0x01, 0x01];
            z.extend((text.len() as u16).to_le_bytes());
            z.extend((!(text.len() as u16)).to_le_bytes());
            z.extend(text);
            let (mut a, mut b) = (1u32, 0u32);
            for &byte in text {
                a = (a + byte as u32) % 65521;
                b = (b + a) % 65521;
            }
            z.extend(((b << 16) | a).to_be_bytes());
            z
        };
        let with_xmp = |text: &str, compressed: u8| {
            let mut png = sample(image::ImageFormat::Png);
            let mut data = b"XML:com.adobe.xmp\0".to_vec();
            data.extend([compressed, 0, 0, 0]);
            data.extend(if compressed == 1 { zlib(text.as_bytes()) } else { text.as_bytes().to_vec() });
            png.splice(33..33, chunk(b"iTXt", &data));
            png
        };
        let dir = temp_dir("png-xmp");
        let has = |png: &[u8], text: &[u8]| png.windows(text.len()).any(|w| w == text);
        let gps = with_xmp(r#"<rdf:Description exif:GPSLatitude="40,25N"/>"#, 0);
        assert!(!has(&edit_copy(&dir, "gps.png", &gps, true).1, b"GPSLatitude"));
        assert!(!has(&edit_copy(&dir, "gps.png", &gps, false).1, b"GPSLatitude"));
        assert!(has(&edit_copy(&dir, "title.png", &with_xmp("<dc:title>Walk</dc:title>", 0), true).1, b"Walk"));
        // Compressed XMP can't be checked, so it goes too.
        let packed = with_xmp("<dc:title>Packed</dc:title>", 1);
        assert!(has(&packed, b"Packed"));
        assert!(!has(&edit_copy(&dir, "packed.png", &packed, true).1, b"Packed"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn webp_metadata_is_shown_not_edited() {
        let dir = temp_dir("webp");
        use image::ImageEncoder;
        let mut webp = Vec::new();
        let mut enc = image::codecs::webp::WebPEncoder::new_lossless(&mut webp);
        let tiff = Metadata::new_from_vec(&sample_jpeg(), FileExtension::JPEG).unwrap().encode().unwrap();
        enc.set_exif_metadata(tiff.strip_prefix(b"Exif\0\0").unwrap_or(&tiff).to_vec()).unwrap();
        enc.write_image(&[7; 16 * 8 * 3], 16, 8, image::ExtendedColorType::Rgb8).unwrap();
        let source = dir.join("a.webp");
        std::fs::write(&source, &webp).unwrap();
        let d = read(&source).unwrap();
        assert_eq!((d.read_only, d.original[1].as_str()), (Some("A WebP file's metadata is shown, not edited."), "Ana"));
        let mut values = d.original.clone();
        values[1] = "Bea".into();
        assert!(write_copy(&source, &dir.join("b.webp"), &d.original, &values, false).is_err());
        assert!(!dir.join("b.webp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_copy_keeps_the_source_format() {
        let dir = temp_dir("same-format");
        let source = dir.join("a.png");
        std::fs::write(&source, sample(image::ImageFormat::Png)).unwrap();
        let d = read(&source).unwrap();
        for wrong in ["b.jpg", "b.tif", "b"] {
            assert_eq!(write_copy(&source, &dir.join(wrong), &d.original, &d.original, true).err(), Some(trf("the copy must be a .{ext} file", &[("ext", &"png")])), "{wrong}");
        }
        std::fs::remove_dir_all(dir).unwrap();
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
        assert!(write_copy(&source, &dir.join("copy.png"), &d.original, &values, true).is_err(), "another format");
        write_copy(&source, &dir.join("copy.jpg"), &d.original, &values, true).unwrap();
        let copy = read(&dir.join("copy.jpg")).unwrap();
        let source_after = std::fs::read(&source).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(source_after, original);
        assert_eq!(copy.original[2], "© Ana");
        assert!(copy.gps.is_none());
    }
}
