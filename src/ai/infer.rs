use super::*;

/// The last inpainting model, by file and GPU setting. LaMa takes ~18 s to load (at any
/// optimisation level) and 1–4 s to run, and objects tend to be removed in a series.
pub(super) static INPAINTER: std::sync::Mutex<Option<((String, bool), ort::session::Session)>> = std::sync::Mutex::new(None);

pub(super) fn session(file: &str) -> Result<ort::session::Session, String> {
    // Loading another model frees the kept one: both may not fit a 4 GB GPU.
    INPAINTER.lock().unwrap().take();
    let path = models_dir()?.join(file);
    // One thread per physical core was ~10% faster than one per hardware thread here.
    // shortcut: assumes two hardware threads per core; read the real core count if that's wrong somewhere.
    let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
    let cpu = || -> Result<ort::session::Session, String> {
        let builder = ort::session::Session::builder(environment()?).map_err(|e| e.to_string())?;
        let mut builder = builder.with_intra_threads(threads).map_err(|e| e.to_string())?;
        builder.commit_from_file(&path).map_err(|e| e.to_string())
    };
    if !(USE_GPU.load(Ordering::Relaxed) && { environment()?; GPU_LOADED.load(Ordering::Relaxed) }) {
        return cpu();
    }
    // The GPU, falling back to the CPU (and saying why) if it can't be used.
    let gpu = (|| -> Result<ort::session::Session, String> {
        let builder = ort::session::Session::builder(environment()?).map_err(|e| e.to_string())?;
        let mut builder = builder.with_execution_providers([ort::ep::CUDA::default().build().error_on_failure()]).map_err(|e| e.to_string())?;
        builder.commit_from_file(&path).map_err(|e| e.to_string())
    })();
    match gpu {
        Ok(session) => {
            *GPU_ERROR.lock().unwrap() = None;
            Ok(session)
        }
        Err(e) => {
            *GPU_ERROR.lock().unwrap() = Some(e);
            cpu()
        }
    }
}

/// Whether the last session runs on the GPU.
pub fn on_gpu() -> bool {
    GPU_LOADED.load(Ordering::Relaxed) && USE_GPU.load(Ordering::Relaxed) && gpu_error().is_none()
}

/// Denoise / Enhance: runs an image-to-image model over `img` in overlapping tiles.
/// `progress` gets 0–1 after each tile; setting `cancel` stops it between tiles.
pub fn restore(file: &str, img: &RgbaImage, progress: &dyn Fn(f32), cancel: &AtomicBool) -> Result<RgbaImage, String> {
    let mut session = session(file)?;
    // A model exported with a fixed size only takes tiles of that size.
    let fixed = session.inputs().first().and_then(|i| i.dtype().tensor_shape()).and_then(|shape| match shape.iter().copied().collect::<Vec<i64>>().as_slice() {
        [.., h, w] if h == w && (1..=MAX_INPUT_SIDE).contains(h) => Some(*h as usize),
        _ => None,
    });
    // Big tiles: per pixel they cost the same. A smaller image gets a tile just big enough
    // (a multiple of 64, which these models accept), so the preview isn't padded out.
    // Each catalog model says how big (`Known::max_tile`); 512 for others.
    let name = Path::new(file).file_name().and_then(|n| n.to_str()).unwrap_or(file);
    let cap = known(name).map_or(512, |k| k.max_tile as usize);
    let long = img.width().max(img.height()) as usize;
    let tile = fixed.unwrap_or(long.div_ceil(64).max(1) * 64).min(fixed.unwrap_or(cap));
    let run = |input: Vec<f32>, t: usize| -> Result<Vec<f32>, String> {
        let tensor = ort::value::Tensor::from_array(([1usize, 3, t, t], input)).map_err(|e| e.to_string())?;
        let outputs = session.run(ort::inputs![tensor]).map_err(|e| e.to_string())?;
        let (_, out) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        Ok(out.to_vec())
    };
    tiled(img, tile, tile / 8, run, progress, cancel)
}

