// Everything here is test code: the i18n test reads source only up to this marker.
// #[cfg(test)]

use super::*;

#[test]
fn mask_helpers() {
    // A 2×2 prediction: one strong pixel, scaled up to 4×4.
    let mask = to_mask(&[0.1, 0.1, 0.1, 0.9], 2, false, 4, 4);
    assert_eq!(mask.get_pixel(3, 3)[0], 255);
    assert_eq!(mask.get_pixel(0, 0)[0], 0);
    let b = bbox(&mask).unwrap();
    assert!(b.min.x >= 1.0 && b.max == Pos2::new(4.0, 4.0), "{b:?}");
    assert_eq!(bbox(&GrayImage::new(3, 3)), None);
    let img = RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 255]));
    let out = cutout(&img, &mask);
    assert_eq!(out.get_pixel(3, 3).0, [9, 9, 9, 255]);
    assert_eq!(out.get_pixel(0, 0)[3], 0);
}

#[test]
fn input_layout() {
    // A red 1×1 image becomes three planes: R high, G and B low.
    let img = RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
    let input = input(&Preprocess { size: 2, ..U2NET }, &img);
    assert_eq!(input.len(), 3 * 4);
    assert!(input[..4].iter().all(|&v| v > 2.0) && input[4..].iter().all(|&v| v < -1.0));
}

#[test]
fn every_download_has_its_licence() {
    for k in &CATALOG {
        assert!(!k.licenses.is_empty(), "{}", k.file);
    }
    assert_eq!(CATALOG[0].file, "silueta.onnx");
    assert_eq!(CATALOG[0].licenses.iter().map(|l| l.project).collect::<Vec<_>>(), ["U²-Net", "rembg"]);
    for l in CATALOG.iter().flat_map(|k| k.licenses).chain([&RUNTIME_LICENSE]) {
        let ok = match l.spdx {
            "Apache-2.0" => l.text.contains("Apache License") && l.text.contains("Version 2.0"),
            "MIT" => l.text.contains("Permission is hereby granted") && l.text.contains("Copyright"),
            other => panic!("unexpected licence {other}"),
        };
        assert!(ok, "{} text doesn't look like {}", l.project, l.spdx);
    }
}

fn gradient(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x + y) % 256) as u8, (x % 256) as u8]))
}

#[test]
fn tiles_cover_and_blend() {
    let no = AtomicBool::new(false);
    // Identity model: any tiling gives the input back, alpha included.
    for (w, h, tile, overlap) in [(70, 45, 32, 4), (300, 17, 64, 8), (20, 20, 64, 8), (128, 128, 64, 8)] {
        let img = gradient(w, h);
        let out = tiled(&img, tile, overlap, |d, _| Ok(d), &|_| {}, &no).unwrap();
        assert_eq!(out, img, "{w}×{h} in {tile}px tiles");
    }
    // A model that brightens: every pixel is written once, by exactly one tile.
    // 102 is 0.4; 0.4 + 0.2 is 153.
    let img = RgbaImage::from_pixel(100, 60, image::Rgba([102, 102, 102, 255]));
    let out = tiled(&img, 32, 4, |d, _| Ok(d.iter().map(|v| v + 0.2).collect()), &|_| {}, &no).unwrap();
    assert!(out.pixels().all(|p| p.0 == [153, 153, 153, 255]));
}

#[test]
fn tiles_upscale() {
    // A fake 2× model (each pixel becomes a 2×2 block): the result is the image, 2× bigger.
    let double = |d: Vec<f32>, t: usize| -> Result<Vec<f32>, String> {
        let (p, t2) = (t * t, 2 * t);
        Ok((0..3).flat_map(|c| (0..t2 * t2).map(move |i| (c, i))).map(|(c, i)| d[c * p + (i / t2 / 2) * t + (i % t2) / 2]).collect())
    };
    let no = AtomicBool::new(false);
    let img = RgbaImage::from_fn(70, 45, |x, y| image::Rgba([(x * 3) as u8, (y * 5) as u8, 9, 255]));
    let out = tiled(&img, 32, 4, double, &|_| {}, &no).unwrap();
    assert_eq!(out.dimensions(), (140, 90));
    let expected = image::imageops::resize(&img, 140, 90, FilterType::Nearest);
    assert_eq!(out, expected);
    // Alpha is scaled up too.
    let clear = RgbaImage::from_fn(40, 20, |x, _| image::Rgba([1, 2, 3, if x < 20 { 0 } else { 255 }]));
    let out = tiled(&clear, 32, 4, double, &|_| {}, &no).unwrap();
    assert_eq!((out.get_pixel(2, 2)[3], out.get_pixel(77, 37)[3]), (0, 255));
    // An output that's no whole multiple of the input is refused.
    assert!(tiled(&img, 32, 4, |d, _| Ok(d[..10].to_vec()), &|_| {}, &no).is_err());
}

