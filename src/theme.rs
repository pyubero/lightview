//! UI themes: XML files that recolour and reshape the fixed UI (see docs/FEATURES.md, Themes).

use eframe::egui::{self, Color32, CornerRadius, Vec2};
use std::path::{Path, PathBuf};

use crate::i18n::tr;

/// The fallback for a missing key or a broken file: the Light template as shipped.
const BASE: &str = include_str!("../themes/light.xml");
/// Written into the themes folder on first launch, as files users edit.
pub const TEMPLATES: [(&str, &str); 3] = [
    ("dark", include_str!("../themes/dark.xml")),
    ("light", BASE),
    ("high-contrast", include_str!("../themes/high-contrast.xml")),
];
/// Follows the system's dark/light setting with the folder's dark.xml and light.xml.
pub const SYSTEM: &str = "system";
const MAX_BYTES: u64 = 64 * 1024;

/// Colours drawn over the image, which egui's `Visuals` has no slot for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    /// Selection, crop and other outlines.
    pub outline: Color32,
    /// The selected markup element's frame and handle.
    pub handle: Color32,
    /// Perspective and measuring guides.
    pub guide: Color32,
}

pub struct Theme {
    /// Which of egui's dark/light slots it fills, and where the values no theme sets come from.
    pub mode: egui::Theme,
    pub visuals: egui::Visuals,
    pub spacing: egui::Spacing,
    pub canvas: Canvas,
}

/// The name shown in Settings: the shipped themes translated, others by file name.
pub fn name(id: &str) -> String {
    match id {
        SYSTEM => tr("Follow system").into(),
        "dark" => tr("Dark").into(),
        "light" => tr("Light").into(),
        "high-contrast" => tr("High contrast").into(),
        _ => id.into(),
    }
}

/// Where the theme files live.
pub fn dir() -> Option<PathBuf> {
    eframe::storage_dir("lightview").map(|d| d.join("themes"))
}

/// Opens the themes folder in the file manager.
pub fn open_folder() {
    if let Some(dir) = dir() {
        seed(&dir);
        crate::platform::open(dir);
    }
}

/// Creates the themes folder with the templates on first launch.
pub fn seed_folder() {
    if let Some(dir) = dir() {
        seed(&dir);
    }
}

/// An existing folder is left alone, so edits and deletions stay and upgrades never overwrite.
fn seed(dir: &Path) {
    if dir.exists() || std::fs::create_dir_all(dir).is_err() {
        return;
    }
    for (id, xml) in TEMPLATES {
        let _ = std::fs::write(dir.join(format!("{id}.xml")), xml);
    }
}

/// The ids of the themes in the folder, sorted.
pub fn user_themes() -> Vec<String> {
    dir().map(|d| themes_in(&d)).unwrap_or_default()
}

fn themes_in(dir: &Path) -> Vec<String> {
    let entries = std::fs::read_dir(dir).ok();
    let mut ids: Vec<String> = entries.into_iter().flatten().flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml")))
        .filter_map(|p| p.file_stem()?.to_str().map(String::from))
        .filter(|id| id != SYSTEM)
        .collect();
    ids.sort();
    ids
}

/// Reads a theme from the folder by id.
pub fn load(id: &str) -> Result<Theme, String> {
    read(&dir().ok_or_else(|| tr("no data folder").to_string())?, id)
}

fn read(dir: &Path, id: &str) -> Result<Theme, String> {
    let path = dir.join(format!("{id}.xml"));
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    // A theme is any file a user downloaded: keep it small.
    if std::fs::metadata(&path).map_err(error)?.len() > MAX_BYTES {
        return Err(format!("{}: {}", path.display(), tr("bigger than 64 KiB")));
    }
    parse(&std::fs::read_to_string(&path).map_err(error)?).map_err(|e| format!("{}: {e}", path.display()))
}

/// The built-in base theme.
fn base() -> Theme {
    parse(BASE).expect("the base theme parses")
}

/// Applies the theme `id` to `ctx`, keeping the text sizes. A theme that fails to load is
/// replaced by the base theme and the error is returned with the overlay colours.
pub fn apply(ctx: &egui::Context, id: &str) -> (Canvas, Option<String>) {
    let set = |slot, t: &Theme| ctx.style_mut_of(slot, |s| {
        s.visuals = t.visuals.clone();
        s.spacing = t.spacing.clone();
    });
    let or_base = |id| match load(id) {
        Ok(t) => (t, None),
        Err(e) => (base(), Some(e)),
    };
    if id == SYSTEM {
        let ((dark, dark_error), (light, light_error)) = (or_base("dark"), or_base("light"));
        // Each file fills its slot whatever its mode says, so a broken dark.xml shows the base theme.
        set(egui::Theme::Dark, &dark);
        set(egui::Theme::Light, &light);
        ctx.set_theme(egui::ThemePreference::System);
        // On Linux, `main` sets egui's fallback theme from the desktop portal.
        // shortcut: the overlays take dark.xml's colours in either mode.
        return (dark.canvas, dark_error.or(light_error));
    }
    let (theme, error) = or_base(id);
    set(theme.mode, &theme);
    ctx.set_theme(theme.mode);
    (theme.canvas, error)
}