/// Runs `run` (RGB planes in 0–1, `tile`×`tile`) over `img` in tiles overlapping by `overlap`.
/// The model may upscale: its scale is read from the first tile's output, and the result is
/// that many times bigger. Neighbouring tiles each keep their half of the overlap (so each sees
/// context past its edge) and the result is written directly, without float buffers: a 4×
/// result of a 12 MP photo is already 770 MB. Alpha comes from `img`, scaled up bilinearly.
pub(super) fn tiled(img: &RgbaImage, tile: usize, overlap: usize, mut run: impl FnMut(Vec<f32>, usize) -> Result<Vec<f32>, String>, progress: &dyn Fn(f32), cancel: &AtomicBool) -> Result<RgbaImage, String> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let starts = |len: usize| -> Vec<usize> {
        if len <= tile {
            return vec![0];
        }
        // As few tiles as cover `len` with at least `overlap` between neighbours, spread evenly.
        let n = (len - overlap).div_ceil(tile - overlap);
        (0..n).map(|i| i * (len - tile) / (n - 1)).collect()
    };
    // The part of the image each tile writes: up to the middle of its overlaps.
    let owned = |starts: &[usize], len: usize| -> Vec<(usize, usize)> {
        (0..starts.len())
            .map(|i| {
                let a = if i == 0 { 0 } else { (starts[i] + starts[i - 1] + tile) / 2 };
                let b = if i + 1 == starts.len() { len } else { (starts[i + 1] + starts[i] + tile) / 2 };
                (a, b)
            })
            .collect()
    };
    let (xs, ys) = (starts(w), starts(h));
    let (ox, oy) = (owned(&xs, w), owned(&ys, h));
    let plane = tile * tile;
    let mut out: Option<(usize, RgbaImage)> = None;
    for (n, (yi, xi)) in (0..ys.len()).flat_map(|y| (0..xs.len()).map(move |x| (y, x))).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let (x0, y0) = (xs[xi], ys[yi]);
        // Past the image's edge (an image smaller than a tile), repeat the edge pixels.
        let mut input = vec![0f32; 3 * plane];
        for ty in 0..tile {
            for tx in 0..tile {
                let px = img.get_pixel((x0 + tx).min(w - 1) as u32, (y0 + ty).min(h - 1) as u32);
                for c in 0..3 {
                    input[c * plane + ty * tile + tx] = px[c] as f32 / 255.0;
                }
            }
        }
        let res = run(input, tile)?;
        if out.is_none() {
            let scale = ((res.len() / (3 * plane)) as f64).sqrt().round() as usize;
            if scale == 0 || 3 * plane * scale * scale != res.len() {
                return Err(tr("the model's output isn't a whole multiple of its input").into());
            }
            out = Some((scale, with_alpha_of(img, scale)));
        }
        let (scale, result) = out.as_mut().unwrap();
        let (s, ts) = (*scale, tile * *scale);
        if res.len() != 3 * ts * ts {
            return Err(tr("the model's output changed size between tiles").into());
        }
        let ((ax, bx), (ay, by)) = (ox[xi], oy[yi]);
        for y in ay * s..by * s {
            let row = (y - y0 * s) * ts;
            for x in ax * s..bx * s {
                let px = result.get_pixel_mut(x as u32, y as u32);
                for c in 0..3 {
                    px[c] = (res[c * ts * ts + row + x - x0 * s] * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        progress((n + 1) as f32 / (xs.len() * ys.len()) as f32);
    }
    Ok(out.map_or_else(|| img.clone(), |(_, result)| result))
}

/// A `scale`× bigger image whose alpha is `img`'s (scaled up bilinearly); its RGB is filled in later.
fn with_alpha_of(img: &RgbaImage, scale: usize) -> RgbaImage {
    if scale == 1 {
        return img.clone();
    }
    let (w, h) = (img.width() * scale as u32, img.height() * scale as u32);
    if img.pixels().all(|p| p[3] == 255) {
        return RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
    }
    let alpha = GrayImage::from_fn(img.width(), img.height(), |x, y| image::Luma([img.get_pixel(x, y)[3]]));
    let alpha = image::imageops::resize(&alpha, w, h, FilterType::Triangle);
    RgbaImage::from_fn(w, h, |x, y| image::Rgba([0, 0, 0, alpha.get_pixel(x, y)[0]]))
}

/// The part of the image the model sees around `region`, as x, y, width, height: a square with
/// room for context, at least 512 px so small areas aren't enlarged, moved to stay inside the
/// `w`×`h` image and cut only where the image is smaller.
pub(super) fn context_rect(region: Rect, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let side = (region.width().max(region.height()) as u32 * 3 / 2).max(512);
    let (cw, ch) = (side.min(w), side.min(h));
    let place = |center: f32, len: u32, max: u32| (center as u32).saturating_sub(len / 2).min(max - len);
    (place(region.center().x, cw, w), place(region.center().y, ch, h), cw, ch)
}

/// Channel-first bytes, as the inpainting models take them.
pub(super) fn to_planes(rgb: &image::RgbImage) -> Vec<u8> {
    (0..3).flat_map(|c| rgb.pixels().map(move |p| p[c])).collect()
}

pub(super) fn from_planes(data: &[u8], w: u32, h: u32) -> Result<image::RgbImage, String> {
    let plane = (w * h) as usize;
    if data.len() != 3 * plane {
        return Err(tr("the model's output doesn't match its input size").into());
    }
    Ok(image::RgbImage::from_fn(w, h, |x, y| {
        let i = (y * w + x) as usize;
        image::Rgb([data[i], data[plane + i], data[2 * plane + i]])
    }))
}

/// Fills `hole` (coverage over `region`) from the context around it; the result is `region`'s
/// size, with `img`'s alpha. `run` gets the context as RGB and the hole as 255, at `size` if given,
/// and returns the filled context at that size.
pub(super) fn fill_context(img: &RgbaImage, region: Rect, hole: &GrayImage, size: Option<u32>, run: impl FnOnce(image::RgbImage, GrayImage) -> Result<image::RgbImage, String>) -> Result<RgbaImage, String> {
    let (x, y, cw, ch) = context_rect(region, img.width(), img.height());
    let (rx, ry) = (region.min.x as u32, region.min.y as u32);
    let mut rgb = DynamicImage::ImageRgba8(image::imageops::crop_imm(img, x, y, cw, ch).to_image()).into_rgb8();
    let mut mask = GrayImage::new(cw, ch);
    image::imageops::replace(&mut mask, hole, (rx - x) as i64, (ry - y) as i64);
    if let Some(s) = size {
        rgb = image::imageops::resize(&rgb, s, s, FilterType::Triangle);
        mask = image::imageops::resize(&mask, s, s, FilterType::Triangle);
    }
    // Anything touched stays a hole: a tiny selection mustn't vanish in the resize.
    mask.pixels_mut().for_each(|m| m[0] = if m[0] > 0 { 255 } else { 0 });
    let filled = run(rgb, mask)?;
    let filled = if size.is_some() { image::imageops::resize(&filled, cw, ch, FilterType::Lanczos3) } else { filled };
    if filled.dimensions() != (cw, ch) {
        return Err(tr("the model's output doesn't match its input size").into());
    }
    let (rw, rh) = hole.dimensions();
    Ok(RgbaImage::from_fn(rw, rh, |px, py| {
        let c = filled.get_pixel(rx - x + px, ry - y + py);
        image::Rgba([c[0], c[1], c[2], img.get_pixel(rx + px, ry + py)[3]])
    }))
}

/// Remove Object: `img` with `hole` (coverage over `region`) filled in, as a `region`-sized image.
/// A model run can't be interrupted, so `cancel` is checked either side of it.
pub fn inpaint(file: &str, img: &RgbaImage, region: Rect, hole: &GrayImage, cancel: &AtomicBool) -> Result<RgbaImage, String> {
    let key = (file.to_string(), USE_GPU.load(Ordering::Relaxed));
    let kept = INPAINTER.lock().unwrap().take().filter(|(k, _)| *k == key);
    let mut session = match kept {
        Some((_, session)) => session,
        None => session(file)?,
    };
    let lama = file == LAMA_FILE;
    let result = fill_context(img, region, hole, lama.then_some(512), |rgb, mask| {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }
        let (w, h) = rgb.dimensions();
        let (uw, uh) = (w as usize, h as usize);
        if lama {
            // Fixed at 512×512: 0–1 in, 0–255 out; a mask of 1 is the hole.
            let image: Vec<f32> = to_planes(&rgb).iter().map(|&v| v as f32 / 255.0).collect();
            let mask: Vec<f32> = mask.pixels().map(|m| m[0] as f32 / 255.0).collect();
            let image = ort::value::Tensor::from_array(([1usize, 3, uh, uw], image)).map_err(|e| e.to_string())?;
            let mask = ort::value::Tensor::from_array(([1usize, 1, uh, uw], mask)).map_err(|e| e.to_string())?;
            let outputs = session.run(ort::inputs!["image" => image, "mask" => mask]).map_err(|e| e.to_string())?;
            let (_, out) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
            from_planes(&out.iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect::<Vec<u8>>(), w, h)
        } else {
            // MI-GAN's pipeline: any size, bytes in and out; a mask of 0 is the hole.
            let mask: Vec<u8> = mask.pixels().map(|m| 255 - m[0]).collect();
            let image = ort::value::Tensor::from_array(([1usize, 3, uh, uw], to_planes(&rgb))).map_err(|e| e.to_string())?;
            let mask = ort::value::Tensor::from_array(([1usize, 1, uh, uw], mask)).map_err(|e| e.to_string())?;
            let outputs = session.run(ort::inputs!["image" => image, "mask" => mask]).map_err(|e| e.to_string())?;
            let (_, out) = outputs[0].try_extract_tensor::<u8>().map_err(|e| e.to_string())?;
            from_planes(out, w, h)
        }
    });
    *INPAINTER.lock().unwrap() = Some((key, session));
    if cancel.load(Ordering::Relaxed) { Err("cancelled".to_string()) } else { result }
}

/// The model's input size, from its first input's shape (…, H, W); 1024 when it's dynamic.
pub fn input_size(file: &str) -> Result<u32, String> {
    let session = session(file)?;
    let shape = session.inputs().first().and_then(|i| i.dtype().tensor_shape()).ok_or(tr("the model has no image input"))?;
    let dims: Vec<i64> = shape.iter().copied().collect();
    Ok(match dims.as_slice() {
        [.., h, w] if h == w && (1..=MAX_INPUT_SIDE).contains(h) => *h as u32,
        _ => 1024,
    })
}

/// The foreground mask (255 = keep) for `img`, at its size.
pub fn mask(file: &str, pre: &Preprocess, img: &RgbaImage) -> Result<GrayImage, String> {
    let input = input(pre, img);
    let mut session = session(file)?;
    let s = pre.size as usize;
    let tensor = ort::value::Tensor::from_array(([1usize, 3, s, s], input)).map_err(|e| e.to_string())?;
    let outputs = session.run(ort::inputs![tensor]).map_err(|e| e.to_string())?;
    let (_, pred) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    let pred = pred.get(..s * s).ok_or(tr("the model's output doesn't match its input size"))?;
    Ok(to_mask(pred, pre.size, pre.sigmoid, img.width(), img.height()))
}

/// The model's input: resized, scaled to 0–1, normalised, in channel-first order.
pub(super) fn input(pre: &Preprocess, img: &RgbaImage) -> Vec<f32> {
    let small = DynamicImage::ImageRgba8(img.clone()).resize_exact(pre.size, pre.size, FilterType::Triangle).into_rgb8();
    let plane = (pre.size * pre.size) as usize;
    let mut input = vec![0f32; 3 * plane];
    for (i, px) in small.pixels().enumerate() {
        for c in 0..3 {
            input[c * plane + i] = (px[c] as f32 / 255.0 - pre.mean[c]) / pre.std[c];
        }
    }
    input
}

/// Stretches the prediction to 0–255 (like rembg) and scales it to `w`×`h`.
pub(super) fn to_mask(pred: &[f32], size: u32, sigmoid: bool, w: u32, h: u32) -> GrayImage {
    let sig = |v: f32| if sigmoid { 1.0 / (1.0 + (-v).exp()) } else { v };
    let pred: Vec<f32> = pred.iter().map(|&v| sig(v)).collect();
    let (lo, hi) = pred.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let range = (hi - lo).max(1e-6);
    let small = GrayImage::from_vec(size, size, pred.iter().map(|&v| ((v - lo) / range * 255.0).round() as u8).collect()).unwrap();
    DynamicImage::ImageLuma8(small).resize_exact(w, h, FilterType::Triangle).into_luma8()
}

/// `img` with the mask applied to its alpha.
pub fn cutout(img: &RgbaImage, mask: &GrayImage) -> RgbaImage {
    let mut out = img.clone();
    for (px, m) in out.pixels_mut().zip(mask.pixels()) {
        px[3] = (px[3] as u32 * m[0] as u32 / 255) as u8;
    }
    out
}

/// The bounding box of the mask's foreground (≥ 50%), if any.
pub fn bbox(mask: &GrayImage) -> Option<Rect> {
    let (mut min, mut max) = (Pos2::new(f32::MAX, f32::MAX), Pos2::new(f32::MIN, f32::MIN));
    for (x, y, m) in mask.enumerate_pixels() {
        if m[0] >= 128 {
            min = min.min(Pos2::new(x as f32, y as f32));
            max = max.max(Pos2::new(x as f32 + 1.0, y as f32 + 1.0));
        }
    }
    (min.x < max.x).then(|| Rect::from_min_max(min, max))
}
