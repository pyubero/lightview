use super::*;

/// The licence prompt's body: Some(true) to accept and download, Some(false) to cancel.
/// One licence of a download (`step` of `count`, from 0). Some(true) accepts it, Some(false) cancels.
/// The text scrolls within the window's height, so the buttons stay on screen.
pub fn license_ui(ui: &mut egui::Ui, title: &str, l: &License, step: usize, count: usize, note: Option<&str>) -> Option<bool> {
    ui.heading(if count > 1 { trf("Licence ({step} of {count}) — {title}", &[("step", &(step + 1)), ("count", &count), ("title", &title)]) } else { trf("Licence — {title}", &[("title", &title)]) });
    ui.label(tr("Please read and accept the licence before downloading."));
    if let Some(note) = note {
        ui.weak(note);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.strong(format!("{} — {}", l.project, l.spdx));
        if ui.button(tr("Open project page")).clicked() {
            crate::platform::open(l.url);
        }
    });
    // Room for the heading, notes and buttons around the text.
    let height = (ui.ctx().content_rect().height() - 260.0).clamp(100.0, 360.0);
    egui::ScrollArea::vertical().id_salt(("license", step)).max_height(height).show(ui, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(l.text).monospace()).wrap());
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let accept = ui.button(if step + 1 < count { tr("Accept and Continue") } else { tr("Accept and Download") }).clicked();
        let cancel = ui.button(tr("Cancel")).clicked();
        accept.then_some(true).or(cancel.then_some(false))
    })
    .inner
}

/// The GPU rows of the AI Models dialog: NVIDIA (CUDA) and any other card (WebGPU).
fn gpu_ui(ui: &mut egui::Ui, use_gpu: &mut bool, busy: &HashSet<String>, runtime: bool, out: &mut ModelsOut) {
    let button = |ui: &mut egui::Ui, downloading: bool| ui.add_enabled(!downloading, egui::Button::new(if downloading { tr("Downloading…") } else { tr("Download GPU Support") })).clicked();
    if nvidia_driver() || gpu_installed() {
        ui.add_space(4.0);
        ui.strong(tr("GPU (NVIDIA)"));
        if !gpu_installed() {
            ui.horizontal(|ui| {
                ui.label(trf("Run the AI tools on the graphics card ({mb} MB download, NVIDIA's CUDA libraries).", &[("mb", &GPU_DOWNLOAD_MB)]));
                out.download_gpu = button(ui, busy.contains("gpu"));
            });
        }
    }
    // The plugin runs on the AI runtime, so it needs that first.
    if runtime {
        ui.add_space(4.0);
        ui.strong(tr("GPU (AMD, Intel and others, experimental)"));
        if !webgpu_installed() {
            ui.horizontal(|ui| {
                ui.label(trf("Run the AI tools on any graphics card with Vulkan or Direct3D 12 ({mb} MB download, WebGPU). A weak built-in graphics chip can be slower than the CPU.", &[("mb", &WEBGPU_DOWNLOAD_MB)]));
                out.download_webgpu = button(ui, busy.contains("webgpu"));
            });
        }
    }
    if gpu_installed() || webgpu_installed() {
        ui.checkbox(use_gpu, tr("Use the GPU")).on_hover_text(tr("Denoise and Enhance run about 10× faster on a GTX 1050"));
        if gpu_needs_restart() {
            ui.colored_label(ui.visuals().warn_fg_color, tr("Restart lightview to start using the GPU."));
        } else if let Some(e) = gpu_error().filter(|_| *use_gpu) {
            ui.colored_label(ui.visuals().warn_fg_color, trf("The GPU couldn't be used, so the CPU is: {e}", &[("e", &short_error(&e))]));
        }
    }
}

/// The paragraphs of the notice shown before an Enhance model is first used.
pub fn enhance_notice(file: &str) -> Vec<String> {
    const GENERATIVE: &str = tr_key("Enhance Image uses generative AI. To make the image look better it may invent or alter fine details, such as textures, text or faces, that weren't in the original.");
    let k = known(file);
    let mut lines = vec![tr(k.and_then(|k| k.notice).unwrap_or(GENERATIVE)).to_string()];
    if let Some(k) = k.filter(|k| k.scale > 1) {
        lines.push(trf("It also makes the image {k}× bigger ({pixels}× the pixels), which takes a while for a big photo.", &[("k", &k.scale), ("pixels", &(k.scale * k.scale))]));
    }
    lines.push(tr("For a faithful clean-up, use Denoise instead.").into());
    lines
}

/// What the AI Models dialog asks for this frame.
#[derive(Default)]
pub struct ModelsOut {
    pub download_gpu: bool,
    pub download_webgpu: bool,
    /// An index into `CATALOG`.
    pub download: Option<usize>,
    /// Read this model's input size.
    pub detect: Option<String>,
    pub add: bool,
    pub close: bool,
}