/// A theme file: egui's defaults for its mode, then the base theme's values, then the file's.
pub fn parse(xml: &str) -> Result<Theme, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if root.tag_name().name() != "theme" {
        return Err(format!("<{}>: {}", root.tag_name().name(), tr("invalid value")));
    }
    let mode = match root.attribute("mode") {
        None | Some("dark") => egui::Theme::Dark,
        Some("light") => egui::Theme::Light,
        Some(other) => return Err(format!("mode=\"{other}\": {}", tr("invalid value"))),
    };
    let fallback = Canvas { outline: Color32::WHITE, handle: Color32::WHITE, guide: Color32::WHITE };
    let mut t = Theme { mode, visuals: mode.default_visuals(), spacing: egui::Spacing::default(), canvas: fallback };
    apply_elements(&roxmltree::Document::parse(BASE).expect("the base theme is XML"), &mut t)?;
    apply_elements(&doc, &mut t)?;
    Ok(t)
}

fn apply_elements(doc: &roxmltree::Document, t: &mut Theme) -> Result<(), String> {
    for node in doc.root_element().children().filter(|n| n.is_element()) {
        let line = doc.text_pos_at(node.range().start).row;
        let bad = |key: &str, v: &str| format!("{} {line}: {key}=\"{v}\": {}", tr("line"), tr("invalid value"));
        let color = |key: &str| node.attribute(key).map(|v| parse_color(v).ok_or_else(|| bad(key, v))).transpose();
        let number = |key: &str, max: f32| node.attribute(key).map(|v| v.trim().parse::<f32>().ok().filter(|n| n.is_finite()).map(|n| n.clamp(0.0, max)).ok_or_else(|| bad(key, v))).transpose();
        let pair = |key: &str| node.attribute(key).map(|v| parse_pair(v).ok_or_else(|| bad(key, v))).transpose();
        let v = &mut t.visuals;
        match node.tag_name().name() {
            "colors" => {
                if let Some(c) = color("panel")? { v.panel_fill = c; }
                if let Some(c) = color("window")? { v.window_fill = c; }
                if let Some(c) = color("border")? { v.window_stroke.color = c; }
                if let Some(c) = color("field")? { v.extreme_bg_color = c; }
                if let Some(c) = color("text")? { v.widgets.noninteractive.fg_stroke.color = c; }
                if let Some(c) = color("accent")? { v.selection.bg_fill = c; }
                if let Some(c) = color("selected-text")? { v.selection.stroke.color = c; }
                if let Some(c) = color("hyperlink")? { v.hyperlink_color = c; }
                if let Some(c) = color("warn")? { v.warn_fg_color = c; }
                if let Some(c) = color("error")? { v.error_fg_color = c; }
            }
            "widget" => {
                let w = match node.attribute("state") {
                    Some("noninteractive") => &mut v.widgets.noninteractive,
                    Some("inactive") => &mut v.widgets.inactive,
                    Some("hovered") => &mut v.widgets.hovered,
                    Some("active") => &mut v.widgets.active,
                    Some("open") => &mut v.widgets.open,
                    other => return Err(bad("state", other.unwrap_or(""))),
                };
                // Buttons fill with weak_bg_fill, other widgets with bg_fill.
                if let Some(c) = color("fill")? { (w.bg_fill, w.weak_bg_fill) = (c, c); }
                if let Some(c) = color("text")? { w.fg_stroke.color = c; }
                if let Some(c) = color("border")? { w.bg_stroke.color = c; }
            }
            "canvas" => {
                if let Some(c) = color("outline")? { t.canvas.outline = c; }
                if let Some(c) = color("handle")? { t.canvas.handle = c; }
                if let Some(c) = color("guide")? { t.canvas.guide = c; }
            }
            "shape" => {
                if let Some(r) = number("rounding", 16.0)? {
                    let r = CornerRadius::same(r as u8);
                    (v.window_corner_radius, v.menu_corner_radius) = (r, r);
                    for w in widgets(v) { w.corner_radius = r; }
                }
                if let Some(s) = number("stroke", 4.0)? {
                    let s = s.max(0.5);
                    v.window_stroke.width = s;
                    for w in widgets(v) { w.bg_stroke.width = s; }
                }
                if let Some(p) = pair("spacing")? { t.spacing.item_spacing = p; }
                if let Some(p) = pair("padding")? { t.spacing.button_padding = p; }
            }
            _ => {} // Unknown elements are ignored, so newer themes still load.
        }
    }
    Ok(())
}

fn widgets(v: &mut egui::Visuals) -> [&mut egui::style::WidgetVisuals; 5] {
    let w = &mut v.widgets;
    [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open]
}

/// `#rrggbb` or `#rrggbbaa`.
fn parse_color(s: &str) -> Option<Color32> {
    let hex = s.trim().strip_prefix('#').filter(|h| (h.len() == 6 || h.len() == 8) && h.is_ascii());
    let bytes = hex.and_then(|h| (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok()).collect::<Option<Vec<u8>>>());
    match bytes.as_deref() {
        Some([r, g, b]) => Some(Color32::from_rgb(*r, *g, *b)),
        Some([r, g, b, a]) => Some(Color32::from_rgba_unmultiplied(*r, *g, *b, *a)),
        _ => None,
    }
}

