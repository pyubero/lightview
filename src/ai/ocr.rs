use super::*;

/// Recognize Text: the text in `img`, a line per row. `file` is the recogniser; the detector
/// and its character list come with it (`Known.data`). Follows PaddleOCR's own settings.
pub fn read_text(file: &str, img: &RgbaImage) -> Result<String, String> {
    let dir = models_dir()?;
    let dict = std::fs::read_to_string(dir.join(file.replace(".onnx", ".dict.json"))).map_err(|e| e.to_string())?;
    let dict: Vec<String> = ron::from_str(&dict).map_err(|e| e.to_string())?;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Ok(String::new());
    }
    // Detection: the long side at most 1920 px, the short one at least 64, both multiples of 32.
    // Lines are then read from the full-size image, so this only costs the smallest text.
    // shortcut: PaddleOCR allows 4000, but that took 8 s for 12 MP here (1920: 1.5 s, same text read); selecting tiny text first reads it at full size.
    let long = w.max(h) as f32;
    let scale = if long > 1920.0 { 1920.0 / long } else { (64.0 / w.min(h) as f32).max(1.0) };
    let side = |v: u32| (((v as f32 * scale / 32.0).round() as u32) * 32).clamp(32, 4096);
    let (dw, dh) = (side(w), side(h));
    let rgb = DynamicImage::ImageRgba8(img.clone()).into_rgb8();
    let small = image::imageops::resize(&rgb, dw, dh, FilterType::Triangle);
    let input = bgr_planes(&small, |v, c| (v / 255.0 - IMAGENET_MEAN[c]) / IMAGENET_STD[c]);
    let mut detector = session(OCR_DETECTOR.0)?;
    let tensor = ort::value::Tensor::from_array(([1usize, 3, dh as usize, dw as usize], input)).map_err(|e| e.to_string())?;
    let outputs = detector.run(ort::inputs![tensor]).map_err(|e| e.to_string())?;
    let (_, prob) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    let prob = prob.get(..(dw * dh) as usize).ok_or(tr("the model's output doesn't match its input size"))?;
    let (sx, sy) = (w as f32 / dw as f32, h as f32 / dh as f32);
    let boxes: Vec<Rect> = text_boxes(prob, dw, dh).into_iter().map(|r| Rect::from_min_max(Pos2::new(r.min.x * sx, r.min.y * sy), Pos2::new(r.max.x * sx, r.max.y * sy)).intersect(Rect::from_min_size(Pos2::ZERO, egui::vec2(w as f32, h as f32)))).collect();
    drop(outputs);
    drop(detector);
    // Recognition, a line at a time: 48 px high, at least 320 wide (zero padded), at most 3200.
    let mut recogniser = session(file)?;
    let mut lines = Vec::new();
    for b in boxes {
        if b.width() < 1.0 || b.height() < 1.0 {
            continue;
        }
        let (x, y, bw, bh) = (b.min.x as u32, b.min.y as u32, (b.width() as u32).max(1), (b.height() as u32).max(1));
        let line = image::imageops::crop_imm(&rgb, x, y, bw.min(w - x), bh.min(h - y)).to_image();
        let lw = ((48.0 * line.width() as f32 / line.height() as f32).ceil() as u32).clamp(1, 3200);
        let line = image::imageops::resize(&line, lw, 48, FilterType::Triangle);
        let pw = lw.max(320) as usize;
        let mut input = vec![0f32; 3 * 48 * pw];
        for (c, plane) in bgr_planes(&line, |v, _| v / 127.5 - 1.0).chunks(48 * lw as usize).enumerate() {
            for (row, src) in plane.chunks(lw as usize).enumerate() {
                input[c * 48 * pw + row * pw..][..lw as usize].copy_from_slice(src);
            }
        }
        let tensor = ort::value::Tensor::from_array(([1usize, 3, 48, pw], input)).map_err(|e| e.to_string())?;
        let outputs = recogniser.run(ort::inputs![tensor]).map_err(|e| e.to_string())?;
        let (shape, probs) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let classes = shape.iter().last().copied().unwrap_or(0) as usize;
        if classes != dict.len() + 2 {
            return Err(tr("the model's output doesn't match its character list").into());
        }
        let text = ctc_decode(probs, classes, &dict);
        if !text.trim().is_empty() {
            lines.push((b, text.trim().to_string()));
        }
    }
    Ok(join_lines(lines))
}

