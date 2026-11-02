use super::{App, SelKind, Tool};
use crate::i18n::{tr, tr_key};
use crate::markup::{self, Pick, Shape};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Action {
    Open,
    SaveAs,
    SaveCopy,
    CopyPath,
    CopyFile,
    ShowInFolder,
    Rename,
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
    FirstImage,
    LastImage,
    Play,
    Pause,
    NextFrame,
    PrevFrame,
    Undo,
    Redo,
    Crop,
    Perspective,
    Resize,
    AdjustColor,
    AdjustCurves,
    AutoLevels,
    Greyscale,
    Invert,
    Straighten,
    RedEye,
    Pixelate,
    ResetAdjust,
    Measure,
    CopyColor,
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
    /// Recognize Text, or Read QR Code when true: the selection, or the whole image.
    ReadText(bool),
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
pub(super) const SHORTCUTS: [(KeyboardShortcut, Action); 25] = [
    (ctrl(Key::O), Action::Open),
    (KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::S), Action::SaveAs),
    // After Ctrl+Shift+S, which it would also match.
    (ctrl(Key::S), Action::SaveCopy),
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
    (key(Key::Home), Action::FirstImage),
    (key(Key::End), Action::LastImage),
    (key(Key::PageDown), Action::NextFrame),
    (key(Key::PageUp), Action::PrevFrame),
    (key(Key::F2), Action::Rename),
    (ctrl(Key::L), Action::RotateLeft),
    (ctrl(Key::R), Action::RotateRight),
    // Before Ctrl+Z, which would also match Ctrl+Shift+Z.
    (KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::Z), Action::Redo),
    (ctrl(Key::Z), Action::Undo),
    (KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::A), Action::Deselect),
    (ctrl(Key::A), Action::SelectAll),
    (ctrl(Key::Comma), Action::Settings),
];

/// Every menu action a shortcut can be given, with its menu name, in menu order.
pub(super) fn bindable() -> Vec<(Action, &'static str)> {
    bindable_menus().into_iter().flat_map(|(_, items)| items).collect()
}

