// Everything here is test code: the i18n test reads source only up to this marker.
// #[cfg(test)]

use super::*;
use crate::view;

/// An app showing a decoded 2×1 image.
fn app_with_image() -> (App, PathBuf) {
    let (tx, rx) = mpsc::channel();
    let notify = Notify { tx, ctx: Arc::default() };
    let loader = Loader::new(notify.clone(), 0, [100, 100], true);
    let settings = Settings { default_asked: true, ..Settings::default() };
    let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    let mut app = App::new(&cc, Instant::now(), false, false, rx, loader, notify, settings, None);
    let path = PathBuf::from("/a.png");
    let frame = Frame { w: 2, h: 1, pixels: vec![0; 6], alpha: false, delay: std::time::Duration::ZERO };
    app.cache.insert(path.clone(), Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None }));
    app.files = vec![path.clone()];
    (app, path)
}

/// `app_with_image`, with a 2×1 SVG as the image.
fn app_with_svg() -> (App, PathBuf, Arc<Loaded>) {
    let (mut app, path) = app_with_image();
    let tree = crate::svg::load(br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"/>"#).unwrap();
    let frame = crate::svg::render(&tree, 1).unwrap();
    let loaded = Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: Some(tree) });
    app.cache.insert(path.clone(), loaded.clone());
    (app, path, loaded)
}

#[test]
fn svg_sharpens_with_zoom_and_returns_to_1x() {
    let (mut app, _, l) = app_with_svg();
    let doc = Doc::new(2, 1);
    assert!(app.sharp_frame(Some(&l), &doc, 0.5).is_none());
    assert!(app.sharp_frame(Some(&l), &doc, 3.0).is_none(), "nothing until the render arrives");
    let msg = app.rx.recv().unwrap();
    app.handle(msg);
    let (k, frame) = app.sharp_frame(Some(&l), &doc, 3.0).unwrap();
    assert_eq!((k, frame.w, frame.h), (4, 8, 4));
    assert!(app.rx.try_recv().is_err(), "the same tier isn't rendered twice");
    // Still shown while the next tier renders, then back to 1× when zoomed out.
    assert_eq!(app.sharp_frame(Some(&l), &doc, 5.0).unwrap().0, 4);
    assert!(app.sharp_frame(Some(&l), &doc, 1.0).is_none());
    assert!(app.sharp.is_none());
    // The 16× render arrives too late.
    let msg = app.rx.recv().unwrap();
    app.handle(msg);
    assert!(app.sharp.is_none());
}

#[test]
fn a_sharp_svg_draws_at_any_zoom_and_rotation_on_both_paths() {
    let (mut app, path) = app_with_image();
    let tree = crate::svg::load(br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"/>"#).unwrap();
    let l = Arc::new(Loaded { frames: vec![crate::svg::render(&tree, 1).unwrap()], file_size: 6, svg: Some(tree.clone()) });
    app.cache.insert(path.clone(), l.clone());
    let ctx = egui::Context::default();
    ctx.set_fonts(markup::font_definitions());
    for gpu in [true, false] {
        app.settings.gpu_scaling = gpu;
        app.reset_shown();
        app.sharp_wanted = 4;
        app.handle(Msg::Sharp(path.clone(), 4, crate::svg::render(&tree, 4).unwrap()));
        for quarter_turn in [false, true] {
            if quarter_turn {
                app.edit(Doc::rotate_right);
            }
            for view in [View::Fit, View::Manual { scale: 3.3, center: Pos2::new(10.0, 10.0) }, View::Manual { scale: 2.0, center: Pos2::new(99.0, 49.0) }] {
                app.view = view;
                let mut input = egui::RawInput::default();
                input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
                let mut output = ctx.run_ui(input, |ui| {
                    let doc = app.doc();
                    app.draw(ui, Some(&l), doc, None, ui.available_rect_before_wrap());
                });
                output.textures_delta.clear();
                assert_eq!(app.sharp_wanted, 4);
                if gpu {
                    assert_eq!(app.texture.as_ref().unwrap().size(), [400, 200], "the whole sharp frame is uploaded");
                }
            }
        }
    }
}

#[test]
fn svg_with_markup_stays_at_1x_and_navigation_clears_it() {
    let (mut app, path, l) = app_with_svg();
    let mut doc = Doc::new(2, 1);
    doc.elements.push(Element::erase(&Selection::Rect(doc.bounds()), false, doc.bounds(), Color32::TRANSPARENT));
    assert!(app.sharp_frame(Some(&l), &doc, 8.0).is_none());
    assert_eq!(app.sharp_wanted, 1);
    app.sharp_wanted = 4;
    app.handle(Msg::Sharp(path, 4, l.frames[0].clone()));
    assert!(app.sharp.is_some());
    app.reset_shown();
    assert!(app.sharp.is_none() && app.sharp_wanted == 1);
}

#[test]
fn native_decoders_only_get_the_image_on_screen() {
    let (mut app, _) = app_with_image();
    app.cache.clear();
    app.files = ["/a.heic", "/b.avif", "/c.png"].map(PathBuf::from).to_vec();
    app.index = 1;
    assert_eq!(app.to_decode(), ["/b.avif", "/c.png"].map(PathBuf::from));
    app.index = 2;
    assert_eq!(app.to_decode(), ["/c.png"].map(PathBuf::from));
}

#[test]
fn pages_step_by_hand_and_never_play() {
    let (mut app, path) = app_with_image();
    let page = |shade: u8| Frame { w: 2, h: 1, pixels: vec![shade; 6], alpha: false, delay: std::time::Duration::ZERO };
    let loaded = Arc::new(Loaded { frames: vec![page(0), page(1), page(2)], file_size: 6, svg: None });
    app.cache.insert(path, loaded.clone());
    let ctx = egui::Context::default();
    for _ in 0..3 {
        app.animate(&ctx, &loaded, &[]);
    }
    assert_eq!(app.anim_frame, 0);
    assert!(app.make_title(None).contains(trf("page {n} of {count}", &[("n", &1), ("count", &3)]).as_str()));
    app.animate(&ctx, &loaded, &[crate::app::menu::Action::NextFrame]);
    assert_eq!(app.anim_frame, 1);
    app.animate(&ctx, &loaded, &[crate::app::menu::Action::PrevFrame, crate::app::menu::Action::PrevFrame]);
    assert_eq!(app.anim_frame, 2);
}

#[test]
fn title_shows_the_zoom() {
    let (app, _) = app_with_image();
    assert_eq!(app.make_title(Some(0.25)), "a.png — 2×1 — 1 KB — 0.25×");
    assert_eq!(app.make_title(None), "a.png — 2×1 — 1 KB");
}

#[test]
fn only_the_edits_written_count_as_saved() {
    let (mut app, path) = app_with_image();
    app.edit(Doc::rotate_left);
    let doc = app.doc.clone().unwrap();
    let dest = PathBuf::from("/nonexistent/copy.jpg");
    // An EXIF copy, a failed save, and a Save As of the image shown before navigating.
    app.handle(Msg::Saved(dest.clone(), Ok(()), None));
    assert!(app.dirty());
    app.handle(Msg::Saved(dest.clone(), Err("disk full".into()), Some((path.clone(), doc.clone()))));
    assert!(app.dirty());
    app.handle(Msg::Saved(dest.clone(), Ok(()), Some(("/b.png".into(), doc.clone()))));
    assert!(app.dirty());
    app.handle(Msg::Saved(dest, Ok(()), Some((path, doc))));
    assert!(!app.dirty());
}

#[test]
fn a_cancelled_restore_leaves_the_next_run_alone() {
    let (mut app, path) = app_with_image();
    app.restoring = Some(Restoring { id: 2, task: crate::ai::Task::Denoise, progress: 0.0, started: Instant::now(), cancel: Arc::default(), selection: None });
    app.handle(Msg::Progress(1, 0.5));
    app.handle(Msg::Restored(1, path.clone(), Rect::ZERO, false, Err("cancelled".into())));
    assert_eq!(app.restoring.as_ref().map(|r| r.progress), Some(0.0));
    assert!(app.status.is_none());
    app.handle(Msg::Progress(2, 0.5));
    assert_eq!(app.restoring.as_ref().map(|r| r.progress), Some(0.5));
    app.handle(Msg::Restored(2, path, Rect::ZERO, false, Err("cancelled".into())));
    assert!(app.restoring.is_none());
}

#[test]
fn removing_an_object_pastes_the_fill_once_and_clears_the_selection() {
    let (mut app, path) = app_with_image();
    let region = Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, 1.0));
    let sel = Selection::Rect(region);
    let run = |id, sel| Restoring { id, task: crate::ai::Task::Inpaint, progress: 0.0, started: Instant::now(), cancel: Arc::default(), selection: Some(sel) };
    // A failure keeps the selection for another try.
    app.selection = Some(sel.clone());
    app.restoring = Some(run(1, sel.clone()));
    app.handle(Msg::Restored(1, path.clone(), region, true, Err("boom".into())));
    assert!(app.selection.is_some() && app.doc().is_none_or(|d| d.elements.is_empty()));
    app.restoring = Some(run(2, sel));
    let fill = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
    app.handle(Msg::Restored(2, path, region, true, Ok((fill, 0.0))));
    assert!(app.selection.is_none() && app.restoring.is_none());
    assert_eq!(app.doc().map(|d| d.elements.len()), Some(1));
}

