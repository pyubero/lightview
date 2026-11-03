use super::{App, Tool, ms};
use crate::edit::{self, Adjust, Doc};
use crate::i18n::{tr, trf};
use crate::load::{Frame, Loaded, Msg};
use crate::markup::{self, Element, Shape};
use crate::settings::Interpolation;
use crate::svg;
use crate::view::{self, Transform, View};
use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, TextureFilter, TextureOptions, Vec2};
use std::sync::Arc;

/// What a baked frame holds; it's rebuilt when this changes.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct BakeKey {
    generation: u64,
    anim_frame: usize,
    crop: Rect,
    size: [u32; 2],
}

/// The most pixels `flattened` renders: ~200 ms on the UI thread, once per finished change.
const FLAT_MAX_PIXELS: u64 = 16_000_000;

/// What the current texture holds; it's rebuilt when this changes.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct TexKey {
    /// The frame comes from `flattened`.
    pub(super) flat: bool,
    generation: u64,
    anim_frame: usize,
    /// GPU scaling uploads the whole frame, so transform/crop/size don't matter then.
    gpu: bool,
    interpolation: Interpolation,
    transform: Transform,
    crop: [f32; 4],
    size: [u32; 2],
    adjust: Adjust,
    baked: Option<BakeKey>,
}

impl App {
    /// The image with its markup at image resolution, as saving writes it, so the view is
    /// faithful on a low-res image. While a change is in progress the markup stays at screen
    /// resolution, because rendering every frame lags even at 1600×900.
    // shortcut: above the limit, and on animated images, markup stays at screen resolution; render on a thread if that's missed.
    pub(super) fn flattened(&mut self, full: Option<&Loaded>, doc: &Doc) -> Option<Arc<Frame>> {
        let live = self.shown.change_base.is_some() || self.shown.adjusting.is_some() || self.shown.editing_text.is_some();
        let fits = doc.size[0] as u64 * doc.size[1] as u64 <= FLAT_MAX_PIXELS;
        let Some(l) = full.filter(|l| l.frames.len() == 1 && fits && !live && !doc.elements.is_empty()) else {
            self.flat = None;
            return None;
        };
        if self.flat.as_ref().is_none_or(|(generation, done, _)| *generation != self.generation || done != doc) {
            let frame = edit::flat(&l.frames[0], doc).ok()?;
            self.generation += 1;
            self.flat = Some((self.generation, doc.clone(), Arc::new(frame)));
        }
        self.flat.as_ref().map(|f| f.2.clone())
    }

    /// The current SVG drawn for `zoom` (screen px per output px) and how many times bigger it
    /// is than 1×, when it's sharper than the frame. The latest render stays until a newer one
    /// arrives. Markup and shrinking show as saved, so they stay at 1×.
    // shortcut: renders of tiers already zoomed past still finish; cancel them if they hold up big SVGs.
    pub(super) fn sharp_frame(&mut self, full: Option<&Loaded>, doc: &Doc, zoom: f32) -> Option<(u32, Arc<Frame>)> {
        let l = full?;
        let tree = l.svg.as_ref()?;
        let plain = doc.elements.is_empty() && !doc.shrinks() && doc.base.is_none();
        let k = if plain { svg::tier(zoom * doc.scale().max_elem(), [l.frames[0].w, l.frames[0].h]) } else { 1 };
        if k != self.sharp_wanted {
            self.sharp_wanted = k;
            if k == 1 {
                self.sharp = None;
            } else {
                let (tree, path, notify) = (tree.clone(), self.current()?.clone(), self.notify.clone());
                std::thread::spawn(move || {
                    if let Some(frame) = svg::render(&tree, k) {
                        notify.send(Msg::Sharp(path, k, frame));
                    }
                });
            }
        }
        self.sharp.clone()
    }