/// `bindable`, under the menus they're in. Cut, Copy and Paste are left out: they arrive as
/// clipboard events, always Ctrl+X/C/V.
pub(super) fn bindable_menus() -> [(&'static str, Vec<(Action, &'static str)>); 5] {
    use crate::ai::Task;
    let file = vec![
        (Action::Open, tr_key("Open…")),
        (Action::Close, tr_key("Close")),
        (Action::SaveCopy, tr_key("Save Copy")),
        (Action::SaveAs, tr_key("Save As…")),
        (Action::CopyPath, tr_key("Copy Path")),
        (Action::CopyFile, tr_key("Copy File")),
        (Action::ShowInFolder, tr_key("Show in File Manager")),
        (Action::Rename, tr_key("Rename…")),
        (Action::DeleteImage, tr_key("Delete Image…")),
    ];
    let edit = vec![
        (Action::Undo, tr_key("Undo")),
        (Action::Redo, tr_key("Redo")),
        (Action::DeleteSelection, tr_key("Delete Selection")),
        (Action::SelectAll, tr_key("Select All")),
        (Action::Deselect, tr_key("Deselect")),
        (Action::Settings, tr_key("Settings…")),
    ];
    let view = vec![
        (Action::PrevImage, tr_key("Previous Image")),
        (Action::NextImage, tr_key("Next Image")),
        (Action::FirstImage, tr_key("First Image")),
        (Action::LastImage, tr_key("Last Image")),
        (Action::ZoomIn, tr_key("Zoom In")),
        (Action::ZoomOut, tr_key("Zoom Out")),
        (Action::ActualSize, tr_key("Actual Size")),
        (Action::Fit, tr_key("Fit to Window")),
        (Action::Fullscreen, tr_key("Enter Full Screen")),
        (Action::Info, tr_key("Show Image Information")),
        (Action::Play, tr_key("Play Animation")),
        (Action::Pause, tr_key("Pause Animation")),
        (Action::NextFrame, tr_key("Next Frame")),
        (Action::PrevFrame, tr_key("Previous Frame")),
    ];
    let image = vec![
        (Action::Crop, tr_key("Crop")),
        (Action::Resize, tr_key("Resize…")),
        (Action::Perspective, tr_key("Correct Perspective")),
        (Action::Straighten, tr_key("Straighten…")),
        (Action::RotateLeft, tr_key("Rotate Left")),
        (Action::RotateRight, tr_key("Rotate Right")),
        (Action::FlipHorizontal, tr_key("Flip Horizontally")),
        (Action::FlipVertical, tr_key("Flip Vertically")),
        (Action::AdjustColor, tr_key("Adjust Color…")),
        (Action::AdjustCurves, tr_key("Adjust Curves…")),
        (Action::AutoLevels, tr_key("Auto Levels")),
        (Action::ResetAdjust, tr_key("Reset Color Adjustments")),
        (Action::Greyscale, tr_key("Greyscale")),
        (Action::Invert, tr_key("Invert Colors")),
        (Action::RedEye, tr_key("Remove Red Eye")),
        (Action::Pixelate, tr_key("Pixelate Selection")),
        (Action::Cutout(false), tr_key("Remove Background")),
        (Action::Cutout(true), tr_key("Remove Background and Crop")),
        (Action::Restore(Task::Denoise), tr_key("Denoise")),
        (Action::Restore(Task::Enhance), tr_key("Enhance Image")),
        (Action::Inpaint, tr_key("Remove Object")),
        (Action::ReadText(false), tr_key("Recognize Text")),
    ];
    let mut tools = vec![
        (Action::ShowToolbar, tr_key("Show Drawing Toolbar")),
        (Action::SelectTool(SelKind::Rect), tr_key("Rectangular Selection")),
        (Action::SelectTool(SelKind::Ellipse), tr_key("Elliptical Selection")),
        (Action::SelectTool(SelKind::Lasso), tr_key("Lasso")),
        (Action::Measure, tr_key("Measure")),
        (Action::CopyColor, tr_key("Copy Color")),
        (Action::ReadText(true), tr_key("Read QR Code")),
        (Action::Pick(Pick::Draw(Shape::Sketch)), tr_key("Sketch")),
    ];
    tools.extend(markup::SHAPES.map(|(shape, name)| (Action::Pick(Pick::Draw(shape)), name)));
    tools.extend([
        (Action::Pick(Pick::Draw(Shape::Text)), tr_key("Text")),
        (Action::Pick(Pick::Draw(Shape::Highlight)), tr_key("Highlight")),
        (Action::Pick(Pick::Draw(Shape::Magnifier)), tr_key("Magnifier")),
        (Action::AiModels, tr_key("AI Models…")),
        (Action::DownloadRuntime, tr_key("Download AI Runtime…")),
    ]);
    [(tr_key("File"), file), (tr_key("Edit"), edit), (tr_key("View"), view), (tr_key("Image"), image), (tr_key("Tools"), tools)]
}

/// Keys with a fixed meaning (clipboard, confirm, cancel, delete), never offered as shortcuts.
const RESERVED: [KeyboardShortcut; 7] = [ctrl(Key::C), ctrl(Key::X), ctrl(Key::V), key(Key::Enter), key(Key::Escape), key(Key::Delete), key(Key::Backspace)];