/// A big image at 1:1, panned by nothing yet.
fn app_zoomed_in() -> (App, View) {
    let (mut app, path) = app_with_image();
    let frame = Frame { w: 1000, h: 1000, pixels: vec![0; 3_000_000], alpha: false, delay: std::time::Duration::ZERO };
    app.cache.insert(path, Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None }));
    app.view = crate::view::actual_size(Vec2::splat(1000.0));
    let view = app.view;
    (app, view)
}

fn press(button: egui::PointerButton, pressed: bool, at: f32) -> egui::Event {
    egui::Event::PointerButton { pos: Pos2::new(at, at), button, pressed, modifiers: egui::Modifiers::NONE }
}

fn drag_events(button: egui::PointerButton) -> Vec<egui::Event> {
    let to = |at| egui::Event::PointerMoved(Pos2::new(at, at));
    vec![to(100.0), press(button, true, 100.0), to(130.0), to(160.0), to(190.0), press(button, false, 190.0), to(190.0)]
}

/// Runs `image_input` for one frame per event, in a 400 px viewport.
fn feed(app: &mut App, events: Vec<egui::Event>) {
    let ctx = egui::Context::default();
    ctx.set_fonts(markup::font_definitions());
    for event in events {
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
        input.events.push(event);
        let mut output = ctx.run_ui(input, |ui| {
            let doc = app.doc().unwrap();
            app.image_input(ui, &doc, ui.available_rect_before_wrap(), 1.0);
        });
        output.textures_delta.clear();
    }
}

const LEFT: egui::PointerButton = egui::PointerButton::Primary;
const RIGHT: egui::PointerButton = egui::PointerButton::Secondary;

#[test]
fn right_drag_pans_with_any_tool() {
    for tool in [Tool::Draw(Shape::Rect), Tool::Draw(Shape::Magnifier), Tool::Select, Tool::Selection(SelKind::Rect), Tool::Crop, Tool::Measure, Tool::Pan] {
        let (mut app, start) = app_zoomed_in();
        app.tool = tool;
        feed(&mut app, drag_events(RIGHT));
        assert_ne!(app.view, start);
        assert!(!app.right_pan);
        // The tool saw nothing.
        assert!(app.doc().unwrap().elements.is_empty() && app.change_base.is_none());
        assert!(app.selection.is_none() && app.crop_rect.is_none() && app.measure.is_none());
    }
}

#[test]
fn left_drag_still_uses_the_tool_or_pans() {
    let (mut app, start) = app_zoomed_in();
    app.tool = Tool::Draw(Shape::Rect);
    feed(&mut app, drag_events(LEFT));
    assert_eq!(app.view, start);
    assert_eq!(app.doc().unwrap().elements.len(), 1);
    let (mut app, start) = app_zoomed_in();
    feed(&mut app, drag_events(LEFT));
    assert_ne!(app.view, start, "with no tool, left-drag pans");
}

#[test]
fn a_right_press_during_a_left_drawing_leaves_it_alone() {
    let (mut app, start) = app_zoomed_in();
    app.tool = Tool::Draw(Shape::Rect);
    let mut events = drag_events(LEFT);
    // Tapped once the drawing is under way.
    events.splice(4..4, [press(RIGHT, true, 160.0), press(RIGHT, false, 160.0)]);
    feed(&mut app, events);
    assert_eq!(app.view, start);
    assert!(app.change_base.is_none() && !app.right_pan);
    assert_eq!(app.doc().unwrap().elements.len(), 1);
    assert_eq!(app.undo.len(), 1);
}

#[test]
fn right_drag_pans_the_before_after_preview() {
    let (mut app, start) = app_zoomed_in();
    let texture = egui::Context::default().load_texture("t", egui::ColorImage::example(), TextureOptions::LINEAR);
    app.compare = Some(Compare { task: crate::ai::Task::Denoise, region: Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)), texture, split: 0.5, ms_per_mp: 0.0, scale: 1 });
    feed(&mut app, drag_events(RIGHT));
    assert_ne!(app.view, start);
    assert_eq!(app.compare.as_ref().map(|c| c.split), Some(0.5));
}

fn with_line(app: &mut App) -> Arc<Loaded> {
    let mut line = Element::new(Shape::Line, Pos2::new(1.0, 1.0), Style::for_image(Vec2::splat(1000.0)));
    line.points[1] = Pos2::new(50.0, 50.0);
    app.doc_mut().unwrap().elements.push(line);
    app.current().and_then(|p| app.cache.get(p)).unwrap().clone()
}