#[test]
fn tiles_report_progress_and_cancel() {
    let img = gradient(100, 100);
    let seen = std::cell::RefCell::new(Vec::new());
    tiled(&img, 64, 8, |d, _| Ok(d), &|p| seen.borrow_mut().push(p), &AtomicBool::new(false)).unwrap();
    assert_eq!(*seen.borrow(), [0.25, 0.5, 0.75, 1.0]);
    // Cancelled after the first tile.
    let cancel = AtomicBool::new(false);
    let mut runs = 0;
    let result = tiled(&img, 64, 8, |d, _| { runs += 1; cancel.store(true, Ordering::Relaxed); Ok(d) }, &|_| {}, &cancel);
    assert_eq!((result, runs), (Err("cancelled".into()), 1));
    // A failing model stops the run.
    assert!(tiled(&img, 64, 8, |_, _| Err("boom".into()), &|_| {}, &AtomicBool::new(false)).is_err());
}

#[test]
fn catalog_entries() {
    for k in &CATALOG {
        assert_eq!(k.pre.is_some(), k.task == Task::Background, "{}", k.file);
        assert_eq!(k.sha256.len(), 64, "{}", k.file);
        for (data, sha) in k.data {
            assert_eq!(sha.len(), 64, "{data}");
            assert!(k.task == Task::Ocr || *data == format!("{}.data", k.file), "{data}");
        }
    }
    // Each task's first entry is its default.
    assert_eq!(CATALOG.iter().find(|k| k.task == Task::Denoise).unwrap().file, "scunet_color_real_psnr.onnx");
    assert_eq!(CATALOG.iter().find(|k| k.task == Task::Enhance).unwrap().file, "BSRGANx4_fp16.onnx");
    assert_eq!(CATALOG.iter().find(|k| k.task == Task::Inpaint).unwrap().file, LAMA_FILE);
    assert!(CATALOG.iter().all(|k| (k.scale > 1) == (k.file == "BSRGANx4_fp16.onnx")));
}