    /// Draws the markup over the image (unless it's in the image already, `flat`), plus the
    /// selection handle and the text being typed.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_markup(&mut self, ui: &egui::Ui, doc: &Doc, flat: bool, dest: Rect, crop: Rect, texture: egui::TextureId, uv: &dyn Fn(Pos2) -> Pos2) {
        let k = dest.width() / crop.width();
        let to_screen = |p: Pos2| view::to_screen(dest, crop, p);
        let painter = ui.painter().with_clip_rect(dest);
        let mut used = Vec::new();
        let painted = if flat { &[][..] } else { &doc.elements[..] };
        for (i, e) in painted.iter().enumerate() {
            let own = e.texture_key().and_then(|key| {
                used.push(key);
                if let std::collections::hash_map::Entry::Vacant(slot) = self.shown.element_textures.entry(key) {
                    let image = e.texture_image(ui.ctx().input(|i| i.max_texture_side) as f32)?;
                    slot.insert(ui.ctx().load_texture("markup", image, TextureOptions::LINEAR));
                }
                Some(self.shown.element_textures[&key].id())
            });
            if Some(i) != self.shown.editing_text {
                markup::paint(&painter, e, &to_screen, k, texture, uv, own);
            }
        }
        self.shown.element_textures.retain(|key, _| used.contains(key));
        if self.shown.tool == Tool::Select && let Some(e) = self.shown.selected.and_then(|i| doc.elements.get(i)) {
            let r = Rect::from_two_pos(to_screen(e.bbox().min), to_screen(e.bbox().max)).expand(3.0);
            painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, Color32::from_rgb(66, 165, 245)), egui::StrokeKind::Outside);
            painter.rect_filled(Rect::from_center_size(r.max, Vec2::splat(9.0)), 1.0, Color32::from_rgb(66, 165, 245));
        }
        let Some(i) = self.shown.editing_text else { return };
        let Some(e) = doc.elements.get(i) else { return };
        let mut text = e.text.clone();
        let font = egui::FontId::proportional(e.style.font_size * k);
        let edit = egui::TextEdit::multiline(&mut text).font(font).text_color(e.style.color).frame(egui::Frame::NONE).desired_width(f32::INFINITY.min(dest.right() - to_screen(e.points[0]).x).max(40.0)).desired_rows(1);
        let response = egui::Area::new(egui::Id::unique("markup-text")).fixed_pos(to_screen(e.points[0])).order(egui::Order::Foreground).show(ui.ctx(), |ui| ui.add(edit)).inner;
        if std::mem::take(&mut self.focus_text) {
            response.request_focus();
        }
        if text != e.text && let Some(e) = self.doc_mut().and_then(|d| d.elements.get_mut(i)) {
            e.text = text;
        }
        if response.lost_focus() {
            self.finish_text();
        }
    }

    /// Draws the current image (or its preview, or the error) and everything over it.
    pub(super) fn draw(&mut self, ui: &egui::Ui, full: Option<&Loaded>, doc: Option<Doc>, hovered_at: Option<Pos2>, viewport: Rect) {
        let ctx = ui.ctx();
        let ppp = ctx.pixels_per_point();
        let preview = self.shown.preview.clone();
        let base = doc.as_ref().and_then(|d| d.base.clone());
        let (frame, view, doc) = match (full, doc) {
            (Some(l), Some(doc)) => (Some(base.as_ref().map_or(&l.frames[self.shown.anim_frame], |b| &*b.0)), self.view, doc),
            _ => (preview.as_deref(), View::Fit, preview.as_ref().map(|f| Doc::new(f.w, f.h)).unwrap_or(Doc::new(1, 1))),
        };
        // Flattened pixels hold every edit, so they're drawn as a plain image of the output's size.
        let edited = doc.clone();
        let flat = self.flattened(full, &doc);
        let (frame, doc) = match &flat {
            Some(f) => (Some(&**f), Doc::new(f.w, f.h)),
            None => (frame, doc),
        };
        // A shrunk image is shown from its resized pixels: the decoded ones still hold the lost detail.
        let bake_key = frame.filter(|_| doc.shrinks()).map(|_| BakeKey { generation: self.generation, anim_frame: self.shown.anim_frame, crop: doc.crop, size: doc.plain_size() });
        if let (Some(key), Some(frame)) = (bake_key, frame) && self.baked.as_ref().map(|b| b.0) != Some(key) {
            self.baked = Some((key, Arc::new(edit::bake(frame, &doc))));
        }
        let baked = bake_key.and_then(|_| self.baked.as_ref()).map(|b| b.1.clone());
        let sharp = self.sharp_frame(full, &edited, view::scale(view, edited.size_vec(), viewport, ppp));
        let frame = baked.as_deref().or(sharp.as_ref().map(|s| &*s.1)).or(frame);
        let source = baked.is_some().then(|| doc.on_baked()).or(sharp.as_ref().map(|s| doc.on_sharp(s.0)));
        let source = source.as_ref().unwrap_or(&doc);
        let Some(frame) = frame else {
            if let Some(e) = &self.error {
                ui.painter().text(viewport.center(), egui::Align2::CENTER_CENTER, trf("Can't open this image:\n{e}", &[("e", &e)]), egui::TextStyle::Body.resolve(ui.style()), Color32::GRAY);
            }
            return;
        };
        let frame_size = Vec2::new(frame.w as f32, frame.h as f32);
        let (dest, crop) = view::layout(view, doc.size_vec(), viewport, ppp);
        let interpolation = self.settings.interpolation();
        // Images bigger than the GPU's texture limit always take the CPU path.
        // shortcut: colour adjustments only run on the CPU path; add a shader if GPU users miss smooth zoom while adjusting.
        let gpu = self.settings.gpu_scaling && frame.w.max(frame.h) as usize <= ctx.input(|i| i.max_texture_side) && doc.adjust.is_identity();
        let key = if gpu {
            TexKey { flat: flat.is_some(), generation: self.generation, anim_frame: self.shown.anim_frame, gpu, interpolation, transform: Transform::default(), crop: [0.0; 4], size: [frame.w, frame.h], adjust: Adjust::default(), baked: bake_key }
        } else {
            let src = source.to_source(crop);
            let size = [(dest.width() * ppp).round().max(1.0) as u32, (dest.height() * ppp).round().max(1.0) as u32];
            TexKey { flat: flat.is_some(), generation: self.generation, anim_frame: self.shown.anim_frame, gpu, interpolation, transform: doc.transform, crop: [src.min.x, src.min.y, src.width(), src.height()], size, adjust: doc.adjust, baked: bake_key }
        };
        self.upload_texture(ctx, frame, key, &doc);
        let texture = self.texture.as_ref().unwrap().id();
        if frame.alpha {
            // Transparent parts show as a checkerboard, not as the background.
            let painter = ui.painter().with_clip_rect(dest.intersect(viewport));
            painter.add(markup::checker(dest.intersect(viewport), None));
        }
        // Straighten turns the drawn image about its centre: no pixel work while the slider moves.
        // shortcut: markup and the checkerboard stay upright meanwhile; turn them too if that's confusing.
        let centre = view::to_screen(dest, crop, doc.bounds().center());
        let turn = egui::emath::Rot2::from_angle(if let Tool::Straighten(degrees) = self.shown.tool { degrees.to_radians() } else { 0.0 });
        let mut mesh = egui::Mesh::with_texture(texture);
        let shown = [crop.left_top(), crop.right_top(), crop.right_bottom(), crop.left_bottom()];
        let screen = [dest.left_top(), dest.right_top(), dest.right_bottom(), dest.left_bottom()];
        let unit = [Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0)];
        for ((pos, p), unit) in screen.into_iter().zip(shown).zip(unit) {
            // The GPU texture is the whole frame: each corner samples the source point the transform maps it from.
            let uv = if gpu { (source.source_point(p).to_vec2() / frame_size).to_pos2() } else { unit };
            mesh.vertices.push(egui::epaint::Vertex { pos: centre + turn * (pos - centre), uv, color: Color32::WHITE });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        ui.painter().with_clip_rect(viewport).add(mesh);
        let frame_wh = [frame.w, frame.h];
        self.visible = full.is_some().then_some(crop);
        self.paint_compare(ui, dest, crop);
        // The magnifier samples the image texture: the whole frame (GPU) or the visible part (CPU).
        let uv = |p: Pos2| if gpu { (source.source_point(p).to_vec2() / frame_size).to_pos2() } else { ((p - crop.min) / crop.size()).to_pos2() };
        if !edited.elements.is_empty() || self.shown.editing_text.is_some() || !self.shown.element_textures.is_empty() {
            self.paint_markup(ui, &edited, flat.is_some(), dest, crop, texture, &uv);
        }
        if let Some(p) = hovered_at.filter(|_| self.shown.tool == Tool::Draw(Shape::Magnifier) && self.shown.compare.is_none()) && let Some(style) = self.shown.style {
            // The lens a click here would place.
            let size = doc.size_vec();
            let lens = Element::lens(Rect::from_min_size(Pos2::ZERO, size).clamp(view::to_image(dest, crop, p)), size, style);
            markup::paint(&ui.painter().with_clip_rect(dest), &lens, &|p| view::to_screen(dest, crop, p), dest.width() / crop.width(), texture, &uv, None);
        }
        if self.settings.pixel_grid {
            let painter = ui.painter().with_clip_rect(dest.intersect(viewport));
            // On pixel centres, or a line blurs across two screen pixels.
            let snap = |p: Pos2| Pos2::new(painter.round_to_pixel_center(p.x), painter.round_to_pixel_center(p.y));
            for [a, b] in view::grid_lines(dest, crop, ppp) {
                painter.line_segment([snap(a), snap(b)], egui::Stroke::new(1.0 / ppp, Color32::from_rgba_unmultiplied(128, 128, 128, 120)));
            }
        }
        self.paint_selection(ui, dest, crop);
        if let Tool::Crop(rect) = self.shown.tool {
            self.paint_crop(ui, rect, dest, crop, viewport);
        }
        if let Tool::Perspective { corners, .. } = self.shown.tool {
            self.paint_perspective(ui, corners, dest, crop, viewport);
        }
        if let Tool::Straighten(degrees) = self.shown.tool {
            self.paint_straighten(ui, degrees, doc.size_vec(), dest, crop, viewport);
        }
        if let Tool::Measure { ends, .. } = self.shown.tool {
            self.paint_measure(ui, ends, &edited, dest, crop, viewport);
        }
        if self.shown.tool == Tool::CopyColor {
            bubble(ui.painter(), Pos2::new(viewport.center().x, viewport.top() + 24.0), tr("Click a pixel to copy its colour · Esc to stop"));
        }
        if view::zoomed_in(crop, doc.size_vec()) {
            let scale = view::scale(view, doc.size_vec(), viewport, ppp);
            self.navigator(ui, frame, source, &doc, key, crop, scale, viewport);
        }
        if self.timings {
            self.report_timings(ctx, full.is_some(), frame_wh);
        }
    }

    /// The whole image small in a corner while zoomed in, with what's on screen marked; clicking
    /// or dragging on it moves there. `key` is the main texture's.
    // shortcut: an animation shows the frame it was first built from, and markup not flattened (over 16 MP, or during a change) is missing.
    #[allow(clippy::too_many_arguments)]
    fn navigator(&mut self, ui: &egui::Ui, frame: &Frame, source: &Doc, doc: &Doc, key: TexKey, crop: Rect, scale: f32, viewport: Rect) {
        let ctx = ui.ctx();
        let (img, ppp) = (doc.size_vec(), ctx.pixels_per_point());
        let rect = view::navigator_rect(img, viewport);
        let size = [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32];
        let src = source.to_source(doc.bounds());
        let interpolation = Interpolation::Bilinear;
        let key = TexKey { gpu: false, interpolation, transform: doc.transform, crop: [src.min.x, src.min.y, src.width(), src.height()], size, adjust: doc.adjust, anim_frame: 0, ..key };
        if self.navigator.as_ref().map(|n| n.0) != Some(key) {
            let mut pixels = resize(frame, src, size, doc.transform, interpolation);
            if !doc.adjust.is_identity() {
                doc.adjust.apply(&mut pixels, if frame.alpha { 4 } else { 3 });
            }
            let image = to_color_image(frame.alpha, [size[0] as usize, size[1] as usize], &pixels);
            self.navigator = Some((key, ctx.load_texture("navigator", image, TextureOptions::LINEAR)));
        }
        let Some((_, texture)) = &self.navigator else { return };
        let response = ui.interact(rect, ui.scope_id().with("navigator"), egui::Sense::click_and_drag());
        let painter = ui.painter().with_clip_rect(viewport);
        painter.rect_filled(rect.expand(1.0), 0.0, Color32::from_black_alpha(190));
        painter.image(texture.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let k = rect.size() / img;
        let shown = Rect::from_min_max(rect.min + crop.min.to_vec2() * k, rect.min + crop.max.to_vec2() * k);
        painter.rect_stroke(shown, 0.0, egui::Stroke::new(3.0, Color32::from_black_alpha(160)), egui::StrokeKind::Middle);
        painter.rect_stroke(shown, 0.0, egui::Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
        if response.is_pointer_button_down_on() && let Some(p) = response.interact_pointer_pos() {
            self.view = View::Manual { scale, center: ((p - rect.min) / k).to_pos2() };
        }
    }

    /// Makes the texture hold what `key` describes, if it doesn't yet.
    fn upload_texture(&mut self, ctx: &egui::Context, frame: &Frame, key: TexKey, doc: &Doc) {
        if self.tex_key == Some(key) {
            return;
        }
        let (gpu, interpolation) = (key.gpu, key.interpolation);
        let (image, options) = if gpu {
            let options = match interpolation {
                Interpolation::Nearest => TextureOptions::NEAREST,
                // Mipmaps keep big images from aliasing when shrunk.
                _ => TextureOptions { mipmap_mode: Some(TextureFilter::Linear), ..TextureOptions::LINEAR },
            };
            (to_color_image(frame.alpha, [frame.w as usize, frame.h as usize], &frame.pixels), options)
        } else {
            // shortcut: resize runs on the UI thread every pan/zoom frame; GPU scaling is the smooth path.
            let unadjusted = TexKey { adjust: Adjust::default(), ..key };
            if self.resized.as_ref().map(|r| r.0) != Some(unadjusted) {
                let [x, y, w, h] = key.crop;
                self.resized = Some((unadjusted, resize(frame, Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)), key.size, doc.transform, interpolation)));
            }
            let pixels = &self.resized.as_ref().unwrap().1;
            let size = [key.size[0] as usize, key.size[1] as usize];
            let image = if doc.adjust.is_identity() {
                to_color_image(frame.alpha, size, pixels)
            } else {
                // Only the adjustment re-runs while a slider moves.
                let mut pixels = pixels.clone();
                doc.adjust.apply(&mut pixels, if frame.alpha { 4 } else { 3 });
                to_color_image(frame.alpha, size, &pixels)
            };
            (image, TextureOptions::LINEAR)
        };
        match &mut self.texture {
            Some(t) => t.set(image, options),
            None => self.texture = Some(ctx.load_texture("image", image, options)),
        }
        self.tex_key = Some(key);
    }

    /// The result on the right of the line, the original on the left.
    fn paint_compare(&self, ui: &egui::Ui, dest: Rect, crop: Rect) {
        let Some(c) = &self.shown.compare else { return };
        let r = Rect::from_two_pos(view::to_screen(dest, crop, c.region.min), view::to_screen(dest, crop, c.region.max));
        let x = r.left() + r.width() * c.split;
        let after = Rect::from_min_max(Pos2::new(x, r.top()), r.max);
        ui.painter().with_clip_rect(after.intersect(dest)).image(c.texture.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let painter = ui.painter().with_clip_rect(dest);
        painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, Color32::from_white_alpha(160)), egui::StrokeKind::Outside);
        painter.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], egui::Stroke::new(2.0, Color32::WHITE));
        painter.circle_filled(Pos2::new(x, r.center().y), 6.0, Color32::WHITE);
    }

    fn paint_selection(&self, ui: &egui::Ui, dest: Rect, crop: Rect) {
        let Some(sel) = &self.shown.selection else { return };
        let mut outline: Vec<Pos2> = sel.outline().into_iter().map(|p| view::to_screen(dest, crop, p)).collect();
        outline.push(outline[0]);
        // Marching-ants style: white dashes over a dark line, visible on any image.
        let painter = ui.painter().with_clip_rect(dest.expand(1.0));
        painter.add(egui::Shape::line(outline.clone(), egui::Stroke::new(1.0, Color32::from_black_alpha(200))));
        painter.extend(egui::Shape::dashed_line(&outline, egui::Stroke::new(1.0, Color32::WHITE), 5.0, 4.0));
    }

    fn paint_crop(&self, ui: &egui::Ui, rect: Option<Rect>, dest: Rect, crop: Rect, viewport: Rect) {
        let painter = ui.painter().with_clip_rect(dest);
        let shade = Color32::from_black_alpha(140);
        match rect.map(|r| Rect::from_two_pos(view::to_screen(dest, crop, r.min), view::to_screen(dest, crop, r.max))) {
            Some(r) => {
                // Darken everything outside the crop.
                for side in [
                    Rect::from_x_y_ranges(dest.x_range(), dest.top()..=r.top()),
                    Rect::from_x_y_ranges(dest.x_range(), r.bottom()..=dest.bottom()),
                    Rect::from_x_y_ranges(dest.left()..=r.left(), r.y_range()),
                    Rect::from_x_y_ranges(r.right()..=dest.right(), r.y_range()),
                ] {
                    painter.rect_filled(side, 0.0, shade);
                }
                painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
                if let Some(name) = rect.and_then(view::ratio_name) {
                    bubble(&painter, Pos2::new(r.center().x, r.top() + 20.0), name);
                }
            }
            None => {
                painter.rect_filled(dest, 0.0, shade);
            }
        }
        bubble(ui.painter(), Pos2::new(viewport.center().x, viewport.top() + 24.0), tr("Drag to select · Enter to crop · Esc to cancel"));
    }

    /// Straighten: the canvas the turned image will get, with a grid to line things up against.
    fn paint_straighten(&self, ui: &egui::Ui, degrees: f32, size: Vec2, dest: Rect, crop: Rect, viewport: Rect) {
        let image = Rect::from_two_pos(view::to_screen(dest, crop, Pos2::ZERO), view::to_screen(dest, crop, size.to_pos2()));
        let kept = Rect::from_center_size(image.center(), edit::straighten_size(size, degrees) * (image.size() / size));
        let painter = ui.painter().with_clip_rect(viewport);
        let shade = Color32::from_black_alpha(160);
        for side in [
            Rect::from_x_y_ranges(viewport.x_range(), viewport.top()..=kept.top()),
            Rect::from_x_y_ranges(viewport.x_range(), kept.bottom()..=viewport.bottom()),
            Rect::from_x_y_ranges(viewport.left()..=kept.left(), kept.y_range()),
            Rect::from_x_y_ranges(kept.right()..=viewport.right(), kept.y_range()),
        ] {
            painter.rect_filled(side, 0.0, shade);
        }
        const LINES: usize = 8;
        let lines = (1..LINES).flat_map(|i| {
            let (x, y) = (kept.left() + kept.width() * i as f32 / LINES as f32, kept.top() + kept.height() * i as f32 / LINES as f32);
            [[Pos2::new(x, kept.top()), Pos2::new(x, kept.bottom())], [Pos2::new(kept.left(), y), Pos2::new(kept.right(), y)]]
        });
        for [a, b] in lines {
            painter.line_segment([a, b], egui::Stroke::new(3.0, Color32::from_black_alpha(90)));
            painter.line_segment([a, b], egui::Stroke::new(1.0, Color32::from_white_alpha(140)));
        }
        painter.rect_stroke(kept, 0.0, egui::Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Middle);
        bubble(ui.painter(), Pos2::new(viewport.center().x, viewport.top() + 24.0), tr("Turn until the lines run straight · Enter to apply · Esc to cancel"));
    }

    fn paint_perspective(&self, ui: &egui::Ui, corners: [Pos2; 4], dest: Rect, crop: Rect, viewport: Rect) {
        let q = corners.map(|p| view::to_screen(dest, crop, p));
        let painter = ui.painter().with_clip_rect(viewport);
        let convex = edit::is_convex(q);
        let color = if convex { Color32::YELLOW } else { Color32::RED };
        let mut lines = vec![[q[0], q[1]], [q[1], q[2]], [q[2], q[3]], [q[3], q[0]]];
        if convex {
            // The guide lines of a square grid on the rectangle, to line up with the subject.
            let at = edit::homography(q);
            let p = |u: f64, v: f64| Pos2::new(at(u, v)[0] as f32, at(u, v)[1] as f32);
            lines.extend((1..4).flat_map(|i| [[p(i as f64 / 4.0, 0.0), p(i as f64 / 4.0, 1.0)], [p(0.0, i as f64 / 4.0), p(1.0, i as f64 / 4.0)]]));
        }
        for [a, b] in lines.iter().copied() {
            painter.line_segment([a, b], egui::Stroke::new(3.0, Color32::from_black_alpha(160)));
        }
        for (i, [a, b]) in lines.into_iter().enumerate() {
            painter.line_segment([a, b], egui::Stroke::new(if i < 4 { 1.5 } else { 1.0 }, color));
        }
        for p in q {
            painter.circle(p, 6.0, Color32::from_black_alpha(120), egui::Stroke::new(1.5, color));
        }
        bubble(ui.painter(), Pos2::new(viewport.center().x, viewport.top() + 24.0), tr("Drag the corners onto a rectangle · Enter to apply · Esc to cancel"));
    }

    fn paint_measure(&self, ui: &egui::Ui, ends: Option<[Pos2; 2]>, doc: &Doc, dest: Rect, crop: Rect, viewport: Rect) {
        let hint = match ends {
            Some(m) => {
                let [a, b] = m.map(|p| view::to_screen(dest, crop, p));
                let painter = ui.painter().with_clip_rect(viewport);
                // A dark halo keeps the guide line visible on light and dark images.
                painter.line_segment([a, b], egui::Stroke::new(3.0, Color32::from_black_alpha(160)));
                painter.line_segment([a, b], egui::Stroke::new(1.5, Color32::YELLOW));
                for p in [a, b] {
                    painter.circle(p, 5.0, Color32::from_black_alpha(120), egui::Stroke::new(1.5, Color32::YELLOW));
                }
                let [dx, dy, d] = doc.measure(m[0], m[1]);
                trf("Distance: {d} px · ΔX: {dx} px · ΔY: {dy} px", &[("d", &format!("{d:.0}")), ("dx", &format!("{dx:.0}")), ("dy", &format!("{dy:.0}"))])
            }
            None => tr("Drag to measure in image pixels · Esc to stop").into(),
        };
        bubble(ui.painter(), Pos2::new(viewport.center().x, viewport.top() + 24.0), &hint);
    }

    fn report_timings(&mut self, ctx: &egui::Context, full: bool, frame_wh: [u32; 2]) {
        if !full && self.preview_reported != Some(frame_wh) {
            eprintln!("preview     {:.1} ms  ({}×{})", ms(self.t0), frame_wh[0], frame_wh[1]);
            self.preview_reported = Some(frame_wh);
        }
        if full && !self.full_reported {
            eprintln!("full image  {:.1} ms", ms(self.t0));
            self.full_reported = true;
            self.close_next = self.bench;
            // The image reaches the screen after this frame; closing waits one more.
            ctx.request_repaint();
        }
        if let Some(t) = self.nav_start.filter(|_| full) {
            eprintln!("navigation  {:.1} ms", ms(t));
            self.nav_start = None;
        }
    }

    /// The short message at the bottom, for as long as it lasts.
    pub(super) fn show_status(&mut self, ui: &egui::Ui, viewport: Rect) {
        let ctx = ui.ctx();
        let Some((text, at)) = &self.status else { return };
        const SHOWN: std::time::Duration = std::time::Duration::from_secs(4);
        if at.elapsed() < SHOWN {
            bubble(ui.painter(), Pos2::new(viewport.center().x, viewport.bottom() - 24.0), text);
            ctx.request_repaint_after(SHOWN - at.elapsed());
        } else {
            self.status = None;
        }
    }
}

