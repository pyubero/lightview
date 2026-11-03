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
    /// Blur Background: asks for the strength first.
    BlurBackground,
    /// Replace Background: with the background colour setting.
    ReplaceBackground,
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
        (Action::BlurBackground, tr_key("Blur Background…")),
        (Action::ReplaceBackground, tr_key("Replace Background")),
        (Action::Restore(Task::Denoise), tr_key("Denoise")),
        (Action::Restore(Task::Enhance), tr_key("Enhance Image")),
        (Action::Inpaint, tr_key("Remove Object")),
        (Action::ReadText(false), tr_key("Recognize Text")),
        (Action::AiModels, tr_key("AI Models…")),
        (Action::DownloadRuntime, tr_key("Download AI Runtime…")),
    ];
    let mut tools = vec![
        (Action::ShowToolbar, tr_key("Show Drawing Toolbar")),
        (Action::SelectTool(SelKind::Rect), tr_key("Rectangular Selection")),
        (Action::SelectTool(SelKind::Ellipse), tr_key("Elliptical Selection")),
        (Action::SelectTool(SelKind::Lasso), tr_key("Lasso")),
        (Action::SelectTool(SelKind::Object), tr_key("Select Object (AI)")),
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
    /// Whether `action` can run now: its menu item is greyed out otherwise, and its shortcut does nothing.
    pub(super) fn enabled(&self, action: &Action) -> bool {
        use crate::ai::Task;
        let loaded = self.current().and_then(|p| self.cache.get(p));
        let has_image = loaded.is_some();
        // shortcut: animated images can't be saved; add GIF re-encoding if it's wanted.
        let can_save = loaded.is_some_and(|l| l.frames.len() == 1);
        let paged = loaded.is_some_and(|l| l.paged());
        let animated = loaded.is_some_and(|l| l.frames.len() > 1) && !paged;
        let selected = self.shown.selection.is_some();
        let idle = self.restoring.is_none();
        let ready = |task| has_image && idle && crate::ai::runtime_installed() && crate::ai::choose(self.settings.ai.choice(task), task, &crate::ai::installed()).is_some();
        match action {
            Action::Open | Action::OpenRecent(_) | Action::Close | Action::Settings | Action::Fullscreen | Action::AiModels | Action::DownloadRuntime | Action::Describe => true,
            Action::SaveCopy => self.can_save_copy(),
            Action::SaveAs => can_save,
            Action::CopyPath | Action::CopyFile | Action::ShowInFolder | Action::DeleteImage => self.current().is_some(),
            Action::Rename => self.current().is_some() && idle,
            Action::Undo => !(self.shown.undo.is_empty() && self.deleted.is_empty()),
            Action::Redo => !self.shown.redo.is_empty(),
            Action::Cut | Action::DeleteSelection | Action::Deselect => selected,
            Action::PrevImage | Action::FirstImage => self.index > 0,
            Action::NextImage | Action::LastImage => self.index + 1 < self.files.len(),
            Action::Play => animated && self.shown.anim_paused,
            Action::Pause => animated && !self.shown.anim_paused,
            Action::NextFrame | Action::PrevFrame => animated || paged,
            // Done in Adjust Color goes back to the edits from before the panel opened.
            Action::Perspective | Action::Straighten => can_save && self.shown.adjusting.is_none(),
            Action::Greyscale | Action::Invert => has_image && self.shown.adjusting.is_none(),
            Action::RedEye | Action::Pixelate => has_image && selected && idle,
            Action::Cutout(_) | Action::BlurBackground | Action::ReplaceBackground => ready(Task::Background),
            Action::Restore(task) => ready(*task),
            Action::Inpaint => ready(Task::Inpaint) && selected,
            Action::ReadText(false) => ready(Task::Ocr) && !self.reading,
            Action::ReadText(true) => has_image && !self.reading,
            Action::Copy | Action::Paste | Action::SelectAll | Action::Info | Action::ZoomIn | Action::ZoomOut | Action::Fit | Action::ActualSize => has_image,
            Action::Crop | Action::Resize | Action::RotateLeft | Action::RotateRight | Action::FlipHorizontal | Action::FlipVertical => has_image,
            Action::AdjustColor | Action::AdjustCurves | Action::AutoLevels | Action::ResetAdjust => has_image,
            Action::SelectTool(SelKind::Object) => ready(Task::Segment),
            Action::ShowToolbar | Action::SelectTool(_) | Action::Measure | Action::CopyColor | Action::Pick(_) => has_image,
        }
    }

    pub(super) fn menu_bar(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let keys = shortcuts(&self.settings.shortcuts);
        let names = bindable();
        let name = |action: &Action| tr(names.iter().find(|(a, _)| a == action).map_or("", |(_, name)| name));
        // A menu button, with the action's shortcut on the right.
        let button = |ui: &egui::Ui, label: &str, action: &Action| {
            let button = egui::Button::new(label);
            match keys.iter().find(|(_, a)| a == action) {
                Some((sc, _)) => button.shortcut_text(ui.ctx().format_shortcut(sc)),
                None => button,
            }
        };
        // `Button::selectable` with the shortcut.
        let selectable = |ui: &mut egui::Ui, action: Action, on: bool, actions: &mut Vec<Action>| {
            let hint = (action == Action::SelectTool(SelKind::Object)).then(|| tr("Drag a box around one object; its outline becomes the selection. Holes inside it stay selected."));
            let response = ui.add_enabled(self.enabled(&action), button(ui, name(&action), &action).selected(on).frame_when_inactive(on).frame(true));
            let response = match hint {
                Some(h) => response.on_hover_text(h).on_disabled_hover_text(h),
                None => response,
            };
            if response.clicked() {
                actions.push(action);
            }
        };
        let labelled = |ui: &mut egui::Ui, label: &str, action: Action, actions: &mut Vec<Action>| {
            if ui.add_enabled(self.enabled(&action), button(ui, label, &action)).clicked() {
                actions.push(action);
            }
        };
        let item = |ui: &mut egui::Ui, action: Action, actions: &mut Vec<Action>| labelled(ui, name(&action), action, actions);
        // An item whose hint shows even while it's greyed out.
        let hinted = |ui: &mut egui::Ui, action: Action, hint: &str, actions: &mut Vec<Action>| {
            if ui.add_enabled(self.enabled(&action), button(ui, name(&action), &action)).on_hover_text(hint).on_disabled_hover_text(hint).clicked() {
                actions.push(action);
            }
        };
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("File"), |ui| {
                item(ui, Action::Open, actions);
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
                item(ui, Action::Close, actions);
                ui.separator();
                item(ui, Action::SaveCopy, actions);
                item(ui, Action::SaveAs, actions);
                ui.separator();
                item(ui, Action::CopyPath, actions);
                item(ui, Action::CopyFile, actions);
                item(ui, Action::ShowInFolder, actions);
                item(ui, Action::Rename, actions);
                let delete = button(ui, name(&Action::DeleteImage), &Action::DeleteImage);
                let delete = if keys.iter().any(|(_, a)| *a == Action::DeleteImage) { delete } else { delete.shortcut_text("Delete") };
                if ui.add_enabled(self.enabled(&Action::DeleteImage), delete).clicked() {
                    actions.push(Action::DeleteImage);
                }
            });
            ui.menu_button(tr("Edit"), |ui| {
                item(ui, Action::Undo, actions);
                item(ui, Action::Redo, actions);
                ui.separator();
                // Ctrl+C/X/V come as clipboard events, not key presses.
                for (label, action, keys) in [(tr("Cut"), Action::Cut, "Ctrl+X"), (tr("Copy"), Action::Copy, "Ctrl+C"), (tr("Paste"), Action::Paste, "Ctrl+V")] {
                    if ui.add_enabled(self.enabled(&action), egui::Button::new(label).shortcut_text(keys)).clicked() {
                        actions.push(action);
                    }
                }
                item(ui, Action::DeleteSelection, actions);
                ui.separator();
                item(ui, Action::SelectAll, actions);
                item(ui, Action::Deselect, actions);
                ui.separator();
                item(ui, Action::Settings, actions);
            });
            ui.menu_button(tr("View"), |ui| {
                item(ui, Action::PrevImage, actions);
                item(ui, Action::NextImage, actions);
                item(ui, Action::FirstImage, actions);
                item(ui, Action::LastImage, actions);
                ui.separator();
                item(ui, Action::ZoomIn, actions);
                item(ui, Action::ZoomOut, actions);
                item(ui, Action::ActualSize, actions);
                item(ui, Action::Fit, actions);
                ui.separator();
                item(ui, Action::Fullscreen, actions);
                ui.separator();
                let label = if self.show_info { tr("Hide Image Information") } else { tr("Show Image Information") };
                labelled(ui, label, Action::Info, actions);
                ui.separator();
                let paged = self.current().and_then(|p| self.cache.get(p)).is_some_and(|l| l.paged());
                if paged {
                    ui.menu_button(tr("Pages"), |ui| {
                        labelled(ui, tr("Next Page"), Action::NextFrame, actions);
                        labelled(ui, tr("Previous Page"), Action::PrevFrame, actions);
                    });
                }
                ui.add_enabled_ui(self.enabled(&Action::NextFrame) && !paged, |ui| {
                    ui.menu_button(tr("Animation"), |ui| {
                        item(ui, Action::Play, actions);
                        item(ui, Action::Pause, actions);
                        item(ui, Action::NextFrame, actions);
                        item(ui, Action::PrevFrame, actions);
                    });
                });
            });
            ui.menu_button(tr("Image"), |ui| {
                item(ui, Action::Crop, actions);
                item(ui, Action::Resize, actions);
                item(ui, Action::Perspective, actions);
                item(ui, Action::Straighten, actions);
                ui.separator();
                item(ui, Action::RotateLeft, actions);
                item(ui, Action::RotateRight, actions);
                item(ui, Action::FlipHorizontal, actions);
                item(ui, Action::FlipVertical, actions);
                ui.separator();
                item(ui, Action::AdjustColor, actions);
                item(ui, Action::AdjustCurves, actions);
                item(ui, Action::AutoLevels, actions);
                item(ui, Action::ResetAdjust, actions);
                let adjust = self.doc().map(|d| d.adjust).unwrap_or_default();
                for (action, on) in [(Action::Greyscale, adjust.saturation == -1.0), (Action::Invert, adjust.invert)] {
                    if ui.add_enabled(self.enabled(&action), button(ui, name(&action), &action).selected(on)).clicked() {
                        actions.push(action);
                    }
                }
                ui.separator();
                hinted(ui, Action::RedEye, tr("Select the eyes first, then choose this."), actions);
                hinted(ui, Action::Pixelate, tr("Hides faces. For text, a solid shape is safer: pixelation can sometimes be undone."), actions);
                ui.separator();
                ui.weak(tr("AI Image Processing"));
                use crate::ai::Task;
                item(ui, Action::Cutout(false), actions);
                item(ui, Action::Cutout(true), actions);
                item(ui, Action::BlurBackground, actions);
                hinted(ui, Action::ReplaceBackground, tr("Fills the background with the background colour from Settings."), actions);
                item(ui, Action::Restore(Task::Denoise), actions);
                item(ui, Action::Restore(Task::Enhance), actions);
                item(ui, Action::Inpaint, actions);
                let hint = tr("Reads the selection, or the whole image. Straighten tilted text first, and select small text in a large image so it's read at full size.");
                hinted(ui, Action::ReadText(false), hint, actions);
                ui.separator();
                item(ui, Action::AiModels, actions);
                if !crate::ai::runtime_installed() {
                    item(ui, Action::DownloadRuntime, actions);
                }
            });
            ui.menu_button(tr("Tools"), |ui| {
                let label = if self.show_toolbar { tr("Hide Drawing Toolbar") } else { tr("Show Drawing Toolbar") };
                labelled(ui, label, Action::ShowToolbar, actions);
                ui.separator();
                for kind in [SelKind::Rect, SelKind::Ellipse, SelKind::Lasso, SelKind::Object] {
                    selectable(ui, Action::SelectTool(kind), self.shown.tool == Tool::Selection(kind), actions);
                }
                selectable(ui, Action::Measure, matches!(self.shown.tool, Tool::Measure { .. }), actions);
                selectable(ui, Action::CopyColor, self.shown.tool == Tool::CopyColor, actions);
                hinted(ui, Action::ReadText(true), tr("Reads the selection, or the whole image. Shows what the code says; links are never opened."), actions);
                ui.separator();
                item(ui, Action::Pick(Pick::Draw(Shape::Sketch)), actions);
                ui.add_enabled_ui(self.current().is_some_and(|p| self.cache.contains_key(p)), |ui| {
                    ui.menu_button(tr("Shapes"), |ui| {
                        for (shape, _) in markup::SHAPES {
                            item(ui, Action::Pick(Pick::Draw(shape)), actions);
                        }
                    });
                });
                item(ui, Action::Pick(Pick::Draw(Shape::Text)), actions);
                item(ui, Action::Pick(Pick::Draw(Shape::Highlight)), actions);
                item(ui, Action::Pick(Pick::Draw(Shape::Magnifier)), actions);
            });
        });
    }
}