#[test]
fn two_file_download_is_all_or_nothing() {
    let root = std::env::temp_dir().join(format!("lightview-dl-{}", std::process::id()));
    let (src, dest) = (root.join("src"), root.join("models"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("m.onnx"), b"graph").unwrap();
    std::fs::write(src.join("m.onnx.data"), b"weights").unwrap();
    let sha = |f: &str| sha256_file(&src.join(f)).unwrap();
    let (graph, weights) = (sha("m.onnx").leak() as &str, sha("m.onnx.data").leak() as &str);
    let base = file_url(&src).leak();
    let known = |data_sha: &'static str| Known { file: "m.onnx", label: "", task: Task::Denoise, mb: 0, sha256: graph, base, data: vec![("m.onnx.data", data_sha)].leak(), pre: None, converted_by: None, scale: 1, max_tile: 512, notice: None, licenses: &[] };
    // A bad weights checksum installs nothing, not even the good .onnx.
    assert!(download_to(&dest, &known("0000")).is_err());
    let leftovers: Vec<_> = std::fs::read_dir(&dest).unwrap().collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    // Both good: both installed, and listed.
    download_to(&dest, &known(weights)).unwrap();
    let ok = dest.join("m.onnx.data").is_file() && list_models(&dest) == ["m.onnx"];
    std::fs::remove_dir_all(&root).unwrap();
    assert!(ok);
}

#[test]
fn short_gpu_errors() {
    let trace = "/onnxruntime_src/onnxruntime/core/providers/cuda/cuda_call.cc:154 std::conditional_t<THRW, void> onnxruntime::CudaCall(…) CUDA failure 100: no CUDA-capable device is detected ; GPU=-1 ; hostname=x ; file=cuda_execution_provider.cc";
    assert_eq!(short_error(trace), "CUDA failure 100: no CUDA-capable device is detected");
    assert_eq!(short_error("libcudnn.so.9: not found"), "libcudnn.so.9: not found");
    assert!(short_error(&"x".repeat(500)).chars().count() <= 161);
}

#[test]
fn enhance_notices() {
    let bsrgan = enhance_notice("BSRGANx4_fp16.onnx");
    assert!(bsrgan[0].contains("generative") && bsrgan[1].contains("4× bigger (16× the pixels)"));
    // An unknown model: the generative warning, no size line.
    let mine = enhance_notice("mine.onnx");
    assert_eq!(mine.len(), 2);
    assert!(mine[0].contains("generative") && !mine.iter().any(|l| l.contains("bigger")));
}

#[test]
fn sigmoid_logits() {
    // Logits −4, 0, 4: with the sigmoid the middle one is half-way; without it, too (min–max).
    let mask = to_mask(&[-4.0, 0.0, 4.0, 4.0], 2, true, 2, 2);
    assert_eq!((mask.get_pixel(0, 0)[0], mask.get_pixel(1, 0)[0], mask.get_pixel(0, 1)[0]), (0, 128, 255));
    // A logit far past the others saturates under the sigmoid instead of squashing the rest.
    let squashed = to_mask(&[-6.0, 6.0, 100.0, 100.0], 2, false, 2, 2);
    let kept = to_mask(&[-6.0, 6.0, 100.0, 100.0], 2, true, 2, 2);
    assert!(kept.get_pixel(1, 0)[0] > 250 && squashed.get_pixel(1, 0)[0] < 40);
}

#[test]
fn choosing_models() {
    let installed = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    let all = installed(&["isnet-general-use.onnx", "mine.onnx", "silueta.onnx", "u2netp.onnx"]);
    // Default: the catalog's preferred model.
    assert_eq!(choose(None, Task::Background, &all).as_deref(), Some("silueta.onnx"));
    assert_eq!(choose(None, Task::Background, &installed(&["isnet-general-use.onnx", "u2netp.onnx"])).as_deref(), Some("u2netp.onnx"));
    // The user's choice wins while it's installed.
    assert_eq!(choose(Some(&"mine.onnx".into()), Task::Background, &all).as_deref(), Some("mine.onnx"));
    assert_eq!(choose(Some(&"gone.onnx".into()), Task::Background, &all).as_deref(), Some("silueta.onnx"));
    // No catalog model: any installed model; nothing installed: none.
    assert_eq!(choose(None, Task::Background, &installed(&["mine.onnx", "zzz.onnx"])).as_deref(), Some("mine.onnx"));
    assert_eq!(choose(None, Task::Background, &[]), None);
    // Denoise / Enhance: their own catalog models, never a background model.
    let mixed = installed(&["BSRGANx4_fp16.onnx", "mine.onnx", "scunet_color_15.onnx", "scunet_color_real_psnr.onnx", "silueta.onnx"]);
    assert_eq!(choose(None, Task::Denoise, &mixed).as_deref(), Some("scunet_color_real_psnr.onnx"));
    assert_eq!(choose(None, Task::Enhance, &mixed).as_deref(), Some("BSRGANx4_fp16.onnx"));
    assert_eq!(choose(None, Task::Denoise, &installed(&["silueta.onnx", "mine.onnx"])), None);
    assert_eq!(choose(Some(&"mine.onnx".into()), Task::Denoise, &mixed).as_deref(), Some("mine.onnx"));
    assert_eq!(for_task(Task::Enhance, &mixed), ["BSRGANx4_fp16.onnx", "mine.onnx"]);
}

#[test]
fn inpaint_only_uses_its_own_models() {
    let all = ["lama_fp32.onnx", "migan_pipeline_v2.onnx", "mine.onnx", "silueta.onnx"].map(String::from);
    assert_eq!(for_task(Task::Inpaint, &all), ["lama_fp32.onnx", "migan_pipeline_v2.onnx"]);
    assert_eq!(choose(None, Task::Inpaint, &all).as_deref(), Some("lama_fp32.onnx"));
    assert_eq!(choose(None, Task::Inpaint, &all[1..]).as_deref(), Some("migan_pipeline_v2.onnx"));
    assert_eq!(choose(None, Task::Inpaint, &all[2..]), None);
    assert!(for_task(Task::Background, &all).contains(&"mine.onnx".to_string()));
}

#[test]
fn inpaint_context() {
    let r = |x0, y0, x1, y1| Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1));
    // Centred, 512 px at least.
    assert_eq!(context_rect(r(1000.0, 1000.0, 1100.0, 1100.0), 4000, 3000), (794, 794, 512, 512));
    // Big selections keep half their size as context.
    assert_eq!(context_rect(r(1000.0, 1000.0, 1800.0, 1400.0), 4000, 3000), (800, 600, 1200, 1200));
    // Moved in at the edges, not cut.
    assert_eq!(context_rect(r(0.0, 0.0, 10.0, 10.0), 4000, 3000), (0, 0, 512, 512));
    assert_eq!(context_rect(r(3990.0, 2990.0, 4000.0, 3000.0), 4000, 3000), (3488, 2488, 512, 512));
    // An image smaller than the square, and the whole image.
    assert_eq!(context_rect(r(10.0, 10.0, 20.0, 20.0), 300, 2000), (0, 0, 300, 512));
    assert_eq!(context_rect(r(0.0, 0.0, 4000.0, 3000.0), 4000, 3000), (0, 0, 4000, 3000));
}

