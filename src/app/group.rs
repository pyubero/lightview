//! Tools → Group Duplicates in Folder…: the dialog, and what Move and its undo do to the viewer.

use super::{App, Removed};
use crate::dedupe::{self, Moved};
use crate::files::Stamp;
use crate::i18n::{tr, trf};
use crate::load;
use eframe::egui::{self, Color32, Rect, Sense, TextureHandle, TextureOptions, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// The histogram's last bar holds every distance above this.
const BARS: u32 = 32;

pub(super) struct Grouping {
    pub(super) run: u64,
    files: Vec<PathBuf>,
    hashed: Vec<Option<(u64, Stamp)>>,
    /// Images that couldn't be decoded, with why.
    failed: Vec<(usize, String)>,
    done: usize,
    thumbs: Vec<Option<egui::ColorImage>>,
    /// Uploaded once a thumbnail is first on screen; dropped with the dialog.
    textures: HashMap<usize, TextureHandle>,
    cancel: Arc<AtomicBool>,
    /// When comparing started, for the time left.
    started: Instant,
    /// Filled in once every image is compared.
    pub(super) review: Option<Review>,
}

pub(super) struct Review {
    nearest: Vec<Option<u32>>,
    suggested: u32,
    threshold: u32,
    /// Smaller groups stay where they are.
    pub(super) min_size: usize,
    /// The groups, and the threshold and smallest size they were made with.
    pub(super) groups: ((u32, usize), Vec<Vec<usize>>),
}

impl Grouping {
    pub(super) fn add(&mut self, i: usize, result: Result<dedupe::Hashed, String>) {
        match result {
            Ok((hash, stamp, thumb)) => (self.hashed[i], self.thumbs[i]) = (Some((hash, stamp)), Some(thumb)),
            Err(e) => self.failed.push((i, e)),
        }
        self.done += 1;
        if self.done == self.files.len() {
            let hashes = self.hashes();
            let nearest = dedupe::nearest(&hashes);
            let suggested = dedupe::suggest(&nearest);
            self.failed.sort_by_key(|(i, _)| *i);
            self.review = Some(Review { groups: ((suggested, 2), dedupe::group(&hashes, suggested, 2)), nearest, suggested, threshold: suggested, min_size: 2 });
        }
    }

    fn hashes(&self) -> Vec<Option<u64>> {
        self.hashed.iter().map(|h| h.map(|(h, _)| h)).collect()
    }

    fn name(&self, i: usize) -> String {
        self.files[i].file_name().unwrap_or_default().to_string_lossy().into_owned()
    }
}

impl Drop for Grouping {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// "About … left" from the pace so far; nothing until a few images and a second have gone by.
pub(super) fn time_left(elapsed: f64, done: usize, total: usize) -> Option<String> {
    if done < 3 || elapsed < 1.0 || done >= total {
        return None;
    }
    let secs = (elapsed / done as f64 * (total - done) as f64).ceil() as u64;
    Some(if secs < 60 {
        trf("About {s} s left", &[("s", &secs.max(1).to_string())])
    } else {
        trf("About {m} min left", &[("m", &secs.div_ceil(60).to_string())])
    })
}

/// What the threshold means, in words.
fn meaning(t: u32) -> &'static str {
    match t {
        0 => tr("Identical pixels only"),
        1..=4 => tr("Resized or recompressed copies"),
        5..=10 => tr("Small edits, crops and bursts"),
        _ => tr("Merely similar scenes too"),
    }
}

impl App {
    /// Starts comparing the folder's images on other threads; the dialog shows the progress.
    pub(super) fn group_duplicates(&mut self) {
        self.group_runs += 1;
        let files = self.files.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        dedupe::hash_all(files.clone(), self.group_runs, self.settings.color_managed, cancel.clone(), self.notify.clone());
        let n = files.len();
        self.grouping = Some(Grouping { run: self.group_runs, files, hashed: vec![None; n], failed: Vec::new(), done: 0, thumbs: vec![None; n], textures: HashMap::new(), cancel, started: Instant::now(), review: None });
    }

    pub(super) fn group_dialog(&mut self, ctx: &egui::Context) {
        let Some(g) = self.grouping.as_mut() else { return };
        let (mut close, mut apply, mut show) = (false, false, None);
        let modal = egui::Modal::new(egui::Id::unique("group-duplicates")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(tr("Group Duplicates in Folder"));
            let Some(r) = g.review.as_mut() else {
                ui.label(trf("Comparing images: {done} of {total}", &[("done", &g.done.to_string()), ("total", &g.files.len().to_string())]));
                ui.add(egui::ProgressBar::new(g.done as f32 / g.files.len().max(1) as f32));
                if let Some(left) = time_left(g.started.elapsed().as_secs_f64(), g.done, g.files.len()) {
                    ui.label(left);
                }
                // Keeps the estimate ticking between images that take long.
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                ui.add_space(6.0);
                close = ui.button(tr("Cancel")).clicked();
                return;
            };
            // Each image's distance to its closest look-alike: duplicates pile up on the left.
            let mut bars = [0u32; BARS as usize + 1];
            for d in r.nearest.iter().flatten() {
                bars[(*d).min(BARS) as usize] += 1;
            }
            ui.label(tr("How different each image is from its closest look-alike:"));
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 80.0), Sense::hover());
            let top = *bars.iter().max().unwrap_or(&1).max(&1) as f32;
            let w = rect.width() / bars.len() as f32;
            let visuals = ui.visuals();
            for (d, &n) in bars.iter().enumerate() {
                let h = if n == 0 { 0.0 } else { (n as f32 / top).sqrt().max(0.05) * rect.height() };
                let x = rect.left() + d as f32 * w;
                let bar = Rect::from_min_max(egui::pos2(x + 1.0, rect.bottom() - h), egui::pos2(x + w - 1.0, rect.bottom()));
                let colour = if d as u32 <= r.threshold { visuals.selection.bg_fill } else { visuals.weak_text_color() };
                ui.painter().rect_filled(bar, 1.0, colour);
            }
            let x = rect.left() + (r.suggested as f32 + 0.5) * w;
            ui.painter().vline(x, rect.y_range(), egui::Stroke::new(1.0, visuals.strong_text_color()));
            ui.horizontal(|ui| {
                ui.label("0");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(format!("> {BARS}")));
            });
            ui.add(egui::Slider::new(&mut r.threshold, 0..=20).text(tr("Largest difference")));
            ui.label(trf("{meaning} · suggested: {n}", &[("meaning", &meaning(r.threshold)), ("n", &r.suggested.to_string())]));
            ui.add(egui::Slider::new(&mut r.min_size, 2..=50).text(tr("Smallest group"))).on_hover_text(tr("Groups with fewer images stay where they are."));
            if r.groups.0 != (r.threshold, r.min_size) {
                r.groups = ((r.threshold, r.min_size), dedupe::group(&g.hashed.iter().map(|h| h.map(|(h, _)| h)).collect::<Vec<_>>(), r.threshold, r.min_size));
            }
            // A copy, so the thumbnails below can borrow the rest of `g`.
            let groups = r.groups.1.clone();
            let moving: usize = groups.iter().map(Vec::len).sum();
            let mut summary = trf("Groups: {groups} · Images to move: {moving} · Staying: {staying}", &[("groups", &groups.len().to_string()), ("moving", &moving.to_string()), ("staying", &(g.files.len() - moving - g.failed.len()).to_string())]);
            if !g.failed.is_empty() {
                summary = format!("{summary} · {}", trf("Not compared: {n}", &[("n", &g.failed.len().to_string())]));
            }
            ui.strong(summary);
            ui.separator();
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                for group in &groups {
                    let folder = g.files[group[0]].file_stem().unwrap_or_default().to_string_lossy().into_owned();
                    ui.label(format!("📁 {folder}"));
                    ui.horizontal_wrapped(|ui| {
                        for &i in group {
                            let closest = group.iter().filter(|&&j| j != i).filter_map(|&j| Some(dedupe::distance(g.hashed[i]?.0, g.hashed[j]?.0))).min().unwrap_or(0);
                            let (rect, response) = ui.allocate_exact_size(Vec2::splat(96.0), Sense::click());
                            if ui.is_rect_visible(rect) {
                                let tex = g.textures.entry(i).or_insert_with(|| {
                                    let img = g.thumbs[i].take().unwrap_or_else(|| egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]));
                                    ui.ctx().load_texture(format!("group-{i}"), img, TextureOptions::LINEAR)
                                });
                                let fit = Rect::from_center_size(rect.center(), tex.size_vec2() * (96.0 / tex.size_vec2().max_elem()).min(1.0));
                                ui.painter().image(tex.id(), fit, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            let tip = trf("{name} · difference: {d}", &[("name", &g.name(i)), ("d", &closest.to_string())]);
                            if response.on_hover_text(tip).clicked() {
                                show = Some(g.files[i].clone());
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
                if !g.failed.is_empty() {
                    ui.collapsing(tr("Not compared"), |ui| {
                        for (i, e) in &g.failed {
                            ui.label(format!("{}: {e}", g.name(*i)));
                        }
                    });
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                apply = ui.add_enabled(!groups.is_empty(), egui::Button::new(tr("Move"))).clicked();
                close = ui.button(tr("Cancel")).clicked();
            });
        });
        if let Some(path) = show
            && let Some(i) = self.files.iter().position(|p| *p == path)
        {
            self.go_to(i);
        }
        if apply {
            self.move_groups();
        } else if close || modal.should_close() {
            self.grouping = None;
        }
    }

    /// Moves the groups the dialog shows, then shows what's left of the folder.
    pub(super) fn move_groups(&mut self) {
        let Some(g) = self.grouping.take() else { return };
        let Some(r) = &g.review else { return };
        let groups: Vec<Vec<(PathBuf, Stamp)>> = r.groups.1.iter().map(|group| group.iter().filter_map(|&i| Some((g.files[i].clone(), g.hashed[i]?.1))).collect()).collect();
        let moved = dedupe::move_groups(&groups);
        self.forget_moved(&moved.moves);
        let mut status = trf("Images moved: {n} · Folders: {f}", &[("n", &moved.moves.len().to_string()), ("f", &moved.created.len().to_string())]);
        if !moved.skipped.is_empty() || moved.error.is_some() {
            let mut lines = moved.skipped.clone();
            if let Some(e) = &moved.error {
                let stopped = trf("Stopped: {e}", &[("e", e)]);
                status = format!("{status} · {stopped}");
                lines.push(stopped);
            }
            self.found_text = Some((tr("Group Duplicates in Folder"), lines.join("\n")));
        }
        self.set_status(status);
        if !moved.created.is_empty() {
            self.deleted.push(Removed::Grouped(moved));
        }
    }

    /// Drops moved files from the folder: the viewer stays in it, on the next image left (or the
    /// previous, at the end), as after Delete.
    fn forget_moved(&mut self, moves: &[(PathBuf, PathBuf)]) {
        let gone: HashSet<&PathBuf> = moves.iter().map(|(from, _)| from).collect();
        let shown_moved = self.current().is_some_and(|p| gone.contains(p));
        let before = self.files[..self.index.min(self.files.len())].iter().filter(|p| !gone.contains(p)).count();
        self.files.retain(|p| !gone.contains(p));
        self.index = before.min(self.files.len().saturating_sub(1));
        for (from, to) in moves {
            self.cache.remove(from);
            if let Some(entry) = self.recent.iter_mut().find(|p| *p == from) {
                *entry = to.clone();
            }
        }
        if shown_moved {
            self.reset_shown();
        }
        if let Some(p) = self.current() {
            load::relist(p, self.settings.sort, &self.notify);
        }
        self.request_decodes();
    }

    /// Ctrl+Z after a Move: everything goes back, and the folder is listed again.
    pub(super) fn ungroup(&mut self, moved: &Moved) {
        let problems = dedupe::undo(moved);
        for (from, to) in &moved.moves {
            if let Some(entry) = self.recent.iter_mut().find(|p| *p == to) {
                *entry = from.clone();
            }
        }
        match self.current() {
            Some(p) => load::relist(p, self.settings.sort, &self.notify),
            None => {
                if let Some((from, _)) = moved.moves.first() {
                    self.open(from.clone());
                }
            }
        }
        if problems.is_empty() {
            self.set_status(trf("Images moved back: {n}", &[("n", &moved.moves.len().to_string())]));
        } else {
            self.found_text = Some((tr("Group Duplicates in Folder"), problems.join("\n")));
        }
    }
}