/// `x,y`, each clamped to 0–24.
fn parse_pair(s: &str) -> Option<Vec2> {
    let (x, y) = s.split_once(',')?;
    let n = |v: &str| v.trim().parse::<f32>().ok().filter(|n| n.is_finite()).map(|n| n.clamp(0.0, 24.0));
    Some(Vec2::new(n(x)?, n(y)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `xml` applied over egui's defaults alone, with no base theme underneath.
    fn without_base(xml: &str) -> egui::Visuals {
        let doc = roxmltree::Document::parse(xml).unwrap();
        let mode = if doc.root_element().attribute("mode") == Some("light") { egui::Theme::Light } else { egui::Theme::Dark };
        let mut t = Theme { mode, visuals: mode.default_visuals(), spacing: egui::Spacing::default(), canvas: Canvas { outline: Color32::TRANSPARENT, handle: Color32::TRANSPARENT, guide: Color32::TRANSPARENT } };
        apply_elements(&doc, &mut t).unwrap();
        assert!(![t.canvas.outline, t.canvas.handle, t.canvas.guide].contains(&Color32::TRANSPARENT));
        t.visuals
    }

    #[test]
    fn templates_parse_and_set_every_key() {
        for (id, xml) in TEMPLATES {
            let t = parse(xml).unwrap_or_else(|e| panic!("{id}: {e}"));
            // Same result with or without the base: the template relies on none of its values.
            assert_eq!(t.visuals, without_base(xml), "{id} leaves a key to the base theme");
        }
        assert_eq!(parse(TEMPLATES[1].1).unwrap().mode, egui::Theme::Light);
    }

    #[test]
    fn missing_keys_take_the_base_values() {
        let t = parse(r##"<theme><colors panel="#102030"/><widget state="hovered" fill="#ff000080"/><unknown/></theme>"##).unwrap();
        let base = base();
        assert_eq!(t.mode, egui::Theme::Dark);
        assert_eq!(t.visuals.panel_fill, Color32::from_rgb(16, 32, 48));
        assert_eq!(t.visuals.widgets.hovered.weak_bg_fill, Color32::from_rgba_unmultiplied(255, 0, 0, 128));
        assert_eq!(t.visuals.window_fill, base.visuals.window_fill);
        assert_eq!(t.visuals.widgets.inactive, base.visuals.widgets.inactive);
        assert_eq!(t.canvas, base.canvas);
    }

    #[test]
    fn bad_values_are_errors_with_their_line() {
        let e = parse("<theme>\n<colors\n panel=\"red\"/></theme>").err().unwrap();
        assert!(e.contains(" 2: panel=\"red\""), "{e}");
        assert!(parse(r#"<theme mode="sepia"/>"#).is_err());
        assert!(parse(r#"<theme><widget state="pressed"/></theme>"#).is_err());
        assert!(parse(r#"<theme><shape spacing="8"/></theme>"#).is_err());
        assert!(parse("<colours/>").is_err());
        assert!(parse("not xml").is_err());
        assert!(parse_color("#ééé").is_none());
    }

    #[test]
    fn numbers_are_clamped() {
        assert!(parse(r#"<theme><shape padding="NaN,1"/></theme>"#).is_err());
        let t = parse(r#"<theme><shape rounding="900" stroke="-3" spacing="100,-1"/></theme>"#).unwrap();
        assert_eq!(t.visuals.window_corner_radius, CornerRadius::same(16));
        assert_eq!(t.visuals.widgets.inactive.bg_stroke.width, 0.5);
        assert_eq!(t.spacing.item_spacing, Vec2::new(24.0, 0.0));
    }

    #[test]
    fn a_missing_theme_applies_the_base() {
        let ctx = egui::Context::default();
        let (canvas, error) = apply(&ctx, "no such theme, surely");
        assert!(error.is_some());
        assert_eq!(canvas, base().canvas);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert_eq!(ctx.style_of(egui::Theme::Light).visuals, base().visuals);
    }

    #[test]
    fn folder_is_seeded_once_and_read() {
        let dir = std::env::temp_dir().join(format!("lightview-theme-test-{}", std::process::id())).join("themes");
        seed(&dir);
        assert_eq!(themes_in(&dir), ["dark", "high-contrast", "light"]);
        assert_eq!(read(&dir, "high-contrast").unwrap().visuals.panel_fill, Color32::BLACK);
        std::fs::remove_file(dir.join("dark.xml")).unwrap();
        std::fs::write(dir.join("big.xml"), format!("<theme>{}</theme>", " ".repeat(65 * 1024))).unwrap();
        seed(&dir);
        assert_eq!(themes_in(&dir), ["big", "high-contrast", "light"]);
        assert!(read(&dir, "big").is_err());
        assert!(read(&dir, "dark").is_err());
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }
}
