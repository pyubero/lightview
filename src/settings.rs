use crate::ai::{Preprocess, Task};
use crate::i18n::{self, Language, tr};
use eframe::egui;
use fast_image_resize::{FilterType, ResizeAlg};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;

/// Key in eframe's storage (`~/.local/share/lightview/app.ron`).
pub const KEY: &str = "settings";

const DESKTOP_FILE: &str = "lightview.desktop";
const MIME_TYPES: [&str; 6] = ["image/jpeg", "image/png", "image/gif", "image/webp", "image/bmp", "image/tiff"];
/// egui's own body size, which every text style is scaled from.
const DEFAULT_FONT_SIZE: f32 = 13.0;
/// Capped at 20: the inspector panel is at most 320 pt wide and the toolbar has to fit too.
const FONT_SIZES: [f32; 8] = [10.0, 11.0, 12.0, DEFAULT_FONT_SIZE, 14.0, 16.0, 18.0, 20.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Interpolation {
    Nearest,
    #[default]
    Bilinear,
    Bicubic,
    Lanczos3,
}

impl Interpolation {
    const ALL: [Self; 4] = [Self::Nearest, Self::Bilinear, Self::Bicubic, Self::Lanczos3];

    pub fn cpu(self) -> ResizeAlg {
        match self {
            Self::Nearest => ResizeAlg::Nearest,
            Self::Bilinear => ResizeAlg::Convolution(FilterType::Bilinear),
            Self::Bicubic => ResizeAlg::Convolution(FilterType::CatmullRom),
            Self::Lanczos3 => ResizeAlg::Convolution(FilterType::Lanczos3),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Nearest => tr("Nearest"),
            Self::Bilinear => tr("Bilinear"),
            Self::Bicubic => tr("Bicubic"),
            Self::Lanczos3 => "Lanczos3",
        }
    }

    /// The GPU can only sample nearest or (bi)linear.
    fn gpu_ok(self) -> bool {
        matches!(self, Self::Nearest | Self::Bilinear)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum SortOrder {
    #[default]
    Name,
    DateModified,
}

/// The model each AI task uses, and per-file preprocessing the user set (Tools → AI Models…).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    /// Background removal's model (named before there were other tasks).
    pub model: Option<String>,
    pub denoise: Option<String>,
    pub enhance: Option<String>,
    pub inpaint: Option<String>,
    pub ocr: Option<String>,
    pub overrides: HashMap<String, Preprocess>,
    /// The Enhance models whose notice has been shown (once each).
    pub notices_seen: Vec<String>,
    /// Run models on the GPU when the GPU support is installed.
    #[serde(default = "yes")]
    pub use_gpu: bool,
}

fn yes() -> bool {
    true
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings { model: None, denoise: None, enhance: None, inpaint: None, ocr: None, overrides: HashMap::new(), notices_seen: Vec::new(), use_gpu: true }
    }
}

impl AiSettings {
    pub fn choice(&self, task: Task) -> Option<&String> {
        match task {
            Task::Background => self.model.as_ref(),
            Task::Denoise => self.denoise.as_ref(),
            Task::Enhance => self.enhance.as_ref(),
            Task::Inpaint => self.inpaint.as_ref(),
            Task::Ocr => self.ocr.as_ref(),
        }
    }

    pub fn choice_mut(&mut self, task: Task) -> &mut Option<String> {
        match task {
            Task::Background => &mut self.model,
            Task::Denoise => &mut self.denoise,
            Task::Enhance => &mut self.enhance,
            Task::Inpaint => &mut self.inpaint,
            Task::Ocr => &mut self.ocr,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub gpu_scaling: bool,
    pub interpolation: Interpolation,
    pub background: [u8; 3],
    pub window_size: [f32; 2],
    pub sort: SortOrder,
    pub ai: AiSettings,
    /// None until first launch, which takes the system language.
    pub language: Option<Language>,
    /// The first-launch "default image viewer" question has been dealt with.
    pub default_asked: bool,
    /// UI body text, in points; the other text styles keep their proportions to it.
    pub font_size: f32,
    /// Lines between the pixels when zoomed far in.
    pub pixel_grid: bool,
    /// Convert images with an ICC profile to sRGB, which assumes an sRGB monitor.
    pub color_managed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { gpu_scaling: false, interpolation: Interpolation::Bilinear, background: [0, 0, 0], window_size: [1280.0, 720.0], sort: SortOrder::Name, ai: AiSettings::default(), language: None, default_asked: false, font_size: DEFAULT_FONT_SIZE, pixel_grid: false, color_managed: true }
    }
}

impl Settings {
    /// The interpolation actually used: with GPU scaling, Bicubic/Lanczos3 fall back to Bilinear.
    pub fn interpolation(&self) -> Interpolation {
        if self.gpu_scaling && !self.interpolation.gpu_ok() { Interpolation::Bilinear } else { self.interpolation }
    }
}

/// Scales every text style from egui's defaults, so calling it again never compounds.
pub fn apply_font_size(ctx: &egui::Context, size: f32) {
    // The saved file can be edited by hand.
    let factor = size.clamp(FONT_SIZES[0], FONT_SIZES[FONT_SIZES.len() - 1]) / DEFAULT_FONT_SIZE;
    ctx.all_styles_mut(|style| {
        style.text_styles = egui::style::default_text_styles();
        style.text_styles.values_mut().for_each(|font| font.size *= factor);
    });
}

/// Reads the settings straight from eframe's storage file, so the window size is known
/// before the window is created. Anything missing or unreadable falls back to defaults.
pub fn load() -> Settings {
    let path = eframe::storage_dir("lightview").map(|d| d.join("app.ron"));
    path.and_then(|p| std::fs::read_to_string(p).ok()).map(|s| parse(&s)).unwrap_or_default()
}

fn parse(app_ron: &str) -> Settings {
    ron::from_str::<HashMap<String, String>>(app_ron)
        .ok()
        .and_then(|kv| ron::from_str(kv.get(KEY)?).ok())
        .unwrap_or_default()
}

/// Whether a desktop entry exists for `xdg-mime` to point at (the .deb or `install.sh` put it there).
pub fn desktop_installed() -> bool {
    let home = std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".local/share/applications").join(DESKTOP_FILE));
    std::path::Path::new("/usr/share/applications").join(DESKTOP_FILE).exists() || home.is_some_and(|p| p.exists())
}

