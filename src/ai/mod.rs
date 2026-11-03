//! AI background removal (optionally cropped to the subject). ONNX Runtime and the models aren't
//! shipped: Local AI → Download AI Runtime… / Download … Model… install them on request.
use crate::i18n::{tr, tr_key, trf};
use crate::settings::AiSettings;
use eframe::egui::{self, Pos2, Rect};
use image::{DynamicImage, GrayImage, RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

mod catalog;
mod infer;
mod ocr;
mod runtime;
mod segment;
mod ui;

pub use catalog::*;
pub use infer::*;
pub use ocr::*;
pub use runtime::*;
pub use segment::*;
pub use ui::*;

fn data_dir() -> Result<PathBuf, String> {
    eframe::storage_dir("lightview").ok_or_else(|| tr("no data folder").to_string())
}

/// What a model is for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Task {
    /// Remove Background (and Crop): a mask.
    Background,
    /// Image to image, faithful.
    Denoise,
    /// Image to image, generative.
    Enhance,
    /// Fills in a selected area.
    Inpaint,
    /// Finds and reads text: a recogniser, with the detector and character list as its `data`.
    Ocr,
    /// Select Object: an image encoder, with its mask decoder as `data`.
    Segment,
}

impl Task {
    pub const ALL: [Task; 6] = [Task::Background, Task::Denoise, Task::Enhance, Task::Inpaint, Task::Ocr, Task::Segment];

    pub fn title(self) -> &'static str {
        match self {
            Task::Background => tr("Remove Background"),
            Task::Denoise => tr("Denoise"),
            Task::Enhance => tr("Enhance Image"),
            Task::Inpaint => tr("Remove Object"),
            Task::Ocr => tr("Recognize Text"),
            Task::Segment => tr("Select Object"),
        }
    }
}

#[cfg(test)]
mod tests;