#[test]
fn markup_is_shown_as_it_will_be_saved() {
    let (mut app, _) = app_zoomed_in();
    let l = with_line(&mut app);
    for bar in [true, false] {
        app.show_toolbar = bar;
        let doc = app.doc().unwrap();
        let flat = app.flattened(Some(&l), &doc).unwrap();
        assert_eq!(flat.pixels, edit::flat(&l.frames[0], &doc).unwrap().pixels);
        // Nothing changed: no new render, so nothing else is invalidated.
        let generation = app.generation;
        assert!(Arc::ptr_eq(&flat, &app.flattened(Some(&l), &doc).unwrap()));
        assert_eq!(app.generation, generation);
    }
    // A change renders again.
    let before = app.flattened(Some(&l), &app.doc().unwrap()).unwrap();
    app.doc_mut().unwrap().elements[0].points[1] = Pos2::new(90.0, 20.0);
    assert!(!Arc::ptr_eq(&before, &app.flattened(Some(&l), &app.doc().unwrap()).unwrap()));
}

#[test]
fn flattening_stops_where_it_cant_apply() {
    let (mut app, _) = app_zoomed_in();
    let l = with_line(&mut app);
    let doc = app.doc().unwrap();
    assert!(app.flattened(Some(&l), &doc).is_some());
    // Still decoding.
    assert!(app.flattened(None, &doc).is_none() && app.flat.is_none());
    // No markup.
    assert!(app.flattened(Some(&l), &Doc::new(1000, 1000)).is_none());
    // Animated.
    let frame = || Frame { w: 2, h: 2, pixels: vec![0; 12], alpha: false, delay: std::time::Duration::ZERO };
    let animated = Loaded { frames: vec![frame(), frame()], file_size: 6, svg: None };
    assert!(app.flattened(Some(&animated), &doc).is_none());
    // Past the limit.
    let sized = |w, h| { let mut d = doc.clone(); d.resize(w, h); d };
    assert!(app.flattened(Some(&l), &sized(4000, 3000)).is_some(), "12 MP");
    assert!(app.flattened(Some(&l), &sized(4000, 4000)).is_some(), "16 MP");
    assert!(app.flattened(Some(&l), &sized(6000, 4000)).is_none(), "24 MP");
}

#[test]
fn markup_stays_at_screen_resolution_until_a_change_ends() {
    let (mut app, _) = app_zoomed_in();
    let l = with_line(&mut app);
    let doc = app.doc().unwrap();
    // A small image too: rendering every frame of a drag lags at 1600×900.
    app.change_base = Some(doc.clone());
    assert!(app.flattened(Some(&l), &doc).is_none(), "tracing");
    app.change_base = None;
    assert!(app.flattened(Some(&l), &doc).is_some(), "released");
    app.adjusting = Some(doc.clone());
    assert!(app.flattened(Some(&l), &doc).is_none(), "Adjust Color open");
    app.adjusting = None;
    // Typing, even once something else has taken `change_base`.
    app.editing_text = Some(0);
    assert!(app.flattened(Some(&l), &doc).is_none(), "typing");
    app.editing_text = None;
    assert!(app.flattened(Some(&l), &doc).is_some(), "text finished");
}

#[test]
fn the_texture_follows_flattening_with_the_elements_still_editable() {
    let (mut app, _) = app_zoomed_in();
    let l = with_line(&mut app);
    let ctx = egui::Context::default();
    ctx.set_fonts(markup::font_definitions());
    let draw = |app: &mut App| {
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
        let mut output = ctx.run_ui(input, |ui| {
            let doc = app.doc();
            app.draw(ui, Some(&l), doc, None, ui.available_rect_before_wrap());
        });
        output.textures_delta.clear();
        app.tex_key.unwrap()
    };
    assert!(draw(&mut app).flat);
    assert_eq!(app.doc().unwrap().elements.len(), 1, "flattening only changes the view");
    // Without flattening it's drawn on top again, from a new texture.
    app.adjusting = app.doc();
    assert!(!draw(&mut app).flat);
}

#[test]
fn curves_panel_runs_on_a_real_app() {
    let (mut app, _) = app_with_image();
    app.adjusting = app.doc();
    app.curve_panel = Some(CurvePanel::default());
    let ctx = egui::Context::default();
    ctx.set_fonts(markup::font_definitions());
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.adjust_panel(ui.ctx()));
        output.textures_delta.clear();
    }
    assert!(app.curve_panel.is_some() && app.adjusting.is_some(), "still open");
    assert_eq!(app.doc().unwrap().adjust, edit::Adjust::default(), "just opening it changes nothing");
}

/// Clipboard between two processes (needs a desktop session):
/// `cargo test clipboard_copy -- --ignored & sleep 3; cargo test clipboard_paste -- --ignored`
#[test]
#[ignore]
fn clipboard_copy() {
    let bytes: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
    let mut clipboard = arboard::Clipboard::new().unwrap();
    clipboard.set_image(arboard::ImageData { width: 4, height: 3, bytes: bytes.into() }).unwrap();
    // Like the app's window, keep owning the clipboard for a while.
    std::thread::sleep(std::time::Duration::from_secs(8));
}

#[test]
#[ignore]
fn clipboard_paste() {
    let img = arboard::Clipboard::new().unwrap().get_image().unwrap();
    assert_eq!((img.width, img.height), (4, 3));
    assert_eq!(img.bytes[..8], [0, 1, 2, 3, 4, 5, 6, 7]);
}

/// The file Copy File copies in `file_copy` and `file_paste` look for.
fn copied_file() -> PathBuf {
    std::env::temp_dir().join("lightview-copy-file.png")
}

/// Copy File between two processes (needs a desktop session):
/// `cargo test file_copy -- --ignored & sleep 3; cargo test file_paste -- --ignored`
#[test]
#[ignore]
fn file_copy() {
    let (mut app, _) = app_with_image();
    std::fs::write(copied_file(), b"png").unwrap();
    app.files = vec![copied_file()];
    app.run_actions(&[Action::CopyFile], &egui::Context::default(), false);
    assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some(tr("Copied the file")));
    // The app holds the clipboard; so does this, for a while.
    std::thread::sleep(std::time::Duration::from_secs(8));
}

#[test]
#[ignore]
fn file_paste() {
    let files = arboard::Clipboard::new().unwrap().get().file_list().unwrap();
    assert_eq!(files, [copied_file().canonicalize().unwrap()]);
}

