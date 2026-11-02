use super::App;
use crate::edit;
use crate::i18n::{tr, trf};
use crate::load::Msg;
use crate::markup::Selection;
use eframe::egui::{self, Pos2, Rect, TextureHandle, Vec2};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// A Denoise / Enhance run in progress.
pub(super) struct Restoring {
    /// Tells this run's messages from those of a cancelled run still winding down.
    pub(super) id: u64,
    pub(super) task: crate::ai::Task,
    pub(super) progress: f32,
    pub(super) started: Instant,
    pub(super) cancel: Arc<AtomicBool>,
    /// Applying to a selection: its outline, to mask the result with.
    pub(super) selection: Option<Selection>,
}

/// The before/after preview of Denoise / Enhance on part of the image.
pub(super) struct Compare {
    pub(super) task: crate::ai::Task,
    /// Output px.
    pub(super) region: Rect,
    pub(super) texture: TextureHandle,
    /// Where the before/after line is, 0–1 across the region.
    pub(super) split: f32,
    pub(super) ms_per_mp: f64,
    /// How many times bigger the model makes the image (an upscaler), else 1.
    pub(super) scale: u32,
}

impl App {
    /// Runs background removal on the edited image on another thread, cropping to the subject when `crop`.
    pub(super) fn cutout(&mut self, crop: bool) {
        let Some((l, doc, path)) = self.current().and_then(|p| Some((self.cache.get(p)?.clone(), self.doc()?, p.clone()))) else { return };
        let task = crate::ai::Task::Background;
        let Some(file) = crate::ai::choose(self.settings.ai.choice(task), task, &crate::ai::installed()) else { return };
        let preset = crate::ai::preset(&file, &self.settings.ai.overrides);
        self.set_status(trf("Running {file}…", &[("file", &file)]));
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            let result = edit::render(&l.frames[0], &doc).and_then(|img| {
                let img = img.into_rgba8();
                let pre = match preset {
                    Some(p) => p,
                    None => crate::ai::Preprocess::auto(crate::ai::input_size(&file)?),
                };
                let mask = crate::ai::mask(&file, &pre, &img)?;
                let object = if crop { Some(crate::ai::bbox(&mask).ok_or(tr("no subject found"))?) } else { None };
                Ok((crate::ai::cutout(&img, &mask), object))
            });
            notify.send(Msg::Cutout(path, result));
        });
    }

    /// Recognize Text, or Read QR Code when `qr`, on the selection's bounds (or the whole edited
    /// image) on another thread.
    // shortcut: no progress or Cancel (≈ 2 s for a page on the CPU); add them if big images take long.
    pub(super) fn read_text(&mut self, qr: bool) {
        let Some((l, doc)) = self.current().and_then(|p| Some((self.cache.get(p)?.clone(), self.doc()?))) else { return };
        let task = crate::ai::Task::Ocr;
        let file = if qr { None } else { Some(crate::ai::choose(self.settings.ai.choice(task), task, &crate::ai::installed())) };
        if self.reading || file == Some(None) {
            return;
        }
        let region = self.selection.as_ref().filter(|s| !s.is_empty()).map(|s| Rect::from_min_max(s.bbox().min.floor(), s.bbox().max.ceil()).intersect(doc.bounds())).filter(|r| r.width() >= 1.0 && r.height() >= 1.0);
        self.reading = true;
        self.set_status(if qr { tr("Reading QR codes…") } else { tr("Recognizing text…") });
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            let result = edit::render(&l.frames[0], &doc).and_then(|img| {
                let mut img = img.into_rgba8();
                if let Some(r) = region {
                    img = image::imageops::crop_imm(&img, r.min.x as u32, r.min.y as u32, r.width() as u32, r.height() as u32).to_image();
                }
                match file.flatten() {
                    Some(file) => crate::ai::read_text(&file, &img),
                    None => Ok(edit::read_qr(&img).join("\n")),
                }
            });
            notify.send(Msg::TextFound(qr, result));
        });
    }

    /// Denoise / Enhance on the part of the image on screen (at most 256 px square, so it takes
    /// seconds), for the before/after preview.
    pub(super) fn preview_restore(&mut self, task: crate::ai::Task) {
        let (Some(vis), Some(doc)) = (self.visible, self.doc()) else { return };
        let size = vis.size().min(Vec2::splat(256.0));
        let region = Rect::from_center_size(vis.center(), size);
        let region = Rect::from_min_max(region.min.round(), region.max.round()).intersect(doc.bounds());
        self.compare = None;
        self.run_restore(task, region, false, None);
    }

    /// Runs Denoise / Enhance on `region` (output px) of the edited image on another thread.
    fn run_restore(&mut self, task: crate::ai::Task, region: Rect, whole: bool, selection: Option<Selection>) {
        let Some((l, doc, path)) = self.current().and_then(|p| Some((self.cache.get(p)?.clone(), self.doc()?, p.clone()))) else { return };
        let Some(file) = crate::ai::choose(self.settings.ai.choice(task), task, &crate::ai::installed()) else { return };
        if region.width() < 1.0 || region.height() < 1.0 || self.restoring.is_some() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.restore_runs += 1;
        let id = self.restore_runs;
        self.restoring = Some(Restoring { id, task, progress: 0.0, started: Instant::now(), cancel: cancel.clone(), selection });
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = edit::render(&l.frames[0], &doc).and_then(|img| {
                let img = img.into_rgba8();
                let part = image::imageops::crop_imm(&img, region.min.x as u32, region.min.y as u32, region.width() as u32, region.height() as u32).to_image();
                let progress = |p: f32| notify.send(Msg::Progress(id, p));
                let out = crate::ai::restore(&file, &part, &progress, &cancel)?;
                let mp = (out.width() * out.height()) as f64 / 1e6;
                Ok((out, started.elapsed().as_secs_f64() * 1000.0 / mp))
            });
            notify.send(Msg::Restored(id, path, region, whole, result));
        });
    }

    /// Remove Object: fills in the selection on another thread. No preview: Ctrl+Z undoes it.
    pub(super) fn inpaint(&mut self) {
        let task = crate::ai::Task::Inpaint;
        let Some((l, doc, path)) = self.current().and_then(|p| Some((self.cache.get(p)?.clone(), self.doc()?, p.clone()))) else { return };
        let Some(file) = crate::ai::choose(self.settings.ai.choice(task), task, &crate::ai::installed()) else { return };
        let Some(sel) = self.selection.clone().filter(|s| !s.is_empty() && self.restoring.is_none()) else { return };
        let region = Rect::from_min_max(sel.bbox().min.floor(), sel.bbox().max.ceil()).intersect(doc.bounds());
        if region.width() < 1.0 || region.height() < 1.0 {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.restore_runs += 1;
        let id = self.restore_runs;
        self.restoring = Some(Restoring { id, task, progress: 0.0, started: Instant::now(), cancel: cancel.clone(), selection: Some(sel.clone()) });
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            let (w, h) = (region.width() as u32, region.height() as u32);
            let result = edit::render(&l.frames[0], &doc).and_then(|img| {
                let img = img.into_rgba8();
                let hole = crate::markup::polygon_mask(&sel.outline(), None, region.min, 1.0, w, h).and_then(|m| image::GrayImage::from_raw(w, h, m.data().to_vec())).ok_or(tr("the selection is empty"))?;
                Ok((crate::ai::inpaint(&file, &img, region, &hole, &cancel)?, 0.0))
            });
            notify.send(Msg::Restored(id, path, region, true, result));
        });
    }

    /// The bottom bar while Denoise / Enhance runs: progress, time left, Cancel.
    pub(super) fn restore_progress(&mut self, ctx: &egui::Context, viewport: Rect) {
        let Some(r) = &self.restoring else { return };
        let left = (r.progress > 0.02).then(|| r.started.elapsed().as_secs_f64() * (1.0 - r.progress as f64) / r.progress as f64);
        let name = if crate::ai::on_gpu() { trf("{task} on the GPU", &[("task", &r.task.title())]) } else { r.task.title().to_string() };
        let left = left.map_or(String::new(), |s| format!(" · {}", trf("about {time} left", &[("time", &format_duration(s))])));
        let label = format!("{name}… {:.0} %{left}", r.progress * 100.0);
        let cancel = egui::Area::new(egui::Id::unique("restore-progress")).fixed_pos(Pos2::new(viewport.center().x - 200.0, viewport.bottom() - 64.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(400.0);
                ui.label(label);
                ui.horizontal(|ui| {
                    ui.add(egui::ProgressBar::new(r.progress).desired_width(320.0));
                    ui.button(tr("Cancel")).clicked()
                }).inner
            }).inner
        }).inner;
        if cancel {
            r.cancel.store(true, Ordering::Relaxed);
        }
        // The time left updates even between tiles.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }

    /// The bar over the before/after preview: Apply to Whole Image / Selection, Discard.
    pub(super) fn compare_bar(&mut self, ctx: &egui::Context, viewport: Rect) {
        let Some(c) = &self.compare else { return };
        let Some(doc) = self.doc() else { return };
        let mp = doc.size_vec().x as f64 * doc.size_vec().y as f64 / 1e6;
        // shortcut: overlapping tiles do ~1.3–1.4× the image's pixels on a 12 MP photo; count the tiles if estimates are off.
        let estimate = format_duration(c.ms_per_mp * mp * 1.4 / 1000.0);
        // A bigger patch doesn't fit back into the image, so upscalers only do the whole image.
        let selection = self.selection.clone().filter(|s| !s.is_empty() && c.scale == 1);
        let result = if c.scale > 1 { format!(", {} × {} px", doc.size[0] * c.scale, doc.size[1] * c.scale) } else { String::new() };
        let (whole, sel, discard) = egui::Area::new(egui::Id::unique("compare-bar")).fixed_pos(Pos2::new(viewport.center().x - 260.0, viewport.top() + 12.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(trf("{task} preview of the visible area · drag to compare · Before | After", &[("task", &c.task.title())]));
                ui.horizontal(|ui| {
                    let whole = ui.button(trf("Apply to Whole Image{size} (about {estimate})", &[("size", &result), ("estimate", &estimate)])).clicked();
                    let sel = selection.is_some() && ui.button(tr("Apply to Selection")).clicked();
                    (whole, sel, ui.button(tr("Discard")).clicked())
                }).inner
            }).inner
        }).inner;
        let task = c.task;
        if whole || sel || discard {
            self.compare = None;
        }
        if whole {
            self.run_restore(task, doc.bounds(), true, None);
        } else if sel && let Some(s) = selection {
            let b = Rect::from_min_max(s.bbox().min.floor(), s.bbox().max.ceil()).intersect(doc.bounds());
            self.run_restore(task, b, true, Some(s));
        }
    }

    /// The Enhance model about to be used, if its notice hasn't been shown yet.
    pub(super) fn unseen_notice(&self) -> Option<String> {
        let task = crate::ai::Task::Enhance;
        crate::ai::choose(self.settings.ai.choice(task), task, &crate::ai::installed()).filter(|f| !self.settings.ai.notices_seen.contains(f))
    }

    /// Shown before an Enhance model is first used.
    pub(super) fn enhance_notice_dialog(&mut self, ctx: &egui::Context) {
        let Some(file) = self.enhance_notice.clone() else { return };
        let modal = egui::Modal::new(egui::Id::unique("enhance-notice")).show(ctx, |ui| {
            ui.heading(tr("Enhance Image"));
            for line in crate::ai::enhance_notice(&file) {
                ui.label(line);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| (ui.button(tr("Continue")).clicked(), ui.button(tr("Cancel")).clicked())).inner
        });
        let (go, cancel) = modal.inner;
        if go {
            self.settings.ai.notices_seen.push(file);
            self.enhance_notice = None;
            self.preview_restore(crate::ai::Task::Enhance);
        } else if cancel || modal.should_close() {
            self.enhance_notice = None;
        }
    }
}

/// "40 s", "3 min", "1 h 5 min".
fn format_duration(secs: f64) -> String {
    let secs = secs.max(1.0).round() as u64;
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs.div_ceil(60)),
        _ => format!("{} h {} min", secs / 3600, secs % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(format_duration(0.2), "1 s");
        assert_eq!(format_duration(42.4), "42 s");
        assert_eq!(format_duration(61.0), "2 min");
        assert_eq!(format_duration(3725.0), "1 h 2 min");
    }
}
