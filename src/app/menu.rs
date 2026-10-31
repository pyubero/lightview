use super::{App, SelKind, Tool};
use crate::i18n::tr;
use crate::markup::{self, Pick, Shape};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use std::path::PathBuf;

#[derive(Clone, PartialEq)]
pub(super) enum Action {
    Open,
    SaveAs,
    OpenRecent(PathBuf),
    Close,
    Info,
    Settings,
    ZoomIn,
    ZoomOut,
    Fit,
    ActualSize,
    Fullscreen,
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    PrevImage,
    NextImage,
    Play,
    Pause,
    NextFrame,
    PrevFrame,
    Undo,
    Redo,
    Crop,
    Resize,
    AdjustColor,
    AutoLevels,
    ResetAdjust,
    Measure,
    ShowToolbar,
    Pick(Pick),
    Describe,
    SelectTool(SelKind),
    SelectAll,
    Deselect,
    Copy,
    Cut,
    Paste,
    DeleteSelection,
    DeleteImage,
    /// Remove Background, and crop to the subject when true.
    Cutout(bool),
    /// Denoise or Enhance Image: shows a before/after preview first.
    Restore(crate::ai::Task),
    /// Remove Object: fills in the selection.
    Inpaint,
    DownloadRuntime,
    AiModels,
}

const fn ctrl(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::COMMAND, key)
}

const fn key(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::NONE, key)
}

/// Keyboard shortcuts, also shown in the menus. Ctrl+Plus is an alias for Ctrl+=.
pub(super) const SHORTCUTS: [(KeyboardShortcut, Action); 19] = [
    (ctrl(Key::O), Action::Open),
    (KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::S), Action::SaveAs),
    (ctrl(Key::W), Action::Close),
    (ctrl(Key::I), Action::Info),
    (ctrl(Key::Equals), Action::ZoomIn),
    (ctrl(Key::Plus), Action::ZoomIn),
    (ctrl(Key::Minus), Action::ZoomOut),
    (ctrl(Key::Num0), Action::Fit),
    (ctrl(Key::Num1), Action::ActualSize),
    (key(Key::F11), Action::Fullscreen),
    (key(Key::ArrowLeft), Action::PrevImage),
    (key(Key::ArrowRight), Action::NextImage),
    (ctrl(Key::L), Action::RotateLeft),
    (ctrl(Key::R), Action::RotateRight),
    // Before Ctrl+Z, which would also match Ctrl+Shift+Z.
    (KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::Z), Action::Redo),
    (ctrl(Key::Z), Action::Undo),
    (KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::A), Action::Deselect),
    (ctrl(Key::A), Action::SelectAll),
    (ctrl(Key::Comma), Action::Settings),
];