/// The AI Models dialog body: installed models (which tool uses which, and their
/// preprocessing) and the catalog to download from. `busy` holds files being downloaded.
pub fn models_ui(ui: &mut egui::Ui, s: &mut AiSettings, installed: &[String], busy: &HashSet<String>, runtime: bool) -> ModelsOut {
    let mut out = ModelsOut::default();
    ui.heading(tr("AI Models"));
    if !runtime {
        ui.colored_label(ui.visuals().warn_fg_color, tr("The AI runtime isn't installed yet: AI > Download AI Runtime…"));
    }
    gpu_ui(ui, &mut s.use_gpu, busy, runtime, &mut out);
    let name = |f: &str| {
        let mb = models_dir().ok().and_then(|d| std::fs::metadata(d.join(f)).ok()).map_or(0, |m| m.len().div_ceil(1_000_000));
        match known(f) {
            Some(k) => format!("{} ({} MB)", tr(k.label), k.mb),
            None => format!("{f} ({mb} MB)"),
        }
    };
    egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
        for task in Task::ALL {
            ui.add_space(8.0);
            ui.strong(task.title());
            let models = for_task(task, installed);
            let current = choose(s.choice(task), task, installed);
            if models.is_empty() {
                ui.weak(tr("No model installed yet."));
            } else {
                ui.horizontal(|ui| {
                    ui.label(tr("Model"));
                    egui::ComboBox::from_id_salt(("ai-model", task.title())).selected_text(current.as_deref().map(name).unwrap_or_default()).show_ui(ui, |ui| {
                        for f in &models {
                            if ui.selectable_label(current.as_ref() == Some(f), name(f)).on_hover_text(f).clicked() {
                                *s.choice_mut(task) = Some(f.clone());
                            }
                        }
                    });
                });
                if task == Task::Background {
                    ui.weak(tr("Used by Remove Background and Remove Background and Crop."));
                    for f in &models {
                        egui::CollapsingHeader::new(trf("Preprocessing: {file}", &[("file", &f)])).id_salt(("pre", f)).show(ui, |ui| preprocess_ui(ui, s, f, runtime, &mut out));
                    }
                }
            }
            if task == Task::Enhance {
                ui.weak(tr("Real-ESRGAN and DarkIR can't be downloaded here. Convert them yourself with lightview-models, then copy the .onnx files into the models folder:"));
                ui.hyperlink(LIGHTVIEW_MODELS_URL);
            }
            let mut available: Vec<(usize, &Known)> = CATALOG.iter().enumerate().filter(|(_, k)| k.task == task && !installed.iter().any(|f| f == k.file)).collect();
            available.sort_by_key(|(_, k)| k.mb);
            egui::Grid::new(("catalog-grid", task.title())).num_columns(3).spacing([16.0, 6.0]).show(ui, |ui| {
                for (i, k) in available {
                    ui.label(tr(k.label));
                    ui.label(format!("{} MB", k.mb));
                    let downloading = busy.contains(k.file);
                    if ui.add_enabled(!downloading, egui::Button::new(if downloading { tr("Downloading…") } else { tr("Download") })).clicked() {
                        out.download = Some(i);
                    }
                    ui.end_row();
                }
            });
        }
    });
    ui.add_space(8.0);
    ui.weak(tr("Only add models from sources you trust: they run with your permissions."));
    ui.horizontal(|ui| {
        out.add = ui.button(tr("Add model file…")).clicked();
        if ui.button(tr("Open models folder")).clicked() {
            open_folder();
        }
        out.close = ui.button(tr("Close")).clicked();
    });
    out
}

/// One model's preprocessing; any change becomes that file's override.
fn preprocess_ui(ui: &mut egui::Ui, s: &mut AiSettings, f: &str, runtime: bool, out: &mut ModelsOut) {
    let current = preset(f, &s.overrides);
    if current.is_none() {
        ui.weak(tr("Size read from the model when it runs; the usual normalisation."));
    }
    let mut p = current.unwrap_or(Preprocess::auto(1024));
    let before = p;
    egui::Grid::new(("pre-grid", f)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label(tr("Input size"));
        ui.add(egui::DragValue::new(&mut p.size).range(32..=4096).suffix(" px"));
        ui.end_row();
        for (label, values, range) in [(tr("Mean (R, G, B)"), &mut p.mean, 0.0..=1.0), (tr("Std (R, G, B)"), &mut p.std, 0.001..=10.0)] {
            ui.label(label);
            ui.horizontal(|ui| {
                for v in values.iter_mut() {
                    ui.add(egui::DragValue::new(v).range(range.clone()).speed(0.001).fixed_decimals(3));
                }
            });
            ui.end_row();
        }
        ui.label(tr("Output"));
        ui.checkbox(&mut p.sigmoid, tr("Apply sigmoid (the model outputs logits)"));
        ui.end_row();
    });
    if p != before {
        s.overrides.insert(f.to_string(), p);
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(runtime, egui::Button::new(tr("Detect size"))).on_hover_text(tr("Read the input size from the model")).clicked() {
            out.detect = Some(f.to_string());
        }
        if ui.add_enabled(s.overrides.contains_key(f), egui::Button::new(tr("Reset to default"))).clicked() {
            s.overrides.remove(f);
        }
    });
}
