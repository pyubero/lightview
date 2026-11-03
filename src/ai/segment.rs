use super::*;

/// The side MobileSAM's encoder works at (it pads to a square itself).
const SAM_SIDE: f32 = 1024.0;

/// Select Object: the outline, in image px, of the object MobileSAM finds inside `rect` (image px).
pub fn select(file: &str, img: &RgbaImage, rect: Rect) -> Result<Vec<Pos2>, String> {
    let decoder = known(file).and_then(|k| k.data.first()).ok_or(tr("the model's mask decoder is missing"))?.0;
    let (w, h) = img.dimensions();
    // The encoder takes RGB in 0–255, height × width × 3, with its long side at 1024; it normalises and pads itself.
    let s = SAM_SIDE / w.max(h) as f32;
    let (sw, sh) = (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1));
    let small = image::imageops::resize(&DynamicImage::ImageRgba8(img.clone()).into_rgb8(), sw, sh, FilterType::Triangle);
    let pixels: Vec<f32> = small.into_raw().into_iter().map(f32::from).collect();
    let embeddings = {
        let mut encoder = session(file)?;
        let input = ort::value::Tensor::from_array(([sh as usize, sw as usize, 3], pixels)).map_err(|e| e.to_string())?;
        let outputs = encoder.run(ort::inputs!["input_image" => input]).map_err(|e| e.to_string())?;
        let (shape, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        (shape.iter().map(|&d| d as usize).collect::<Vec<_>>(), data.to_vec())
    };
    let mut decoder = session(decoder)?;
    // The box as its two corners, labelled 2 and 3, in the encoder's coordinates.
    let corners = vec![rect.min.x * s, rect.min.y * s, rect.max.x * s, rect.max.y * s];
    let inputs = ort::inputs![
        "image_embeddings" => ort::value::Tensor::from_array((embeddings.0, embeddings.1)).map_err(|e| e.to_string())?,
        "point_coords" => ort::value::Tensor::from_array(([1usize, 2, 2], corners)).map_err(|e| e.to_string())?,
        "point_labels" => ort::value::Tensor::from_array(([1usize, 2], vec![2f32, 3.0])).map_err(|e| e.to_string())?,
        "mask_input" => ort::value::Tensor::from_array(([1usize, 1, 256, 256], vec![0f32; 256 * 256])).map_err(|e| e.to_string())?,
        "has_mask_input" => ort::value::Tensor::from_array(([1usize], vec![0f32])).map_err(|e| e.to_string())?,
        "orig_im_size" => ort::value::Tensor::from_array(([2usize], vec![h as f32, w as f32])).map_err(|e| e.to_string())?,
    ];
    let outputs = decoder.run(inputs).map_err(|e| e.to_string())?;
    let (_, logits) = outputs["masks"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    let logits = logits.get(..(w * h) as usize).ok_or(tr("the model's output doesn't match its input size"))?;
    // Only inside the box: the user drew it around the object.
    let inside = |x: u32, y: u32| rect.contains(Pos2::new(x as f32 + 0.5, y as f32 + 0.5)) && logits[(y * w + x) as usize] > 0.0;
    let mask: Vec<bool> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| inside(x, y)).collect();
    Ok(outline(&largest_part(&mask, w, h), w, h))
}

/// Only the biggest 4-connected part of `mask`.
pub(super) fn largest_part(mask: &[bool], w: u32, h: u32) -> Vec<bool> {
    let (w, h) = (w as usize, h as usize);
    let mut label = vec![0u32; mask.len()];
    let (mut best, mut best_size, mut next) = (0, 0, 0);
    for start in 0..mask.len() {
        if !mask[start] || label[start] != 0 {
            continue;
        }
        next += 1;
        label[start] = next;
        let (mut stack, mut size) = (vec![start], 0);
        while let Some(i) = stack.pop() {
            size += 1;
            let (x, y) = (i % w, i / w);
            let around = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            for j in around.into_iter().flatten() {
                if mask[j] && label[j] == 0 {
                    label[j] = next;
                    stack.push(j);
                }
            }
        }
        if size > best_size {
            (best, best_size) = (next, size);
        }
    }
    label.iter().map(|&l| best != 0 && l == best).collect()
}

/// The outer boundary of the shape in `mask` (one 4-connected part), along pixel edges, as the
/// corners where it turns; empty when the mask is.
pub(super) fn outline(mask: &[bool], w: u32, h: u32) -> Vec<Pos2> {
    let (w, h) = (w as i32, h as i32);
    let at = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && mask[(y * w + x) as usize];
    let Some(first) = mask.iter().position(|&m| m) else { return Vec::new() };
    let start = (first as i32 % w, first as i32 / w);
    // Walk the pixel edges clockwise, the shape on the right: start along the first pixel's top edge.
    let (mut c, mut d) = (start, (1, 0));
    // Whether the edge from corner `c` in direction `d` has the shape on its right only.
    let edge = |c: (i32, i32), d: (i32, i32)| {
        let (right, left) = ((-d.1, d.0), (d.1, -d.0));
        let pixel = |side: (i32, i32)| at((2 * c.0 + d.0 + side.0).div_euclid(2), (2 * c.1 + d.1 + side.1).div_euclid(2));
        pixel(right) && !pixel(left)
    };
    let mut points = Vec::new();
    for _ in 0..4 * (w + 1) * (h + 1) {
        // Prefer turning left, so the walk hugs the shape.
        let turns = [(d.1, -d.0), d, (-d.1, d.0), (-d.0, -d.1)];
        let Some(&nd) = turns.iter().find(|&&t| edge(c, t)) else { break };
        if nd != d || points.is_empty() {
            points.push(Pos2::new(c.0 as f32, c.1 as f32));
        }
        d = nd;
        c = (c.0 + d.0, c.1 + d.1);
        if c == start {
            break;
        }
    }
    points
}