pub fn is_default() -> bool {
    Command::new("xdg-mime").args(["query", "default", "image/jpeg"]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == DESKTOP_FILE)
}

/// Sets lightview as this user's default for the formats `install.sh` also claims.
pub fn make_default() -> Result<(), String> {
    crate::ai::run(Command::new("xdg-mime").args(["default", DESKTOP_FILE]).args(MIME_TYPES))
}

/// The Settings dialog body.
pub fn ui(ui: &mut egui::Ui, s: &mut Settings) {
    egui::Grid::new("settings-grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
        ui.label(tr("Language"));
        let current = s.language.unwrap_or(Language::English);
        egui::ComboBox::from_id_salt("language").selected_text(current.name()).show_ui(ui, |ui| {
            for language in Language::ALL {
                if ui.selectable_label(current == language, language.name()).clicked() {
                    s.language = Some(language);
                    i18n::set(language);
                }
            }
        });
        ui.end_row();

        ui.label(tr("Font size"));
        egui::ComboBox::from_id_salt("font-size").selected_text(s.font_size.to_string()).show_ui(ui, |ui| {
            for size in FONT_SIZES {
                if ui.selectable_value(&mut s.font_size, size, size.to_string()).clicked() {
                    apply_font_size(ui.ctx(), size);
                }
            }
        });
        ui.end_row();

        ui.label(tr("GPU image scaling"));
        ui.checkbox(&mut s.gpu_scaling, "").on_hover_text(tr("Smoother zooming and panning. Uses more GPU memory and makes switching images a little slower."));
        ui.end_row();

        ui.label(tr("Interpolation"));
        ui.vertical(|ui| {
            egui::ComboBox::from_id_salt("interpolation").selected_text(s.interpolation.name()).show_ui(ui, |ui| {
                for i in Interpolation::ALL {
                    ui.selectable_value(&mut s.interpolation, i, i.name());
                }
            });
            if s.interpolation() != s.interpolation {
                ui.weak(tr("Uses Bilinear while GPU scaling is on"));
            }
        });
        ui.end_row();

        ui.label(tr("Background colour"));
        ui.color_edit_button_srgb(&mut s.background);
        ui.end_row();

        ui.label(tr("Pixel grid"));
        ui.checkbox(&mut s.pixel_grid, "").on_hover_text(tr("Shows the edges of the pixels when zoomed far in."));
        ui.end_row();

        ui.label(tr("Colour management"));
        ui.checkbox(&mut s.color_managed, "").on_hover_text(tr("Converts images with a colour profile to sRGB, so wide-gamut photos look right on an sRGB monitor. Turn it off for a wide-gamut monitor."));
        ui.end_row();

        ui.label(tr("Default window size"));
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.window_size[0]).range(320.0..=7680.0).suffix(" px"));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut s.window_size[1]).range(240.0..=4320.0).suffix(" px"));
            });
            ui.weak(tr("Applies the next time lightview starts"));
        });
        ui.end_row();

        ui.label(tr("Sort order"));
        egui::ComboBox::from_id_salt("sort").selected_text(match s.sort {
            SortOrder::Name => tr("File name"),
            SortOrder::DateModified => tr("Date modified"),
        }).show_ui(ui, |ui| {
            ui.selectable_value(&mut s.sort, SortOrder::Name, tr("File name"));
            ui.selectable_value(&mut s.sort, SortOrder::DateModified, tr("Date modified"));
        });
        ui.end_row();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_round_trip() {
        let mut ai = AiSettings { model: Some("mine.onnx".into()), ..AiSettings::default() };
        ai.overrides.insert("mine.onnx".into(), Preprocess::auto(512));
        let s = Settings { gpu_scaling: true, interpolation: Interpolation::Lanczos3, background: [1, 2, 3], window_size: [800.0, 600.0], sort: SortOrder::DateModified, ai, language: Some(Language::German), default_asked: true, font_size: 18.0, pixel_grid: true, color_managed: false };
        // eframe stores each value as a RON string inside a RON map.
        let kv = HashMap::from([(KEY.to_string(), ron::to_string(&s).unwrap()), ("recent".to_string(), "/a".to_string())]);
        assert_eq!(parse(&ron::to_string(&kv).unwrap()), s);
    }

    #[test]
    fn older_ai_settings_still_load() {
        let kv = HashMap::from([(KEY.to_string(), r#"(ai: (model: Some("silueta.onnx")))"#.to_string())]);
        let s = parse(&ron::to_string(&kv).unwrap());
        assert_eq!(s.ai.choice(Task::Background).map(String::as_str), Some("silueta.onnx"));
        assert_eq!((s.ai.denoise, s.ai.enhance, s.ai.notices_seen.len()), (None, None, 0));
        assert!(!s.default_asked && s.language.is_none());
    }

    #[test]
    fn missing_or_broken_falls_back() {
        assert_eq!(parse(""), Settings::default());
        assert_eq!(parse("not ron at all"), Settings::default());
        // A stored value missing newer fields keeps the defaults for them.
        let kv = HashMap::from([(KEY.to_string(), "(gpu_scaling: true)".to_string())]);
        assert_eq!(parse(&ron::to_string(&kv).unwrap()), Settings { gpu_scaling: true, ..Settings::default() });
    }

    #[test]
    fn font_size_scales_every_text_style() {
        use egui::TextStyle::{Body, Heading};
        let ctx = egui::Context::default();
        let size = |style| ctx.global_style().text_styles[&style].size;
        apply_font_size(&ctx, 18.0);
        apply_font_size(&ctx, 18.0);
        assert!((size(Body) - 18.0).abs() < 1e-4);
        assert!((size(Heading) - 18.0 * 18.0 / 13.0).abs() < 1e-4);
        apply_font_size(&ctx, 500.0);
        assert!((size(Body) - 20.0).abs() < 1e-4);
        // Also catches egui changing its defaults under DEFAULT_FONT_SIZE.
        apply_font_size(&ctx, DEFAULT_FONT_SIZE);
        assert_eq!(ctx.global_style().text_styles, egui::style::default_text_styles());
    }

    #[test]
    fn gpu_falls_back_to_bilinear() {
        let s = Settings { gpu_scaling: true, interpolation: Interpolation::Lanczos3, ..Settings::default() };
        assert_eq!(s.interpolation(), Interpolation::Bilinear);
        assert_eq!(Settings { gpu_scaling: false, ..s.clone() }.interpolation(), Interpolation::Lanczos3);
        assert_eq!(Settings { interpolation: Interpolation::Nearest, ..s.clone() }.interpolation(), Interpolation::Nearest);
    }
}
