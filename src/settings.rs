use crate::ai::{Preprocess, Task};
use eframe::egui;
use fast_image_resize::{FilterType, ResizeAlg};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;

/// Key in eframe's storage (`~/.local/share/lightview/app.ron`).
pub const KEY: &str = "settings";

const DESKTOP_FILE: &str = "lightview.desktop";
const MIME_TYPES: [&str; 5] = ["image/jpeg", "image/png", "image/gif", "image/webp", "image/bmp"];

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
        AiSettings { model: None, denoise: None, enhance: None, overrides: HashMap::new(), notices_seen: Vec::new(), use_gpu: true }
    }
}

impl AiSettings {
    pub fn choice(&self, task: Task) -> Option<&String> {
        match task {
            Task::Background => self.model.as_ref(),
            Task::Denoise => self.denoise.as_ref(),
            Task::Enhance => self.enhance.as_ref(),
        }
    }

    pub fn choice_mut(&mut self, task: Task) -> &mut Option<String> {
        match task {
            Task::Background => &mut self.model,
            Task::Denoise => &mut self.denoise,
            Task::Enhance => &mut self.enhance,
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
    /// The first-launch "default image viewer" question has been dealt with.
    pub default_asked: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { gpu_scaling: false, interpolation: Interpolation::Bilinear, background: [0, 0, 0], window_size: [1280.0, 720.0], sort: SortOrder::Name, ai: AiSettings::default(), default_asked: false }
    }
}

impl Settings {
    /// The interpolation actually used: with GPU scaling, Bicubic/Lanczos3 fall back to Bilinear.
    pub fn interpolation(&self) -> Interpolation {
        if self.gpu_scaling && !self.interpolation.gpu_ok() { Interpolation::Bilinear } else { self.interpolation }
    }
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
        ui.label("GPU image scaling");
        ui.checkbox(&mut s.gpu_scaling, "").on_hover_text("Smoother zooming and panning. Uses more GPU memory and makes switching images a little slower.");
        ui.end_row();

        ui.label("Interpolation");
        ui.vertical(|ui| {
            egui::ComboBox::from_id_salt("interpolation").selected_text(format!("{:?}", s.interpolation)).show_ui(ui, |ui| {
                for i in Interpolation::ALL {
                    ui.selectable_value(&mut s.interpolation, i, format!("{i:?}"));
                }
            });
            if s.interpolation() != s.interpolation {
                ui.weak("Uses Bilinear while GPU scaling is on");
            }
        });
        ui.end_row();

        ui.label("Background colour");
        ui.color_edit_button_srgb(&mut s.background);
        ui.end_row();

        ui.label("Default window size");
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.window_size[0]).range(320.0..=7680.0).suffix(" px"));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut s.window_size[1]).range(240.0..=4320.0).suffix(" px"));
            });
            ui.weak("Applies the next time lightview starts");
        });
        ui.end_row();

        ui.label("Sort order");
        egui::ComboBox::from_id_salt("sort").selected_text(match s.sort {
            SortOrder::Name => "File name",
            SortOrder::DateModified => "Date modified",
        }).show_ui(ui, |ui| {
            ui.selectable_value(&mut s.sort, SortOrder::Name, "File name");
            ui.selectable_value(&mut s.sort, SortOrder::DateModified, "Date modified");
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
        let s = Settings { gpu_scaling: true, interpolation: Interpolation::Lanczos3, background: [1, 2, 3], window_size: [800.0, 600.0], sort: SortOrder::DateModified, ai, default_asked: true };
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
        assert!(!s.default_asked);
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
    fn gpu_falls_back_to_bilinear() {
        let s = Settings { gpu_scaling: true, interpolation: Interpolation::Lanczos3, ..Settings::default() };
        assert_eq!(s.interpolation(), Interpolation::Bilinear);
        assert_eq!(Settings { gpu_scaling: false, ..s.clone() }.interpolation(), Interpolation::Lanczos3);
        assert_eq!(Settings { interpolation: Interpolation::Nearest, ..s.clone() }.interpolation(), Interpolation::Nearest);
    }
}