impl App {
    pub(super) fn menu_bar(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let loaded = self.current().and_then(|p| self.cache.get(p));
        let has_image = loaded.is_some();
        // shortcut: animated images can't be saved; add GIF re-encoding if it's wanted.
        let can_save = loaded.is_some_and(|l| l.frames.len() == 1);
        fn item(ui: &mut egui::Ui, label: &str, action: Action, enabled: bool, actions: &mut Vec<Action>) {
            let mut button = egui::Button::new(label);
            if let Some((sc, _)) = SHORTCUTS.iter().find(|(_, a)| *a == action) {
                button = button.shortcut_text(ui.ctx().format_shortcut(sc));
            }
            if ui.add_enabled(enabled, button).clicked() {
                actions.push(action);
            }
        }
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("File"), |ui| {
                item(ui, tr("Open…"), Action::Open, true, actions);
                ui.menu_button(tr("Open Recent"), |ui| {
                    if self.recent.is_empty() {
                        ui.add_enabled(false, egui::Button::new(tr("No recent files")));
                    }
                    for path in &self.recent {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
                            actions.push(Action::OpenRecent(path.clone()));
                        }
                    }
                });
                item(ui, tr("Close"), Action::Close, true, actions);
                ui.separator();
                item(ui, tr("Save As…"), Action::SaveAs, can_save, actions);
                ui.separator();
                if ui.add_enabled(self.current().is_some(), egui::Button::new(tr("Delete Image…")).shortcut_text("Delete")).clicked() {
                    actions.push(Action::DeleteImage);
                }
            });
            ui.menu_button(tr("Edit"), |ui| {
                item(ui, tr("Undo"), Action::Undo, !(self.undo.is_empty() && self.deleted.is_empty()), actions);
                item(ui, tr("Redo"), Action::Redo, !self.redo.is_empty(), actions);
                ui.separator();
                let selected = self.selection.is_some();
                // Ctrl+C/X/V come as clipboard events, not key presses.
                for (label, action, enabled, keys) in [
                    (tr("Cut"), Action::Cut, selected, "Ctrl+X"),
                    (tr("Copy"), Action::Copy, has_image, "Ctrl+C"),
                    (tr("Paste"), Action::Paste, has_image, "Ctrl+V"),
                ] {
                    if ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys)).clicked() {
                        actions.push(action);
                    }
                }
                item(ui, tr("Delete Selection"), Action::DeleteSelection, selected, actions);
                ui.separator();
                item(ui, tr("Select All"), Action::SelectAll, has_image, actions);
                item(ui, tr("Deselect"), Action::Deselect, selected, actions);
                ui.separator();
                item(ui, tr("Settings…"), Action::Settings, true, actions);
            });
            ui.menu_button(tr("View"), |ui| {
                item(ui, tr("Previous Image"), Action::PrevImage, self.index > 0, actions);
                item(ui, tr("Next Image"), Action::NextImage, self.index + 1 < self.files.len(), actions);
                ui.separator();
                item(ui, tr("Zoom In"), Action::ZoomIn, has_image, actions);
                item(ui, tr("Zoom Out"), Action::ZoomOut, has_image, actions);
                item(ui, tr("Actual Size"), Action::ActualSize, has_image, actions);
                item(ui, tr("Fit to Window"), Action::Fit, has_image, actions);
                ui.separator();
                item(ui, tr("Enter Full Screen"), Action::Fullscreen, true, actions);
                ui.separator();
                let label = if self.show_info { tr("Hide Image Information") } else { tr("Show Image Information") };
                item(ui, label, Action::Info, has_image, actions);
                ui.separator();
                let animated = loaded.is_some_and(|l| l.frames.len() > 1);
                ui.add_enabled_ui(animated, |ui| {
                    ui.menu_button(tr("Animation"), |ui| {
                        item(ui, tr("Play Animation"), Action::Play, self.anim_paused, actions);
                        item(ui, tr("Pause Animation"), Action::Pause, !self.anim_paused, actions);
                        item(ui, tr("Next Frame"), Action::NextFrame, true, actions);
                        item(ui, tr("Previous Frame"), Action::PrevFrame, true, actions);
                    });
                });
            });
            ui.menu_button(tr("Image"), |ui| {
                item(ui, tr("Crop"), Action::Crop, has_image, actions);
                item(ui, tr("Resize…"), Action::Resize, has_image, actions);
                ui.separator();
                item(ui, tr("Rotate Left"), Action::RotateLeft, has_image, actions);
                item(ui, tr("Rotate Right"), Action::RotateRight, has_image, actions);
                item(ui, tr("Flip Horizontally"), Action::FlipHorizontal, has_image, actions);
                item(ui, tr("Flip Vertically"), Action::FlipVertical, has_image, actions);
                ui.separator();
                item(ui, tr("Adjust Color…"), Action::AdjustColor, has_image, actions);
                item(ui, tr("Auto Levels"), Action::AutoLevels, has_image, actions);
                item(ui, tr("Reset Color Adjustments"), Action::ResetAdjust, has_image, actions);
                ui.separator();
                ui.weak(tr("AI Image Processing"));
                let runtime = crate::ai::runtime_installed();
                let models = crate::ai::installed();
                use crate::ai::Task;
                let ready = |task| has_image && runtime && self.restoring.is_none() && crate::ai::choose(self.settings.ai.choice(task), task, &models).is_some();
                item(ui, tr("Remove Background"), Action::Cutout(false), ready(Task::Background), actions);
                item(ui, tr("Remove Background and Crop"), Action::Cutout(true), ready(Task::Background), actions);
                item(ui, tr("Denoise"), Action::Restore(Task::Denoise), ready(Task::Denoise), actions);
                item(ui, tr("Enhance Image"), Action::Restore(Task::Enhance), ready(Task::Enhance), actions);
                item(ui, tr("Remove Object"), Action::Inpaint, ready(Task::Inpaint) && self.selection.is_some(), actions);
            });
            ui.menu_button(tr("Tools"), |ui| {
                let label = if self.show_toolbar { tr("Hide Drawing Toolbar") } else { tr("Show Drawing Toolbar") };
                item(ui, label, Action::ShowToolbar, has_image, actions);
                ui.separator();
                for (label, kind) in [(tr("Rectangular Selection"), SelKind::Rect), (tr("Elliptical Selection"), SelKind::Ellipse), (tr("Lasso"), SelKind::Lasso)] {
                    if ui.add_enabled(has_image, egui::Button::selectable(self.tool == Tool::Selection(kind), label)).clicked() {
                        actions.push(Action::SelectTool(kind));
                    }
                }
                let measuring = self.tool == Tool::Measure;
                if ui.add_enabled(has_image, egui::Button::selectable(measuring, tr("Measure"))).clicked() {
                    actions.push(Action::Measure);
                }
                ui.separator();
                item(ui, tr("Sketch"), Action::Pick(Pick::Draw(Shape::Sketch)), has_image, actions);
                ui.add_enabled_ui(has_image, |ui| {
                    ui.menu_button(tr("Shapes"), |ui| {
                        for (shape, name) in markup::SHAPES.map(|(shape, name)| (shape, tr(name))) {
                            item(ui, name, Action::Pick(Pick::Draw(shape)), true, actions);
                        }
                    });
                });
                item(ui, tr("Text"), Action::Pick(Pick::Draw(Shape::Text)), has_image, actions);
                item(ui, tr("Highlight"), Action::Pick(Pick::Draw(Shape::Highlight)), has_image, actions);
                item(ui, tr("Magnifier"), Action::Pick(Pick::Draw(Shape::Magnifier)), has_image, actions);
                ui.separator();
                item(ui, tr("AI Models…"), Action::AiModels, true, actions);
                if !crate::ai::runtime_installed() {
                    item(ui, tr("Download AI Runtime…"), Action::DownloadRuntime, true, actions);
                }
            });
        });
    }
}
