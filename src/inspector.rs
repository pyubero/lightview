use crate::app::format_size;
use crate::edit::Doc;
use crate::exif_edit;
use crate::load::Loaded;
use eframe::egui;
use image::{ImageDecoder, ImageReader};
use std::path::{Path, PathBuf};

/// The inspector sidebar's data for one file.
#[derive(Debug)]
pub struct Inspector {
    pub path: PathBuf,
    profile: String,
    /// The JFIF density, for JPEGs without an EXIF resolution.
    jfif_dpi: Option<String>,
    /// The EXIF sections; `Err` says why there are none.
    pub exif: Result<exif_edit::Dialog, String>,
}

impl Inspector {
    pub fn new(path: &Path) -> Self {
        let jpeg = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
        let icc = ImageReader::open(path).ok().and_then(|r| r.with_guessed_format().ok()?.into_decoder().ok()?.icc_profile().ok()?);
        let profile = match icc {
            Some(icc) => icc_description(&icc).unwrap_or_else(|| "Embedded (unnamed)".into()),
            None => "None (sRGB assumed)".into(),
        };
        // shortcut: EXIF is read for JPEGs only, like EXIF editing; little_exif can also read PNG/WebP if wanted.
        let exif = if jpeg { exif_edit::read(path) } else { Err("EXIF is shown for JPEG files only".into()) };
        let jfif_dpi = jpeg.then(|| std::fs::File::open(path).ok()).flatten().and_then(|mut f| {
            let mut head = [0u8; 18];
            std::io::Read::read_exact(&mut f, &mut head).ok()?;
            jfif_density(&head)
        });
        Inspector { path: path.to_path_buf(), profile, jfif_dpi, exif }
    }
}

/// The sidebar body. Returns true when the user asks to save a copy with edited EXIF.
pub fn ui(ui: &mut egui::Ui, ins: &mut Inspector, loaded: &Loaded, doc: Option<&Doc>) -> bool {
    let mut save = false;
    // Fill the sidebar's width: shrinking to the content made the sidebar shrink with it.
    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        egui::CollapsingHeader::new("General").default_open(true).show(ui, |ui| {
            let (w, h) = (loaded.frames[0].w, loaded.frames[0].h);
            let ext = ins.path.extension().unwrap_or_default().to_string_lossy().to_uppercase();
            let mut dimensions = format!("{w} × {h} px");
            if let Some(d) = doc.filter(|d| d.size != [w, h]) {
                dimensions += &format!(" (edited: {} × {})", d.size[0], d.size[1]);
            }
            let resolution = ins.exif.as_ref().ok().and_then(|e| e.resolution.clone()).or(ins.jfif_dpi.clone()).unwrap_or("—".into());
            let rows = [
                ("Name", ins.path.file_name().unwrap_or_default().to_string_lossy().into_owned()),
                ("Folder", ins.path.parent().map(|p| p.display().to_string()).unwrap_or_default()),
                ("Format", if loaded.frames.len() > 1 { format!("{ext}, {} frames", loaded.frames.len()) } else { ext }),
                ("File size", format_size(loaded.file_size)),
                ("Dimensions", dimensions),
                ("Resolution", resolution),
                ("Colour profile", ins.profile.clone()),
            ];
            egui::Grid::new("general-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                for (k, v) in rows {
                    ui.label(k);
                    ui.add(egui::Label::new(v).wrap());
                    ui.end_row();
                }
            });
        });
        match &mut ins.exif {
            Ok(d) => save = exif_edit::sections_ui(ui, d),
            Err(e) => {
                ui.weak(e.as_str());
            }
        }
    });
    save
}

/// "72 dpi" from a JPEG's JFIF APP0 header, if it has one with a unit.
fn jfif_density(head: &[u8]) -> Option<String> {
    if head.get(0..4)? != [0xFF, 0xD8, 0xFF, 0xE0] || head.get(6..11)? != b"JFIF\0" {
        return None;
    }
    let x = u16::from_be_bytes([*head.get(14)?, *head.get(15)?]);
    match head.get(13)? {
        1 => Some(format!("{x} dpi")),
        2 => Some(format!("{x} dpcm")),
        _ => None,
    }
}

/// The profile's name from its `desc` tag (ICC v2 `desc` or v4 `mluc`). The data is
/// untrusted: every read is bounds-checked.
fn icc_description(icc: &[u8]) -> Option<String> {
    let be32 = |b: &[u8], o: usize| Some(u32::from_be_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?) as usize);
    let count = be32(icc, 128)?;
    let entry = (0..count.min(256)).map(|i| 132 + i * 12).find(|&e| icc.get(e..e + 4) == Some(b"desc"))?;
    let (offset, len) = (be32(icc, entry + 4)?, be32(icc, entry + 8)?);
    let tag = icc.get(offset..offset.checked_add(len)?)?;
    let text = match tag.get(0..4)? {
        b"desc" => {
            let n = be32(tag, 8)?;
            String::from_utf8_lossy(tag.get(12..12usize.checked_add(n)?)?).into_owned()
        }
        b"mluc" => {
            // The first record: its length and offset within the tag, as UTF-16BE.
            let (n, at) = (be32(tag, 20)?, be32(tag, 24)?);
            let units: Vec<u16> = tag.get(at..at.checked_add(n)?)?.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
            String::from_utf16_lossy(&units)
        }
        _ => return None,
    };
    let text = text.trim_end_matches('\0').trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ICC profile whose only tag is `desc` holding `tag`.
    fn icc_with(tag: &[u8]) -> Vec<u8> {
        let mut icc = vec![0u8; 128];
        icc.extend(1u32.to_be_bytes());
        icc.extend(b"desc");
        icc.extend(144u32.to_be_bytes());
        icc.extend((tag.len() as u32).to_be_bytes());
        icc.extend(tag);
        icc
    }

    #[test]
    fn icc_names() {
        let mut v2 = b"desc\0\0\0\0".to_vec();
        v2.extend(14u32.to_be_bytes());
        v2.extend(b"sRGB IEC61966\0");
        assert_eq!(icc_description(&icc_with(&v2)).as_deref(), Some("sRGB IEC61966"));

        let name: Vec<u8> = "Display P3".encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut v4 = b"mluc\0\0\0\0".to_vec();
        v4.extend(1u32.to_be_bytes());
        v4.extend(12u32.to_be_bytes());
        v4.extend(b"enUS");
        v4.extend((name.len() as u32).to_be_bytes());
        v4.extend(28u32.to_be_bytes());
        v4.extend(&name);
        assert_eq!(icc_description(&icc_with(&v4)).as_deref(), Some("Display P3"));

        // Truncated or junk input fails cleanly.
        let good = icc_with(&v4);
        for len in 0..good.len() {
            assert_eq!(icc_description(&good[..len]), None);
        }
        assert_eq!(icc_description(&[0xFF; 300]), None);
    }

    /// `LIGHTVIEW_INSPECT=photo.jpg cargo test print_inspector -- --ignored --nocapture`, to compare with other tools.
    #[test]
    #[ignore]
    fn print_inspector() {
        let path = std::env::var("LIGHTVIEW_INSPECT").unwrap();
        eprintln!("{:#?}", Inspector::new(Path::new(&path)));
    }

    #[test]
    fn jfif() {
        let mut head = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        head.extend(b"JFIF\0\x01\x01\x01\x01\x2c\x01\x2c");
        assert_eq!(jfif_density(&head).as_deref(), Some("300 dpi"));
        head[13] = 0; // aspect ratio only
        assert_eq!(jfif_density(&head), None);
        assert_eq!(jfif_density(&head[..10]), None);
    }
}