#[test]
fn pixelate_and_red_eye_patch_the_selection_and_undo() {
    let (mut app, path) = app_with_image();
    let frame = Frame { w: 40, h: 30, pixels: (0..40 * 30 * 3).map(|i| (i % 251) as u8).collect(), alpha: false, delay: std::time::Duration::ZERO };
    app.cache.insert(path, Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None }));
    let ctx = egui::Context::default();
    for action in [Action::Pixelate, Action::RedEye] {
        app.selection = Some(crate::markup::Selection::Ellipse(Rect::from_min_size(Pos2::new(5.0, 4.0), Vec2::new(20.0, 16.0))));
        app.run_actions(&[action.clone()], &ctx, false);
        let doc = app.doc().unwrap();
        assert_eq!(doc.elements.len(), 1, "one patch");
        assert_eq!(doc.elements[0].points, [Pos2::new(5.0, 4.0), Pos2::new(25.0, 20.0)]);
        assert!(app.selection.is_none(), "the result is shown, not outlined");
        app.run_actions(&[Action::Undo], &ctx, false);
        assert!(app.doc().unwrap().elements.is_empty());
    }
    // Nothing to patch without a selection.
    app.run_actions(&[Action::Pixelate], &ctx, false);
    assert!(app.doc().unwrap().elements.is_empty());
}

#[test]
fn greyscale_and_invert_are_one_undo_step_each() {
    let (mut app, _) = app_with_image();
    let ctx = egui::Context::default();
    app.run_actions(&[Action::Greyscale], &ctx, false);
    app.run_actions(&[Action::Invert], &ctx, false);
    let a = app.doc().unwrap().adjust;
    assert!(a.saturation == -1.0 && a.invert);
    app.run_actions(&[Action::Undo], &ctx, false);
    assert_eq!((app.doc().unwrap().adjust.saturation, app.doc().unwrap().adjust.invert), (-1.0, false));
    app.run_actions(&[Action::Greyscale], &ctx, false);
    assert!(app.doc().unwrap().adjust.is_identity(), "Greyscale toggles back");
}

#[test]
fn read_text_shows_what_it_found() {
    let (mut app, path) = app_with_image();
    let frame = Frame { w: 8, h: 8, pixels: vec![255; 8 * 8 * 3], alpha: false, delay: std::time::Duration::ZERO };
    app.cache.insert(path, Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None }));
    app.run_actions(&[Action::ReadText(true)], &egui::Context::default(), false);
    assert!(app.reading);
    // A blank image has no QR code: the thread says so.
    while app.reading {
        let msg = app.rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        app.handle(msg);
    }
    assert!(app.found_text.is_none() && app.found_text.is_none());
    assert_eq!(app.status.as_ref().unwrap().0, tr("No QR code found"));
    app.handle(Msg::TextFound(false, Ok("Hola".into())));
    assert_eq!(app.found_text, Some((tr("Text in Image"), "Hola".to_string())));
}

#[test]
fn straighten_applies_through_perspective() {
    let (mut app, path) = app_with_image();
    let frame = Frame { w: 40, h: 30, pixels: vec![128; 40 * 30 * 3], alpha: false, delay: std::time::Duration::ZERO };
    app.cache.insert(path, Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None }));
    let ctx = egui::Context::default();
    app.run_actions(&[Action::Straighten], &ctx, false);
    assert!(app.tool == Tool::Straighten);
    app.straighten = 0.0;
    app.apply_straighten();
    assert!(app.tool == Tool::Pan && app.doc().unwrap().base.is_none(), "0° changes nothing");
    app.run_actions(&[Action::Straighten], &ctx, false);
    app.straighten = 8.0;
    app.apply_straighten();
    let doc = app.doc().unwrap();
    let s = edit::straighten_size(Vec2::new(40.0, 30.0), 8.0);
    assert!(app.tool == Tool::Pan && doc.base.is_some());
    assert_eq!(doc.size, [s.x.round() as u32, s.y.round() as u32]);
    let [r, g, b] = app.settings.background;
    let frame = &app.cache.get(app.current().unwrap()).unwrap().frames[0];
    assert_eq!(edit::render(frame, &doc).unwrap().into_rgb8().get_pixel(0, 0).0, [r, g, b], "corners get the background");
    app.run_actions(&[Action::Undo], &ctx, false);
    assert_eq!(app.doc().unwrap().size, [40, 30]);
}

#[test]
fn copy_file_says_what_happened() {
    let (mut app, _) = app_with_image();
    // A file that isn't there can't go on the clipboard, with or without a desktop session.
    app.files = vec![PathBuf::from("/nowhere/lightview.png")];
    app.run_actions(&[Action::CopyFile], &egui::Context::default(), false);
    assert!(app.status.as_ref().is_some_and(|s| s.0.starts_with(&trf("Couldn't copy the file: {e}", &[("e", &"")]))), "{:?}", app.status);
}

/// One frame of `handle_keys` with `key` pressed.
fn press_key(app: &mut App, key: Key) {
    let ctx = egui::Context::default();
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    let mut output = ctx.run_ui(input, |ui| app.handle_keys(ui.ctx(), false, ui.input(|i| i.key_pressed(Key::Escape)), false));
    output.textures_delta.clear();
}

/// A 1000 px image, black on the left half and white on the right, with the corners on the white part.
fn app_with_corners() -> (App, Arc<Loaded>) {
    let (mut app, path) = app_with_image();
    let pixels = (0..1000 * 1000 * 3).map(|i| if (i / 3) % 1000 < 500 { 0 } else { 255 }).collect();
    let frame = Frame { w: 1000, h: 1000, pixels, alpha: false, delay: std::time::Duration::ZERO };
    let l = Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None });
    app.cache.insert(path, l.clone());
    app.settings.gpu_scaling = false;
    app.run_actions(&[Action::Perspective], &egui::Context::default(), false);
    assert!(app.tool == Tool::Perspective);
    app.perspective = Some([Pos2::new(600.0, 100.0), Pos2::new(900.0, 100.0), Pos2::new(900.0, 900.0), Pos2::new(600.0, 900.0)]);
    (app, l)
}

/// Draws one frame and gives the darkest and brightest pixels the texture was made from.
fn drawn_range(app: &mut App, l: &Loaded) -> (u8, u8) {
    let ctx = egui::Context::default();
    ctx.set_fonts(markup::font_definitions());
    let mut input = egui::RawInput::default();
    input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
    let mut output = ctx.run_ui(input, |ui| {
        let doc = app.doc();
        app.draw(ui, Some(l), doc, None, ui.available_rect_before_wrap());
    });
    output.textures_delta.clear();
    let pixels = &app.resized.as_ref().unwrap().1;
    (*pixels.iter().min().unwrap(), *pixels.iter().max().unwrap())
}

#[test]
fn perspective_shows_undoes_and_redoes() {
    let (mut app, l) = app_with_corners();
    assert_eq!(drawn_range(&mut app, &l), (0, 255));
    press_key(&mut app, Key::Enter);
    assert!(app.tool == Tool::Pan && app.perspective.is_none());
    assert_eq!(app.doc().unwrap().size, [300, 800]);
    assert!(app.dirty());
    assert_eq!(drawn_range(&mut app, &l), (255, 255), "only the white part is left");
    app.undo_redo(true);
    assert_eq!(app.doc().unwrap().size, [1000, 1000]);
    assert_eq!(drawn_range(&mut app, &l), (0, 255));
    assert!(!app.dirty());
    app.undo_redo(false);
    assert_eq!(drawn_range(&mut app, &l), (255, 255));
}