/// `img` as channel-first blue, green, red planes (OpenCV's order, which PaddleOCR trained on),
/// each value passed through `f(value, rgb channel)`.
fn bgr_planes(img: &image::RgbImage, f: impl Fn(f32, usize) -> f32) -> Vec<f32> {
    let plane = (img.width() * img.height()) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (i, px) in img.pixels().enumerate() {
        for c in 0..3 {
            out[(2 - c) * plane + i] = f(px[c] as f32, c);
        }
    }
    out
}

/// The text boxes in a detector's probability map (`w`×`h`, its px), as PaddleOCR's DB post-processing
/// finds them: pixels over 0.3, joined into blobs, kept if their mean is at least 0.6, then grown by
/// area × 1.5 / perimeter, since the model marks a shrunk core of each line.
// shortcut: upright boxes, not rotated ones, so tilted text needs Straighten first; vertical text isn't turned.
pub(super) fn text_boxes(prob: &[f32], w: u32, h: u32) -> Vec<Rect> {
    let (w, h) = (w as usize, h as usize);
    let mut seen = vec![false; w * h];
    let mut boxes = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if seen[start] || prob[start] <= 0.3 {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1, mut sum, mut n) = (w, h, 0, 0, 0.0, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            sum += prob[i];
            n += 1;
            let neighbours = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            for j in neighbours.into_iter().flatten() {
                if !seen[j] && prob[j] > 0.3 {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        if bw.min(bh) < 3.0 || sum / (n as f32) < 0.6 {
            continue;
        }
        let grow = bw * bh * 1.5 / (2.0 * (bw + bh));
        boxes.push(Rect::from_min_max(Pos2::new(x0 as f32, y0 as f32), Pos2::new(x1 as f32 + 1.0, y1 as f32 + 1.0)).expand(grow));
    }
    boxes
}

/// A recogniser's output (`classes` probabilities per step) as text: the likeliest class at each
/// step, repeats merged, blanks (class 0) dropped; class k + 1 is `dict[k]`, the last one a space.
pub(super) fn ctc_decode(probs: &[f32], classes: usize, dict: &[String]) -> String {
    let mut text = String::new();
    let mut last = 0;
    for step in probs.chunks_exact(classes) {
        let best = step.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);
        if best != last && best != 0 {
            text.push_str(dict.get(best - 1).map_or(" ", |s| s.as_str()));
        }
        last = best;
    }
    text
}

/// Lines read from `boxes` in reading order: a box whose middle lies within a row's first box
/// joins that row; rows top to bottom, boxes in a row left to right.
pub(super) fn join_lines(mut boxes: Vec<(Rect, String)>) -> String {
    boxes.sort_by(|a, b| a.0.min.y.total_cmp(&b.0.min.y));
    let mut rows: Vec<(Rect, Vec<(Rect, String)>)> = Vec::new();
    for (r, text) in boxes {
        match rows.iter_mut().find(|(first, _)| first.y_range().contains(r.center().y)) {
            Some((_, row)) => row.push((r, text)),
            None => rows.push((r, vec![(r, text)])),
        }
    }
    rows.into_iter()
        .map(|(_, mut row)| {
            row.sort_by(|a, b| a.0.min.x.total_cmp(&b.0.min.x));
            row.into_iter().map(|(_, t)| t).collect::<Vec<_>>().join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the QR codes in `img` say, as text: shown to the user, never acted on.
pub fn read_qr(img: &image::RgbaImage) -> Vec<String> {
    let luma = DynamicImage::ImageRgba8(img.clone()).into_luma8();
    // A decoder of untrusted data on another thread: a panic mustn't lose the reply.
    std::panic::catch_unwind(|| {
        let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(luma.width() as usize, luma.height() as usize, |x, y| luma.get_pixel(x as u32, y as u32)[0]);
        prepared.detect_grids().into_iter().filter_map(|g| g.decode().ok().map(|(_, text)| text)).collect()
    })
    .unwrap_or_default()
}