#[test]
fn inpaint_fills_the_region() {
    let img = RgbaImage::from_fn(600, 600, |x, _| image::Rgba([9, 9, 9, if x < 300 { 255 } else { 40 }]));
    let region = Rect::from_min_max(Pos2::new(290.0, 100.0), Pos2::new(310.0, 110.0));
    let hole = GrayImage::from_pixel(20, 10, image::Luma([255]));
    let green = |rgb: image::RgbImage, mask: GrayImage| {
        // Binary, and at the context's own size when none is asked for.
        assert!(mask.pixels().all(|m| m[0] == 0 || m[0] == 255));
        assert_eq!(rgb.dimensions(), mask.dimensions());
        Ok(image::RgbImage::from_pixel(rgb.width(), rgb.height(), image::Rgb([0, 200, 0])))
    };
    for size in [None, Some(512), Some(256)] {
        let out = fill_context(&img, region, &hole, size, green).unwrap();
        assert_eq!(out.dimensions(), (20, 10));
        // RGB from the model, alpha from the image.
        assert_eq!(out.get_pixel(0, 5).0, [0, 200, 0, 255]);
        assert_eq!(out.get_pixel(19, 5).0, [0, 200, 0, 40]);
    }
    // The hole is where the region is, within the context.
    let (x, y, ..) = context_rect(region, 600, 600);
    fill_context(&img, region, &hole, None, |rgb, mask| {
        assert_eq!(mask.get_pixel(290 - x, 100 - y)[0], 255);
        assert_eq!((mask.get_pixel(289 - x, 100 - y)[0], mask.get_pixel(310 - x, 100 - y)[0]), (0, 0));
        Ok(rgb)
    }).unwrap();
    // One pixel of a much bigger context survives shrinking.
    let big = RgbaImage::new(4000, 3000);
    let dot = Rect::from_min_size(Pos2::new(2000.0, 1500.0), egui::vec2(1600.0, 1.0));
    let line = GrayImage::from_pixel(1600, 1, image::Luma([255]));
    fill_context(&big, dot, &line, Some(512), |rgb, mask| {
        assert!(mask.pixels().any(|m| m[0] == 255));
        Ok(rgb)
    }).unwrap();
    // A model returning the wrong size is refused, not a panic.
    assert!(fill_context(&img, region, &hole, None, |_, _| Ok(image::RgbImage::new(3, 3))).is_err());
    assert!(from_planes(&[0; 5], 2, 2).is_err());
    assert_eq!(from_planes(&to_planes(&image::RgbImage::from_pixel(2, 1, image::Rgb([1, 2, 3]))), 2, 1).unwrap().get_pixel(1, 0).0, [1, 2, 3]);
}