#[test]
fn perspective_of_the_same_size_redraws() {
    // Nothing in the texture key but the generation can tell these pixels apart.
    let (mut app, l) = app_with_corners();
    app.perspective = Some([Pos2::new(0.0, 1000.0), Pos2::new(1000.0, 1000.0), Pos2::new(1000.0, 0.0), Pos2::ZERO]);
    drawn_range(&mut app, &l);
    let (key, resized) = (app.tex_key, app.resized.as_ref().unwrap().1.clone());
    press_key(&mut app, Key::Enter);
    drawn_range(&mut app, &l);
    assert!(app.tex_key != key);
    assert_eq!(app.doc().unwrap().size, [1000, 1000]);
    assert_eq!(app.resized.as_ref().unwrap().1, resized, "flipped top to bottom, the columns are the same");
}

#[test]
fn perspective_asks_before_flattening_markup() {
    let (mut app, _) = app_with_corners();
    with_line(&mut app);
    press_key(&mut app, Key::Enter);
    assert!(app.perspective_prompt);
    assert_eq!(app.doc().unwrap().elements.len(), 1, "nothing is flattened yet");
    // Enter and Esc belong to the dialog while it's open.
    press_key(&mut app, Key::Enter);
    press_key(&mut app, Key::Escape);
    assert!(app.tool == Tool::Perspective && app.perspective.is_some());
    app.apply_perspective();
    assert_eq!(app.doc().unwrap().size, [300, 800]);
    assert!(app.doc().unwrap().elements.is_empty());
}

#[test]
fn perspective_refuses_crossed_corners_and_keeps_the_tool() {
    let (mut app, _) = app_with_corners();
    app.perspective = app.perspective.map(|[a, b, c, d]| [a, c, b, d]);
    press_key(&mut app, Key::Enter);
    assert!(app.tool == Tool::Perspective && app.doc.is_none() && app.status.is_some());
}

#[test]
fn perspective_corners_follow_the_drag() {
    let (mut app, _) = app_zoomed_in();
    app.tool = Tool::Perspective;
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0));
    let (dest, vis) = crate::view::layout(app.view, Vec2::splat(1000.0), viewport, 1.0);
    let at = |p: f32| crate::view::to_image(dest, vis, Pos2::new(p, p));
    let far = Pos2::new(999.0, 999.0);
    app.perspective = Some([at(103.0), far, far, far]);
    feed(&mut app, drag_events(LEFT));
    let moved = app.perspective.unwrap();
    assert!(moved[0].distance(at(190.0)) < 1.0, "{:?}", moved[0]);
    assert_eq!(moved[1], far);
    assert!(app.perspective_drag.is_none());
    // A press away from every corner moves none.
    app.perspective = Some([at(150.0), far, far, far]);
    feed(&mut app, drag_events(LEFT));
    assert_eq!(app.perspective.unwrap()[0], at(150.0));
}

/// A temporary folder holding `files`, empty ones. Removed by hand at the end of a test.
fn temp_folder(name: &str, files: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lightview-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for file in files {
        std::fs::write(dir.join(file), b"").unwrap();
    }
    std::fs::canonicalize(dir).unwrap()
}

/// One frame of `ui` with `input`, with the new events it took.
fn frame(app: &mut App, ctx: &egui::Context, input: egui::RawInput) -> egui::FullOutput {
    use eframe::App as _;
    let mut eframe = eframe::Frame::_new_kittest();
    let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut eframe));
    output.textures_delta.clear();
    output
}