/// The shortcuts in force: the defaults with the user's `changes` (see `Settings::shortcuts`).
/// Most modifiers first: egui ignores an extra Shift, so Ctrl+Z would also catch Ctrl+Shift+Z.
pub(super) fn shortcuts(changes: &[(String, Option<KeyboardShortcut>)]) -> Vec<(KeyboardShortcut, Action)> {
    let change = |a: &Action| changes.iter().find(|(name, _)| *name == format!("{a:?}")).map(|(_, sc)| *sc);
    let mut keys: Vec<(KeyboardShortcut, Action)> = SHORTCUTS.iter().filter(|(_, a)| change(a).is_none()).cloned().collect();
    keys.extend(bindable().into_iter().filter_map(|(a, _)| Some((change(&a)??, a))));
    let count = |m: Modifiers| m.alt as u8 + m.shift as u8 + (m.ctrl || m.command) as u8 + m.mac_cmd as u8;
    keys.sort_by_key(|(sc, _)| std::cmp::Reverse(count(sc.modifiers)));
    keys
}

/// Why `sc` can't be given to `action`: the menu name of the action that has it, or None if it's free.
pub(super) fn taken(keys: &[(KeyboardShortcut, Action)], sc: KeyboardShortcut, action: &Action) -> Option<String> {
    if RESERVED.contains(&sc) {
        return Some(tr("Copy, Cut, Paste, Enter, Esc and Delete").to_string());
    }
    let (_, other) = keys.iter().find(|(k, a)| *k == sc && a != action)?;
    Some(bindable().into_iter().find(|(a, _)| a == other).map_or_else(|| format!("{other:?}"), |(_, name)| tr(name).to_string()))
}