/// Resizes the `crop` part of `frame` so that, after `transform`, it's `size` px.
/// Transforming the small resized output instead of the full image keeps rotating cheap.
pub(super) fn resize(frame: &Frame, crop: Rect, size: [u32; 2], transform: Transform, interpolation: Interpolation) -> Vec<u8> {
    let [w, h] = if transform.rot % 2 == 1 { [size[1], size[0]] } else { size };
    let pixels = edit::resize(frame, crop, w, h, interpolation.cpu());
    if transform == Transform::default() {
        pixels
    } else {
        transform.apply(edit::to_dynamic(frame.alpha, w, h, pixels).unwrap()).into_bytes()
    }
}

/// A short message in a dark rounded box centred on `center`.
pub(super) fn bubble(painter: &egui::Painter, center: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_string(), egui::TextStyle::Body.resolve(&painter.ctx().global_style()), Color32::WHITE);
    let rect = Rect::from_center_size(center, galley.size() + Vec2::new(16.0, 8.0));
    painter.rect_filled(rect, 4.0, Color32::from_black_alpha(190));
    painter.galley(rect.min + Vec2::new(8.0, 4.0), galley, Color32::WHITE);
}

pub(super) fn to_color_image(alpha: bool, size: [usize; 2], pixels: &[u8]) -> ColorImage {
    if alpha { ColorImage::from_rgba_unmultiplied(size, pixels) } else { ColorImage::from_rgb(size, pixels) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_with_transform() {
        // 3×2 RGB frame, red channel = pixel index.
        let pixels: Vec<u8> = (0..6u8).flat_map(|i| [i, 0, 0]).collect();
        let frame = Frame { w: 3, h: 2, pixels, alpha: false, delay: std::time::Duration::ZERO };
        let t = Transform::default().rotate_right();
        let crop = t.to_source(Rect::from_min_size(Pos2::ZERO, Vec2::new(2.0, 3.0)), Vec2::new(3.0, 2.0));
        let out = resize(&frame, crop, [2, 3], t, Interpolation::Bilinear);
        assert_eq!(out.len(), 2 * 3 * 3);
        // Rotated right, the top row reads bottom-left then top-left of the source: 3, 0.
        let red: Vec<u8> = out.chunks(3).map(|p| p[0]).collect();
        assert_eq!(red, [3, 0, 4, 1, 5, 2]);
    }
}