#[test]
fn dropped_image_opens_and_edits_ask_first() {
    let dir = temp_folder("drop-image", &["b.png", "notes.txt"]);
    let (mut app, _) = app_with_image();
    app.open_dropped(vec![dir.join("notes.txt")]);
    assert_eq!(app.files, [PathBuf::from("/a.png")], "not an image: stays where it is");
    assert!(app.status.is_some());
    app.edit(Doc::rotate_left);
    app.open_dropped(vec![dir.join("b.png")]);
    assert!(matches!(app.pending, Some(Pending::Open(_))), "unsaved edits ask first");
    app.pending = None;
    app.doc = None;
    app.open_dropped(vec![dir.join("b.png"), dir.join("notes.txt")]);
    assert_eq!(app.files, [dir.join("b.png")]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dropped_folder_opens_its_first_image() {
    let dir = temp_folder("drop-folder", &["notes.txt", "img10.png", "img2.png"]);
    let (mut app, _) = app_with_image();
    app.open_dropped(vec![dir.clone()]);
    assert!(matches!(app.rx.recv().unwrap(), Msg::Open(p) if p == dir.join("img2.png")));
    let empty = temp_folder("drop-empty", &["notes.txt"]);
    app.open_dropped(vec![empty.clone()]);
    assert!(matches!(app.rx.recv().unwrap(), Msg::Status(_)));
    assert_eq!(app.files, [PathBuf::from("/a.png")]);
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(empty).unwrap();
}

#[test]
fn a_change_on_disk_reloads_or_asks() {
    let stamp = (std::time::SystemTime::UNIX_EPOCH, 9);
    let changed = |path: &PathBuf| Msg::Changed(path.clone(), stamp);
    let shown = |app: &App, path: &PathBuf| app.cache.get(path).unwrap().file_size;
    let (mut app, path) = app_with_image();
    let view = View::Manual { scale: 2.0, center: Pos2::new(1.0, 1.0) };
    app.view = view;
    app.handle(changed(&"/other.png".into()));
    app.handle(Msg::Changed(path.clone(), stamp));
    assert_eq!(app.view, view, "the zoom stays");
    assert_eq!(shown(&app, &path), 6, "the old pixels stay up until the new ones arrive");
    // Lightview's own write.
    app.own_stamp = Some(stamp);
    app.edit(Doc::rotate_left);
    app.handle(changed(&path));
    assert!(!app.changed_prompt && app.dirty());
    app.own_stamp = None;
    // Someone else's write, with edits: they stay until the user answers.
    app.handle(changed(&path));
    assert!(app.changed_prompt && app.dirty());
    let ctx = egui::Context::default();
    ctx.set_fonts(markup::font_definitions());
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.changed_dialog(ui.ctx()));
    output.textures_delta.clear();
    assert!(app.changed_prompt && app.dirty(), "it waits");
    app.answer_changed(false);
    assert!(!app.changed_prompt && app.dirty(), "Keep Edits");
    app.handle(changed(&path));
    app.answer_changed(true);
    assert!(!app.changed_prompt && !app.dirty() && app.doc.is_none(), "Reload");
}

/// A real file changing under a real watcher and decoder, with no edits and then with some.
#[test]
fn an_outside_write_reaches_the_screen() {
    let dir = temp_folder("reload", &[]);
    let path = dir.join("a.png");
    image::RgbImage::new(4, 2).save(&path).unwrap();
    let (tx, rx) = mpsc::channel();
    let notify = Notify { tx, ctx: Arc::default() };
    let loader = Loader::new(notify.clone(), 1, [100, 100], true);
    let settings = Settings { default_asked: true, ..Settings::default() };
    let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    let mut app = App::new(&cc, Instant::now(), false, false, rx, loader, notify, settings, None);
    app.open(path.clone());
    *app.watched.lock().unwrap() = Some(path.clone());
    let wait_for = |app: &mut App, done: &dyn Fn(&App) -> bool| {
        while !done(app) {
            let msg = app.rx.recv_timeout(std::time::Duration::from_secs(10)).expect("nothing arrived");
            app.handle(msg);
        }
    };
    let width = |app: &App| app.cache.get(&path).map(|l| l.frames[0].w);
    wait_for(&mut app, &|app| width(app) == Some(4));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    image::RgbImage::new(6, 2).save(&path).unwrap();
    wait_for(&mut app, &|app| width(app) == Some(6));
    app.edit(Doc::rotate_left);
    image::RgbImage::new(8, 2).save(&path).unwrap();
    wait_for(&mut app, &|app| app.changed_prompt);
    assert_eq!(width(&app), Some(6), "the edits' image stays until the user answers");
    app.answer_changed(true);
    wait_for(&mut app, &|app| width(app) == Some(8));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn changing_the_colour_setting_decodes_again_and_keeps_the_edits() {
    let (mut app, path) = app_with_image();
    app.edit(Doc::rotate_left);
    let generation = app.generation;
    app.settings.color_managed = false;
    app.apply_color_setting();
    assert!(!app.cache.contains_key(&path) && app.generation > generation);
    assert!(app.doc.is_some());
}

#[test]
fn navigating_away_drops_the_question() {
    let mut app = app_in_folder();
    app.changed_prompt = true;
    app.navigate(1);
    assert!(!app.changed_prompt);
}

#[test]
fn save_copy_needs_unsaved_edits_and_writes_a_new_file() {
    let dir = temp_folder("save-copy", &[]);
    let path = dir.join("a.png");
    std::fs::write(&path, b"original").unwrap();
    let (mut app, old) = app_with_image();
    let l = app.cache.remove(&old).unwrap();
    app.cache.insert(path.clone(), l);
    app.files = vec![path.clone()];
    app.run_actions(&[Action::SaveCopy], &egui::Context::default(), false);
    assert!(app.rx.try_recv().is_err(), "no edits, nothing to write");
    app.edit(Doc::rotate_left);
    app.run_actions(&[Action::SaveCopy], &egui::Context::default(), false);
    let msg = app.rx.recv().unwrap();
    app.handle(msg);
    assert!(!app.dirty());
    assert!(image::open(dir.join("a-edited.png")).is_ok());
    assert_eq!(std::fs::read(path).unwrap(), b"original");
    std::fs::remove_dir_all(dir).unwrap();
}

/// `app_with_image` on a folder of three images, showing the middle one.
fn app_in_folder() -> App {
    let (mut app, _) = app_with_image();
    app.files = ["/a.png", "/b.png", "/c.png"].map(PathBuf::from).to_vec();
    app.index = 1;
    let frame = Frame { w: 2, h: 1, pixels: vec![0; 6], alpha: false, delay: std::time::Duration::ZERO };
    app.cache.insert("/b.png".into(), Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None }));
    app
}

#[test]
fn home_and_end_go_to_the_ends_and_stay_put_there() {
    let ctx = egui::Context::default();
    let mut app = app_in_folder();
    app.run_actions(&[Action::LastImage], &ctx, false);
    assert_eq!(app.index, 2);
    app.run_actions(&[Action::FirstImage], &ctx, false);
    assert_eq!(app.index, 0);
    app.index = 1;
    app.edit(Doc::rotate_left);
    app.run_actions(&[Action::FirstImage], &ctx, false);
    assert!(matches!(app.pending, Some(Pending::Navigate(-1))), "unsaved edits ask first");
    app.pending = None;
    // Already there: edits survive, with no question.
    app.files.truncate(2);
    app.run_actions(&[Action::LastImage], &ctx, false);
    assert!(app.pending.is_none() && app.doc.is_some() && app.index == 1);
}

#[test]
fn mouse_side_buttons_navigate_unless_a_dialog_is_open() {
    fn actions(button: egui::PointerButton, modal: bool) -> Vec<Action> {
        let ctx = egui::Context::default();
        ctx.set_fonts(markup::font_definitions());
        let mut found = Vec::new();
        for event in [egui::Event::PointerMoved(Pos2::new(9.0, 9.0)), press(button, true, 9.0)] {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
            input.events.push(event);
            let mut output = ctx.run_ui(input, |ui| {
                found = App::read_shortcuts(ui.ctx(), false, &menu::shortcuts(&[]));
                if modal {
                    egui::Modal::new(egui::Id::unique("dialog")).show(ui.ctx(), |ui| ui.label("dialog"));
                }
            });
            output.textures_delta.clear();
        }
        found
    }
    assert!(actions(egui::PointerButton::Extra1, false) == [Action::PrevImage]);
    assert!(actions(egui::PointerButton::Extra2, false) == [Action::NextImage]);
    assert!(actions(egui::PointerButton::Extra1, true).is_empty());
    assert!(actions(LEFT, false).is_empty());
}

/// One frame with `key` pressed (with `modifiers`): what the shortcuts in `keys` fire.
fn shortcut_actions(keys: &[(egui::KeyboardShortcut, Action)], key: Key, modifiers: egui::Modifiers) -> Vec<Action> {
    let ctx = egui::Context::default();
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    let mut found = Vec::new();
    let mut output = ctx.run_ui(input, |ui| found = App::read_shortcuts(ui.ctx(), false, keys));
    output.textures_delta.clear();
    found
}

#[test]
fn shortcuts_follow_the_users_changes() {
    let alt_c = egui::KeyboardShortcut::new(egui::Modifiers::ALT, Key::C);
    let changes = vec![("Crop".to_string(), Some(alt_c)), ("RotateRight".to_string(), None), ("Undo".to_string(), Some(egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::U)))];
    let keys = menu::shortcuts(&changes);
    assert!(shortcut_actions(&keys, Key::C, egui::Modifiers::ALT) == [Action::Crop]);
    assert!(shortcut_actions(&keys, Key::R, egui::Modifiers::COMMAND).is_empty(), "removed");
    assert!(shortcut_actions(&keys, Key::Z, egui::Modifiers::COMMAND).is_empty(), "Undo moved");
    assert!(shortcut_actions(&keys, Key::U, egui::Modifiers::COMMAND) == [Action::Undo]);
    assert!(shortcut_actions(&keys, Key::O, egui::Modifiers::COMMAND) == [Action::Open], "the rest stay");
    // Ctrl+Shift+Z is Redo, not Undo, whatever the order the table was built in.
    let defaults = menu::shortcuts(&[]);
    let shift = egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND };
    assert!(shortcut_actions(&defaults, Key::Z, shift) == [Action::Redo]);
    // Every default shortcut's action can be named in Settings.
    assert!(defaults.iter().all(|(_, a)| menu::bindable().iter().any(|(b, _)| b == a)));
}