/// `LIGHTVIEW_AI=photo.jpg LIGHTVIEW_AI_MODEL=<file> cargo test --release inpaint_photo -- --ignored --nocapture`
/// (needs the runtime): fills an ellipse inside `LIGHTVIEW_AI_BOX=x,y,w,h` (by default the middle fifth
/// of the image) and writes `photo-<model>-inpainted.png`, the whole image, to the temp folder.
#[test]
#[ignore]
fn inpaint_photo() {
    let path = PathBuf::from(std::env::var("LIGHTVIEW_AI").unwrap());
    let file = std::env::var("LIGHTVIEW_AI_MODEL").unwrap();
    let mut img = image::open(&path).unwrap().into_rgba8();
    let (w, h) = (img.width(), img.height());
    let b: Vec<u32> = std::env::var("LIGHTVIEW_AI_BOX").map_or(vec![w * 2 / 5, h * 2 / 5, w / 5, h / 5], |s| s.split(',').map(|v| v.parse().unwrap()).collect());
    let region = Rect::from_min_size(Pos2::new(b[0] as f32, b[1] as f32), egui::vec2(b[2] as f32, b[3] as f32));
    let oval = crate::markup::Selection::Ellipse(region);
    let hole = crate::markup::polygon_mask(&oval.outline(), None, region.min, 1.0, b[2], b[3]).unwrap();
    let hole = GrayImage::from_raw(b[2], b[3], hole.data().to_vec()).unwrap();
    // The second run reuses the loaded model.
    let t = std::time::Instant::now();
    inpaint(&file, &img, region, &hole, &AtomicBool::new(false)).unwrap();
    eprintln!("{file}: first run {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    let t = std::time::Instant::now();
    let filled = inpaint(&file, &img, region, &hole, &AtomicBool::new(false)).unwrap();
    eprintln!("{file}: filled {}×{} in {:.0} ms, GPU {} ({:?})", b[2], b[3], t.elapsed().as_secs_f64() * 1000.0, on_gpu(), gpu_error());
    input_size(&file).unwrap();
    assert!(INPAINTER.lock().unwrap().is_none(), "another session frees the kept one");
    for (x, y, px) in filled.enumerate_pixels() {
        if hole.get_pixel(x, y)[0] > 0 {
            *img.get_pixel_mut(b[0] + x, b[1] + y) = *px;
        }
    }
    let out = std::env::temp_dir().join(format!("{}-{file}-inpainted.png", path.file_stem().unwrap().to_string_lossy()));
    img.save(&out).unwrap();
    eprintln!("wrote {}", out.display());
}

#[test]
fn self_converted_models() {
    for k in &SELF_CONVERTED {
        assert!(CATALOG.iter().all(|c| c.file != k.file), "{}", k.file);
        let alone = [k.file.to_string()];
        assert_eq!(for_task(Task::Enhance, &alone), alone);
        assert!(for_task(Task::Background, &alone).is_empty() && for_task(Task::Denoise, &alone).is_empty());
        assert_eq!(choose(None, Task::Enhance, &alone).as_deref(), Some(k.file));
        assert_eq!(choose(None, Task::Background, &alone), None);
        assert_eq!(k.max_tile, if k.scale > 1 { 256 } else { 1024 }, "{}", k.file);
        assert_eq!(enhance_notice(k.file).iter().any(|l| l.contains("4× bigger")), k.scale == 4, "{}", k.file);
    }
    // The catalog's model stays the default, and DarkIR is never preferred over an upscaler.
    let all: Vec<String> = SELF_CONVERTED.iter().map(|k| k.file.to_string()).chain(["BSRGANx4_fp16.onnx".to_string()]).collect();
    assert_eq!(choose(None, Task::Enhance, &all).as_deref(), Some("BSRGANx4_fp16.onnx"));
    assert_eq!(choose(None, Task::Enhance, &all[..5]).as_deref(), Some("RealESRGAN_x4plus_fp32.onnx"));
}

#[test]
fn presets_and_overrides() {
    let mut overrides = HashMap::new();
    assert_eq!(preset("u2netp.onnx", &overrides), Some(U2NET));
    assert_eq!(preset("BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx", &overrides).map(|p| p.sigmoid), Some(true));
    assert_eq!(preset("mine.onnx", &overrides), None, "unknown: size comes from the model");
    overrides.insert("u2netp.onnx".to_string(), Preprocess::auto(512));
    assert_eq!(preset("u2netp.onnx", &overrides).map(|p| p.size), Some(512));
}

#[test]
fn lists_only_onnx_files() {
    let dir = std::env::temp_dir().join(format!("lightview-models-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for f in ["b.onnx", "a.ONNX", "c.onnx.part", "notes.txt"] {
        std::fs::write(dir.join(f), b"").unwrap();
    }
    let listed = list_models(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(listed, ["a.ONNX", "b.onnx"]);
}

/// Needs the runtime: `cargo test input_sizes -- --ignored` (silueta and isnet installed).
#[test]
#[ignore]
fn input_sizes() {
    assert_eq!(input_size("silueta.onnx"), Ok(320));
    assert_eq!(input_size("isnet-general-use.onnx"), Ok(1024));
}

/// `LIGHTVIEW_AI=photo.png LIGHTVIEW_AI_MODEL=<file or path> cargo test --release restore_photo -- --ignored --nocapture`
/// (needs the runtime); writes `photo-restored.png` to the temp folder.
#[test]
#[ignore]
fn restore_photo() {
    let path = PathBuf::from(std::env::var("LIGHTVIEW_AI").unwrap());
    let img = image::open(&path).unwrap().into_rgba8();
    let t = std::time::Instant::now();
    let out = restore(&std::env::var("LIGHTVIEW_AI_MODEL").unwrap(), &img, &|_| {}, &AtomicBool::new(false)).unwrap();
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!("restored {}×{} in {ms:.0} ms ({:.0} ms/MP)", img.width(), img.height(), ms / (img.width() * img.height()) as f64 * 1e6);
    let out_path = std::env::temp_dir().join(format!("{}-restored.png", path.file_stem().unwrap().to_string_lossy()));
    out.save(&out_path).unwrap();
    eprintln!("wrote {}", out_path.display());
}

/// Installs the GPU support the way the menu does (~1.5 GB): `cargo test install_gpu -- --ignored`.
#[test]
#[ignore]
fn install_gpu() {
    download_gpu().unwrap();
    assert!(gpu_installed());
}

/// With the GPU support installed: `LIGHTVIEW_AI=crop.png LIGHTVIEW_AI_MODEL=<file or path>
/// cargo test --release gpu_restore -- --ignored --nocapture`. Runs on the GPU, then with
/// the GPU switched off, and checks both give the same image.
#[test]
#[ignore]
fn gpu_restore() {
    let img = image::open(std::env::var("LIGHTVIEW_AI").unwrap()).unwrap().into_rgba8();
    let file = std::env::var("LIGHTVIEW_AI_MODEL").unwrap();
    let mut results = Vec::new();
    for gpu in [true, false] {
        set_use_gpu(gpu);
        let t = std::time::Instant::now();
        let out = restore(&file, &img, &|_| {}, &AtomicBool::new(false)).unwrap();
        eprintln!("use_gpu {gpu}: on_gpu {} in {:.0} ms, error {:?}", on_gpu(), t.elapsed().as_secs_f64() * 1000.0, gpu_error());
        assert_eq!(on_gpu(), gpu);
        results.push(out);
    }
    let worst = results[0].as_raw().iter().zip(results[1].as_raw()).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    eprintln!("largest GPU/CPU difference: {worst}");
    assert!(worst <= 2);
}

/// Installs the runtime the way the menu does: `cargo test install_runtime -- --ignored`.
#[test]
#[ignore]
fn install_runtime() {
    download_runtime().unwrap();
    assert!(runtime_installed());
    environment().unwrap();
}

/// A file:// URL for `platform::download` on Linux and Windows (`file:///C:/…` there).
fn file_url(path: &Path) -> String {
    let p = path.display().to_string().replace('\\', "/");
    if p.starts_with('/') { format!("file://{p}") } else { format!("file:///{p}") }
}

#[test]
fn sha256_of_a_file() {
    let path = std::env::temp_dir().join(format!("lightview-sha-{}", std::process::id()));
    std::fs::write(&path, b"abc").unwrap();
    let sum = sha256_file(&path);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(sum.unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

#[test]
fn bad_checksum_is_refused() {
    let (src, dest) = (std::env::temp_dir().join(format!("lightview-fetch-src-{}", std::process::id())), std::env::temp_dir().join(format!("lightview-fetch-{}", std::process::id())));
    std::fs::write(&src, b"not what was expected").unwrap();
    let result = fetch(&file_url(&src), "0000", &dest);
    std::fs::remove_file(&src).unwrap();
    assert_eq!(result, Err("the download didn't match its checksum".into()));
    assert!(!dest.exists(), "the bad download is removed");
}

/// `LIGHTVIEW_AI=photo.jpg cargo test --release remove_background -- --ignored --nocapture`
/// (needs the runtime; `LIGHTVIEW_AI_MODEL=<file in the models folder>`, silueta by default); writes `photo-cutout.png` to the temp folder.
#[test]
#[ignore]
fn remove_background() {
    let path = PathBuf::from(std::env::var("LIGHTVIEW_AI").unwrap());
    let img = image::open(&path).unwrap().into_rgba8();
    let t = std::time::Instant::now();
    let file = std::env::var("LIGHTVIEW_AI_MODEL").unwrap_or("silueta.onnx".into());
    let pre = preset(&file, &HashMap::new()).unwrap_or_else(|| Preprocess::auto(input_size(&file).unwrap()));
    let mask = mask(&file, &pre, &img).unwrap();
    eprintln!("mask in {:.0} ms, foreground {:?}", t.elapsed().as_secs_f64() * 1000.0, bbox(&mask));
    let out = std::env::temp_dir().join(format!("{}-cutout.png", path.file_stem().unwrap().to_string_lossy()));
    cutout(&img, &mask).save(&out).unwrap();
    eprintln!("wrote {}", out.display());
}

#[test]
fn ctc_merges_repeats_and_drops_blanks() {
    let dict: Vec<String> = ["a", "b"].map(String::from).to_vec();
    // Classes: blank, a, b, space. Steps: a a blank a b space b.
    let steps = [1, 1, 0, 1, 2, 3, 2];
    let probs: Vec<f32> = steps.iter().flat_map(|&s| (0..4).map(move |c| if c == s { 0.9 } else { 0.03 })).collect();
    assert_eq!(ctc_decode(&probs, 4, &dict), "aab b");
}

#[test]
fn text_boxes_keep_confident_blobs_grown() {
    let (w, h) = (40u32, 20u32);
    let mut prob = vec![0f32; (w * h) as usize];
    let mut fill = |x0: u32, y0: u32, x1: u32, y1: u32, v: f32| (y0..y1).for_each(|y| (x0..x1).for_each(|x| prob[(y * w + x) as usize] = v));
    fill(2, 2, 12, 6, 0.9); // A line.
    fill(20, 10, 36, 14, 0.4); // Too faint on average.
    fill(2, 12, 4, 18, 0.9); // Too thin.
    let boxes = text_boxes(&prob, w, h);
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    // 10 × 4 grown by 10·4·1.5 / 28 on each side.
    let g = 60.0 / 28.0;
    assert!((boxes[0].min.x - (2.0 - g)).abs() < 1e-4 && (boxes[0].max.y - (6.0 + g)).abs() < 1e-4, "{boxes:?}");
}

#[test]
fn lines_join_in_reading_order() {
    let r = |x: f32, y: f32| Rect::from_min_size(Pos2::new(x, y), egui::vec2(50.0, 20.0));
    let boxes = vec![(r(0.0, 40.0), "third".into()), (r(80.0, 3.0), "second".into()), (r(0.0, 0.0), "first".into())];
    assert_eq!(join_lines(boxes), "first second\nthird");
    assert_eq!(join_lines(Vec::new()), "");
}

#[test]
fn ocr_parts_are_not_models_and_are_needed() {
    let dir = std::env::temp_dir().join(format!("lightview-ocr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let latin = &CATALOG.iter().find(|k| k.task == Task::Ocr).unwrap();
    std::fs::write(dir.join(latin.file), b"").unwrap();
    std::fs::write(dir.join(OCR_DETECTOR.0), b"").unwrap();
    let without_dict = list_models(&dir);
    std::fs::write(dir.join(latin.data[1].0), b"").unwrap();
    let with_dict = list_models(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(without_dict.is_empty(), "{without_dict:?}");
    assert_eq!(with_dict, [latin.file]);
    // A model lightview doesn't know isn't offered for reading text.
    assert!(for_task(Task::Ocr, &["mine.onnx".into()]).is_empty());
}

#[test]
fn character_lists_parse() {
    let dict: Vec<String> = ron::from_str(r#"["a","\"","\\","ß","🇩🇪"]"#).unwrap();
    assert_eq!(dict, ["a", "\"", "\\", "ß", "🇩🇪"]);
}

/// Prints the text in `LIGHTVIEW_AI` and how long reading it took.
#[test]
#[ignore]
fn read_text_photo() {
    let img = image::open(std::env::var("LIGHTVIEW_AI").unwrap()).unwrap().into_rgba8();
    let file = std::env::var("LIGHTVIEW_AI_MODEL").unwrap_or("latin_PP-OCRv5_mobile_rec.onnx".into());
    let t = std::time::Instant::now();
    let text = read_text(&file, &img).unwrap();
    eprintln!("{}×{} in {} ms:\n{text}", img.width(), img.height(), t.elapsed().as_millis());
}

/// Done when printed text in the six UI languages is read back: one sentence per language,
/// drawn with Roboto. `LIGHTVIEW_AI_MODEL` picks the recogniser (default: Latin).
#[test]
#[ignore]
fn read_text_languages() {
    use ab_glyph::{Font, ScaleFont};
    let lines = [
        "The quick brown fox jumps over the lazy dog.",
        "Größe, Straße und Übung für Bäcker.",
        "El niño comió piñas en el año pasado.",
        "Le garçon élève où la fenêtre était.",
        "Perché la città è più bella così.",
        "A ação e a coração também são nossas.",
    ];
    let font = ab_glyph::FontRef::try_from_slice(include_bytes!("../../fonts/Roboto-Regular.ttf")).unwrap();
    let scaled = font.as_scaled(32.0);
    let mut img = RgbaImage::from_pixel(900, 60 * lines.len() as u32 + 40, image::Rgba([255; 4]));
    for (row, line) in lines.iter().enumerate() {
        let mut x = 30.0;
        for c in line.chars() {
            let mut glyph = scaled.scaled_glyph(c);
            glyph.position = ab_glyph::point(x, 60.0 + 60.0 * row as f32);
            x += scaled.h_advance(glyph.id);
            if let Some(outline) = font.outline_glyph(glyph) {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, v| {
                    let px = img.get_pixel_mut(b.min.x as u32 + gx, b.min.y as u32 + gy);
                    let ink = (255.0 * (1.0 - v)) as u8;
                    px.0 = [ink.min(px[0]), ink.min(px[1]), ink.min(px[2]), 255];
                });
            }
        }
    }
    let file = std::env::var("LIGHTVIEW_AI_MODEL").unwrap_or("latin_PP-OCRv5_mobile_rec.onnx".into());
    let t = std::time::Instant::now();
    let text = read_text(&file, &img).unwrap();
    eprintln!("{} ms:\n{text}", t.elapsed().as_millis());
    let expected = if file.starts_with("latin") { &lines[..] } else { &lines[..1] };
    let got: Vec<&str> = text.lines().collect();
    for (i, line) in expected.iter().enumerate() {
        assert_eq!(got.get(i), Some(line), "line {i}");
    }
}