impl App {
    pub(super) fn menu_bar(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let loaded = self.current().and_then(|p| self.cache.get(p));
        let has_image = loaded.is_some();
        // shortcut: animated images can't be saved; add GIF re-encoding if it's wanted.
        let can_save = loaded.is_some_and(|l| l.frames.len() == 1);
        let keys = shortcuts(&self.settings.shortcuts);
        // A menu button, with the action's shortcut on the right.
        let button = |ui: &egui::Ui, label: &str, action: &Action| {
            let button = egui::Button::new(label);
            match keys.iter().find(|(_, a)| a == action) {
                Some((sc, _)) => button.shortcut_text(ui.ctx().format_shortcut(sc)),
                None => button,
            }
        };
        // `Button::selectable` with the shortcut.
        let selectable = |ui: &egui::Ui, label: &str, action: &Action, on: bool| button(ui, label, action).selected(on).frame_when_inactive(on).frame(true);
        let item = |ui: &mut egui::Ui, label: &str, action: Action, enabled: bool, actions: &mut Vec<Action>| {
            if ui.add_enabled(enabled, button(ui, label, &action)).clicked() {
                actions.push(action);
            }
        };
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
                item(ui, tr("Save Copy"), Action::SaveCopy, self.can_save_copy(), actions);
                item(ui, tr("Save As…"), Action::SaveAs, can_save, actions);
                ui.separator();
                let open = self.current().is_some();
                item(ui, tr("Copy Path"), Action::CopyPath, open, actions);
                item(ui, tr("Copy File"), Action::CopyFile, open, actions);
                item(ui, tr("Show in File Manager"), Action::ShowInFolder, open, actions);
                item(ui, tr("Rename…"), Action::Rename, open && self.restoring.is_none(), actions);
                let delete = button(ui, tr("Delete Image…"), &Action::DeleteImage);
                let delete = if keys.iter().any(|(_, a)| *a == Action::DeleteImage) { delete } else { delete.shortcut_text("Delete") };
                if ui.add_enabled(self.current().is_some(), delete).clicked() {
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
                item(ui, tr("First Image"), Action::FirstImage, self.index > 0, actions);
                item(ui, tr("Last Image"), Action::LastImage, self.index + 1 < self.files.len(), actions);
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
                let paged = loaded.is_some_and(|l| l.paged());
                let animated = loaded.is_some_and(|l| l.frames.len() > 1);
                if paged {
                    ui.menu_button(tr("Pages"), |ui| {
                        item(ui, tr("Next Page"), Action::NextFrame, true, actions);
                        item(ui, tr("Previous Page"), Action::PrevFrame, true, actions);
                    });
                }
                ui.add_enabled_ui(animated && !paged, |ui| {
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
                // Done in Adjust Color goes back to the edits from before the panel opened.
                item(ui, tr("Correct Perspective"), Action::Perspective, can_save && self.adjusting.is_none(), actions);
                item(ui, tr("Straighten…"), Action::Straighten, can_save && self.adjusting.is_none(), actions);
                ui.separator();
                item(ui, tr("Rotate Left"), Action::RotateLeft, has_image, actions);
                item(ui, tr("Rotate Right"), Action::RotateRight, has_image, actions);
                item(ui, tr("Flip Horizontally"), Action::FlipHorizontal, has_image, actions);
                item(ui, tr("Flip Vertically"), Action::FlipVertical, has_image, actions);
                ui.separator();
                item(ui, tr("Adjust Color…"), Action::AdjustColor, has_image, actions);
                item(ui, tr("Adjust Curves…"), Action::AdjustCurves, has_image, actions);
                item(ui, tr("Auto Levels"), Action::AutoLevels, has_image, actions);
                item(ui, tr("Reset Color Adjustments"), Action::ResetAdjust, has_image, actions);
                let adjust = self.doc().map(|d| d.adjust).unwrap_or_default();
                for (label, action, on) in [(tr("Greyscale"), Action::Greyscale, adjust.saturation == -1.0), (tr("Invert Colors"), Action::Invert, adjust.invert)] {
                    if ui.add_enabled(has_image && self.adjusting.is_none(), button(ui, label, &action).selected(on)).clicked() {
                        actions.push(action);
                    }
                }
                ui.separator();
                let patch = has_image && self.selection.is_some() && self.restoring.is_none();
                for (label, action, hint) in [
                    (tr("Remove Red Eye"), Action::RedEye, tr("Select the eyes first, then choose this.")),
                    (tr("Pixelate Selection"), Action::Pixelate, tr("Hides faces. For text, a solid shape is safer: pixelation can sometimes be undone.")),
                ] {
                    if ui.add_enabled(patch, button(ui, label, &action)).on_hover_text(hint).on_disabled_hover_text(hint).clicked() {
                        actions.push(action);
                    }
                }
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
                let hint = tr("Reads the selection, or the whole image. Straighten tilted text first, and select small text in a large image so it's read at full size.");
                if ui.add_enabled(ready(Task::Ocr) && !self.reading, button(ui, tr("Recognize Text"), &Action::ReadText(false))).on_hover_text(hint).on_disabled_hover_text(hint).clicked() {
                    actions.push(Action::ReadText(false));
                }
            });
            ui.menu_button(tr("Tools"), |ui| {
                let label = if self.show_toolbar { tr("Hide Drawing Toolbar") } else { tr("Show Drawing Toolbar") };
                item(ui, label, Action::ShowToolbar, has_image, actions);
                ui.separator();
                for (label, kind) in [(tr("Rectangular Selection"), SelKind::Rect), (tr("Elliptical Selection"), SelKind::Ellipse), (tr("Lasso"), SelKind::Lasso)] {
                    if ui.add_enabled(has_image, selectable(ui, label, &Action::SelectTool(kind), self.tool == Tool::Selection(kind))).clicked() {
                        actions.push(Action::SelectTool(kind));
                    }
                }
                let measuring = self.tool == Tool::Measure;
                if ui.add_enabled(has_image, selectable(ui, tr("Measure"), &Action::Measure, measuring)).clicked() {
                    actions.push(Action::Measure);
                }
                if ui.add_enabled(has_image, selectable(ui, tr("Copy Color"), &Action::CopyColor, self.tool == Tool::CopyColor)).clicked() {
                    actions.push(Action::CopyColor);
                }
                let hint = tr("Reads the selection, or the whole image. Shows what the code says; links are never opened.");
                if ui.add_enabled(has_image && !self.reading, button(ui, tr("Read QR Code"), &Action::ReadText(true))).on_hover_text(hint).on_disabled_hover_text(hint).clicked() {
                    actions.push(Action::ReadText(true));
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