#[test]
fn closing_the_shortcuts_dialog_stops_recording() {
    let (mut app, _) = app_with_image();
    (app.show_shortcuts, app.recording) = (true, Some(Action::Crop));
    let ctx = egui::Context::default();
    let frame = |app: &mut App, events: Vec<egui::Event>| {
        let input = egui::RawInput { events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            app.record_shortcut(ui.ctx());
            app.shortcuts_dialog(ui.ctx());
        });
        output.textures_delta.clear();
    };
    let esc = || egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    // Esc first cancels the recording only, then closes the dialog.
    frame(&mut app, vec![esc()]);
    assert!(app.recording.is_none() && app.show_shortcuts);
    frame(&mut app, vec![esc()]);
    assert!(!app.show_shortcuts);
}

#[test]
fn a_taken_shortcut_is_refused_and_named() {
    let (mut app, _) = app_with_image();
    let record = |app: &mut App, key: Key, modifiers: egui::Modifiers| {
        app.recording = Some(Action::Crop);
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
        let mut output = ctx.run_ui(input, |ui| {
            app.record_shortcut(ui.ctx());
            assert!(!ui.input(|i| i.key_pressed(key)), "nobody else sees the key");
        });
        output.textures_delta.clear();
    };
    record(&mut app, Key::R, egui::Modifiers::COMMAND);
    assert!(app.shortcut_error.as_ref().unwrap().ends_with(tr("Rotate Right")), "{:?}", app.shortcut_error);
    assert!(app.settings.shortcuts.is_empty() && app.recording.is_none());
    record(&mut app, Key::Delete, egui::Modifiers::NONE);
    assert!(app.shortcut_error.is_some() && app.settings.shortcuts.is_empty(), "reserved");
    record(&mut app, Key::Escape, egui::Modifiers::NONE);
    assert!(app.settings.shortcuts.is_empty() && app.recording.is_none(), "Esc cancels");
    record(&mut app, Key::K, egui::Modifiers::NONE);
    assert!(app.shortcut_error.is_none());
    assert!(menu::shortcuts(&app.settings.shortcuts).iter().any(|(sc, a)| *a == Action::Crop && sc.logical_key == Key::K));
    // Giving Crop's key to Crop again is fine.
    record(&mut app, Key::K, egui::Modifiers::NONE);
    assert!(app.shortcut_error.is_none() && app.settings.shortcuts.len() == 1);
}

#[test]
fn copy_path_puts_the_path_on_the_clipboard() {
    let (mut app, _) = app_with_image();
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.run_actions(&[Action::CopyPath], ui.ctx(), false));
    output.textures_delta.clear();
    assert!(output.platform_output.commands.contains(&egui::OutputCommand::CopyText("/a.png".into())));
}

#[test]
fn renaming_follows_the_file_and_keeps_the_edits() {
    let dir = temp_folder("rename-app", &["a.png", "taken.png"]);
    let path = dir.join("a.png");
    let (mut app, old) = app_with_image();
    let l = app.cache.remove(&old).unwrap();
    app.cache.insert(path.clone(), l);
    app.files = vec![path.clone()];
    app.recent = vec![path.clone()];
    app.edit(Doc::rotate_left);
    assert!(app.rename_current("taken.png").is_err());
    assert_eq!(app.files, [path.clone()]);
    app.rename_current("b.png").unwrap();
    let new = dir.join("b.png");
    assert_eq!((app.current(), app.recent.as_slice()), (Some(&new), std::slice::from_ref(&new)));
    assert!(app.cache.contains_key(&new) && !app.cache.contains_key(&path));
    assert!(app.dirty(), "edits stay");
    assert!(matches!(app.rx.recv().unwrap(), Msg::Folder(files) if files.contains(&new)));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_rename_dialog_replaces_the_name_keeps_the_extension_and_shows_refusals() {
    let dir = temp_folder("rename-dialog", &["a.b.png", "taken.png"]);
    let (mut app, old) = app_with_image();
    let l = app.cache.remove(&old).unwrap();
    app.cache.insert(dir.join("a.b.png"), l);
    app.files = vec![dir.join("a.b.png")];
    let ctx = test_ctx();
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
        input.events = events;
        let mut output = ctx.run_ui(input, |ui| app.rename_dialog(ui.ctx()));
        output.textures_delta.clear();
    };
    let enter = || egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    app.run_actions(&[Action::Rename], &ctx, false);
    run(&mut app, vec![]);
    run(&mut app, vec![]);
    run(&mut app, vec![egui::Event::Text("taken".into())]);
    assert_eq!(app.renaming.as_ref().unwrap().name, "taken.png", "the extension wasn't selected");
    run(&mut app, vec![enter()]);
    assert!(app.renaming.as_ref().is_some_and(|r| r.error.is_some()), "refused, and still open");
    assert!(dir.join("a.b.png").exists());
    run(&mut app, vec![]);
    run(&mut app, vec![egui::Event::Text("free".into())]);
    run(&mut app, vec![enter()]);
    assert!(app.renaming.is_none());
    assert_eq!(app.current(), Some(&dir.join("free.png")));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn f2_is_refused_while_an_ai_job_needs_the_path() {
    let (mut app, _) = app_with_image();
    app.restoring = Some(Restoring { id: 1, task: crate::ai::Task::Denoise, progress: 0.0, started: Instant::now(), cancel: Arc::default(), selection: None });
    app.run_actions(&[Action::Rename], &egui::Context::default(), false);
    assert!(app.renaming.is_none());
    app.restoring = None;
    app.run_actions(&[Action::Rename], &egui::Context::default(), false);
    assert!(app.renaming.is_some());
}

/// One frame of input, drawing and the navigator, in a 400 px viewport.
fn view_frame(app: &mut App, ctx: &egui::Context, l: &Loaded, events: Vec<egui::Event>) -> egui::FullOutput {
    let mut input = egui::RawInput::default();
    input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
    input.events = events;
    let mut output = ctx.run_ui(input, |ui| {
        let doc = app.doc();
        let viewport = ui.available_rect_before_wrap();
        app.image_input(ui, doc.as_ref().unwrap(), viewport, 1.0);
        app.draw(ui, Some(l), doc, None, viewport);
    });
    output.textures_delta.clear();
    output
}

fn click_at(app: &mut App, ctx: &egui::Context, l: &Loaded, p: Pos2) -> Vec<egui::OutputCommand> {
    let button = |pressed| egui::Event::PointerButton { pos: p, button: LEFT, pressed, modifiers: egui::Modifiers::NONE };
    let mut commands = Vec::new();
    for events in [vec![egui::Event::PointerMoved(p)], vec![], vec![button(true)], vec![button(false)], vec![]] {
        commands.extend(view_frame(app, ctx, l, events).platform_output.commands);
    }
    commands
}

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_fonts(markup::font_definitions());
    ctx
}

#[test]
fn the_navigator_shows_only_when_zoomed_in_and_isnt_rebuilt_by_panning() {
    let (mut app, _) = app_zoomed_in();
    let (ctx, l) = (test_ctx(), app.cache.values().next().unwrap().clone());
    app.settings.gpu_scaling = false;
    app.view = View::Fit;
    view_frame(&mut app, &ctx, &l, vec![]);
    assert!(app.navigator.is_none());
    app.view = crate::view::actual_size(Vec2::splat(1000.0));
    view_frame(&mut app, &ctx, &l, vec![]);
    let first = app.navigator.as_ref().unwrap().1.id();
    app.view = view::pan(app.view, Vec2::splat(50.0), Vec2::splat(1000.0), Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)), 1.0);
    view_frame(&mut app, &ctx, &l, vec![]);
    assert_eq!(app.navigator.as_ref().unwrap().1.id(), first);
    app.edit(Doc::rotate_right);
    app.view = crate::view::actual_size(Vec2::splat(1000.0));
    view_frame(&mut app, &ctx, &l, vec![]);
    assert_ne!(app.navigator.as_ref().unwrap().1.id(), first, "it follows the edits");
}

#[test]
fn clicking_the_navigator_moves_the_view_and_not_the_tool() {
    let (mut app, _) = app_zoomed_in();
    let (ctx, l) = (test_ctx(), app.cache.values().next().unwrap().clone());
    app.tool = Tool::Draw(Shape::Rect);
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0));
    let nav = view::navigator_rect(Vec2::splat(1000.0), viewport);
    click_at(&mut app, &ctx, &l, nav.right_bottom() - Vec2::splat(10.0));
    let (_, crop) = view::layout(app.view, Vec2::splat(1000.0), viewport, 1.0);
    assert!(crop.max.x > 999.0 && crop.max.y > 999.0, "{crop:?}");
    assert!(app.doc().unwrap().elements.is_empty());
}

#[test]
fn the_navigator_follows_a_rotated_image() {
    let (mut app, path) = app_with_image();
    let frame = Frame { w: 1000, h: 500, pixels: vec![0; 1_500_000], alpha: false, delay: std::time::Duration::ZERO };
    let l = Arc::new(Loaded { frames: vec![frame], file_size: 6, svg: None });
    app.cache.insert(path, l.clone());
    app.edit(Doc::rotate_right);
    let (size, viewport) = (Vec2::new(500.0, 1000.0), Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)));
    app.view = crate::view::actual_size(size);
    let ctx = test_ctx();
    let nav = view::navigator_rect(size, viewport);
    assert!(nav.height() > nav.width());
    click_at(&mut app, &ctx, &l, nav.right_bottom() - Vec2::splat(2.0));
    let (_, crop) = view::layout(app.view, size, viewport, 1.0);
    assert!(crop.max.x > 499.0 && crop.max.y > 999.0, "{crop:?}");
}

/// `app_with_image` with a transparent pixel on the left and a red one on the right, picking.
fn app_picking() -> (App, Arc<Loaded>) {
    let (mut app, path) = app_with_image();
    let frame = Frame { w: 2, h: 1, pixels: vec![0, 0, 255, 128, 255, 16, 0, 255], alpha: true, delay: std::time::Duration::ZERO };
    let l = Arc::new(Loaded { frames: vec![frame], file_size: 8, svg: None });
    app.cache.insert(path, l.clone());
    app.run_actions(&[Action::CopyColor], &egui::Context::default(), false);
    assert!(app.tool == Tool::CopyColor);
    (app, l)
}

#[test]
fn copy_color_copies_the_pixel_clicked() {
    let (mut app, l) = app_picking();
    let ctx = test_ctx();
    let copied = |text: &str| egui::OutputCommand::CopyText(text.into());
    assert!(click_at(&mut app, &ctx, &l, Pos2::new(300.0, 200.0)).contains(&copied("#FF1000")));
    assert!(click_at(&mut app, &ctx, &l, Pos2::new(100.0, 200.0)).contains(&copied("#0000FF80")), "not opaque");
    // What's seen, not the file: here flipped.
    app.edit(Doc::flip_horizontal);
    assert!(click_at(&mut app, &ctx, &l, Pos2::new(100.0, 200.0)).contains(&copied("#FF1000")));
    press_key(&mut app, Key::Escape);
    assert!(app.tool == Tool::Pan);
}

#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Err("unused".into())
    }
}

#[test]
fn a_dropped_file_reaches_the_app_through_egui() {
    let dir = temp_folder("drop-egui", &["b.png"]);
    let (mut app, _) = app_with_image();
    let ctx = egui::Context::default();
    ctx.set_fonts(markup::font_definitions());
    let mut input = egui::RawInput::default();
    input.dropped_files.push(Arc::new(Dropped(dir.join("b.png"))));
    frame(&mut app, &ctx, input);
    assert_eq!(app.files, [dir.join("b.png")]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn recent_list() {
    let mut recent = Vec::new();
    for p in ["/a", "/b", "/a"] {
        push_recent(&mut recent, std::path::Path::new(p));
    }
    assert_eq!(recent, [PathBuf::from("/a"), PathBuf::from("/b")]);
    for i in 0..20 {
        push_recent(&mut recent, std::path::Path::new(&format!("/{i}")));
    }
    assert_eq!(recent.len(), RECENT_MAX);
    assert_eq!(recent[0], PathBuf::from("/19"));
}

#[test]
fn shortcuts_obey_the_menus_rules() {
    let (mut app, path) = app_with_image();
    let ctx = egui::Context::default();
    // Adjust Color is open: Greyscale would be lost on Done, and Perspective is greyed out.
    app.adjusting = app.doc();
    app.run_actions(&[Action::Greyscale, Action::Perspective, Action::Straighten], &ctx, false);
    assert!(app.doc().unwrap().adjust.is_identity() && app.tool == Tool::Pan);
    // An animation can't be warped.
    app.adjusting = None;
    let frame = app.cache.get(&path).unwrap().frames[0].clone();
    app.cache.insert(path, Arc::new(Loaded { frames: vec![frame.clone(), frame], file_size: 6, svg: None }));
    app.run_actions(&[Action::Perspective], &ctx, false);
    assert!(app.tool == Tool::Pan);
}
