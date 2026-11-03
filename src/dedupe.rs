//! Group Duplicates in Folder: perceptual hashes, grouping, and moving each group into a folder.
//! Nothing here is kept on disk: hashes live only as long as the dialog (`docs/DEDUPLICATE.md`).

use crate::edit;
use crate::files::{self, Stamp};
use crate::i18n::{tr, trf};
use crate::load::{self, Frame, Msg, Notify};
use eframe::egui::{self, Pos2, Rect, Vec2};
use fast_image_resize::{FilterType, ResizeAlg};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The longest side of a group member's thumbnail in the dialog.
const THUMB: u32 = 96;

/// What one hashed image gives the dialog.
pub type Hashed = (u64, Stamp, egui::ColorImage);

fn whole(frame: &Frame) -> Rect {
    Rect::from_min_size(Pos2::ZERO, Vec2::new(frame.w as f32, frame.h as f32))
}

/// pHash: the 8×8 lowest frequencies of a 32×32 greyscale DCT, each bit set when above their median.
pub fn phash(frame: &Frame) -> u64 {
    const N: usize = 32;
    let px = edit::resize(frame, whole(frame), N as u32, N as u32, ResizeAlg::Convolution(FilterType::Box));
    let ch = if frame.alpha { 4 } else { 3 };
    let grey: Vec<f32> = px.chunks_exact(ch).map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).collect();
    let cos = |k: usize, n: usize| (std::f32::consts::PI * (2 * n + 1) as f32 * k as f32 / (2 * N) as f32).cos();
    // Rows first (only the 8 frequencies kept), then columns.
    let mut rows = [[0f32; 8]; N];
    for (y, row) in rows.iter_mut().enumerate() {
        for (u, out) in row.iter_mut().enumerate() {
            *out = (0..N).map(|x| cos(u, x) * grey[y * N + x]).sum();
        }
    }
    let mut low = [0f32; 64];
    for v in 0..8 {
        for u in 0..8 {
            low[v * 8 + u] = (0..N).map(|y| cos(v, y) * rows[y][u]).sum();
        }
    }
    let mut sorted = low;
    sorted.sort_by(f32::total_cmp);
    let median = (sorted[31] + sorted[32]) / 2.0;
    low.iter().enumerate().fold(0, |bits, (i, &c)| if c > median { bits | 1 << i } else { bits })
}

pub fn thumb(frame: &Frame) -> egui::ColorImage {
    let scale = THUMB as f32 / frame.w.max(frame.h) as f32;
    let (w, h) = if scale < 1.0 { (((frame.w as f32 * scale) as u32).max(1), ((frame.h as f32 * scale) as u32).max(1)) } else { (frame.w, frame.h) };
    let px = edit::resize(frame, whole(frame), w, h, ResizeAlg::Convolution(FilterType::Bilinear));
    let size = [w as usize, h as usize];
    if frame.alpha { egui::ColorImage::from_rgba_unmultiplied(size, &px) } else { egui::ColorImage::from_rgb(size, &px) }
}

pub fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Each hash's distance to its closest other hash (`None` for images that weren't hashed).
pub fn nearest(hashes: &[Option<u64>]) -> Vec<Option<u32>> {
    hashes.iter().enumerate().map(|(i, a)| {
        let a = (*a)?;
        hashes.iter().enumerate().filter(|&(j, _)| j != i).filter_map(|(_, b)| Some(distance(a, (*b)?))).min()
    }).collect()
}

/// The suggested threshold: Otsu's split of the nearest-neighbour distances, clamped to 2–12;
/// 6 when there's no pile of near-duplicates to split off.
pub fn suggest(nearest: &[Option<u32>]) -> u32 {
    let d: Vec<u32> = nearest.iter().flatten().copied().collect();
    if d.iter().filter(|&&d| d < 12).count() < 2 {
        return 6;
    }
    let n = d.len() as f64;
    let mut hist = [0f64; 65];
    for &v in &d {
        hist[v.min(64) as usize] += 1.0;
    }
    let total: f64 = (0..65).map(|i| i as f64 * hist[i]).sum();
    let (mut w0, mut sum0, mut best, mut picks) = (0.0, 0.0, -1.0, Vec::new());
    for t in 0..64 {
        w0 += hist[t];
        sum0 += t as f64 * hist[t];
        let w1 = n - w0;
        if w0 == 0.0 || w1 == 0.0 {
            continue;
        }
        let between = w0 * w1 * (sum0 / w0 - (total - sum0) / w1).powi(2);
        if between > best + 1e-9 {
            (best, picks) = (between, vec![t]);
        } else if (between - best).abs() <= 1e-9 {
            picks.push(t);
        }
    }
    // Every threshold inside an empty gap splits the same way: take the gap's middle.
    let t = picks.first().zip(picks.last()).map_or(6, |(a, b)| (a + b) / 2) as u32;
    t.clamp(2, 12)
}

/// The groups at threshold `t`: images linked through any chain of neighbours within `t`
/// (a burst stays together), at least two each, members in folder order, biggest groups first.
pub fn group(hashes: &[Option<u64>], t: u32) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..hashes.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..hashes.len() {
        let Some(a) = hashes[i] else { continue };
        for j in i + 1..hashes.len() {
            if hashes[j].is_some_and(|b| distance(a, b) <= t) {
                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                parent[ri.max(rj)] = ri.min(rj);
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); hashes.len()];
    for i in 0..hashes.len() {
        if hashes[i].is_some() {
            let r = root(&mut parent, i);
            groups[r].push(i);
        }
    }
    let mut groups: Vec<Vec<usize>> = groups.into_iter().filter(|g| g.len() > 1).collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    groups
}

/// Hashes `files` on three threads, sending each result as `Msg::Hashed(run, index, …)`.
pub fn hash_all(files: Vec<PathBuf>, run: u64, color_managed: bool, cancel: Arc<AtomicBool>, notify: Notify) {
    let (files, next) = (Arc::new(files), Arc::new(AtomicUsize::new(0)));
    for _ in 0..3 {
        let (files, next, cancel, notify) = (files.clone(), next.clone(), cancel.clone(), notify.clone());
        std::thread::spawn(move || loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= files.len() || cancel.load(Ordering::Relaxed) {
                return;
            }
            // Read before decoding, so a change during the decode shows when moving.
            let stamp = files::stamp(&files[i]);
            let result = load::small(&files[i], color_managed).and_then(|f| Ok((phash(&f), stamp.ok_or(tr("the file is gone"))?, thumb(&f))));
            notify.send(Msg::Hashed(run, i, result));
        });
    }
}

/// What a Move did, kept for Ctrl+Z.
#[derive(Debug, Default)]
pub struct Moved {
    /// From, to.
    pub moves: Vec<(PathBuf, PathBuf)>,
    pub created: Vec<PathBuf>,
    /// Files left where they were, by name, with why.
    pub skipped: Vec<String>,
    /// What stopped the run early.
    pub error: Option<String>,
}

/// A new folder beside `first`, named after it without its extension, with " (2)"… when taken.
fn new_folder(first: &Path) -> std::io::Result<PathBuf> {
    let dir = first.parent().unwrap_or(Path::new("."));
    let stem = first.file_stem().unwrap_or_default().to_string_lossy();
    // Windows refuses folder names ending in a dot or space.
    let stem = stem.trim_end_matches(['.', ' ']);
    let stem = if stem.is_empty() { "group" } else { stem };
    for n in 1.. {
        let name = if n == 1 { stem.to_string() } else { format!("{stem} ({n})") };
        let path = dir.join(name);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// Moves each group (first member included) into a new folder named after its first member.
/// A file that changed since it was hashed is skipped; the first error stops the run.
pub fn move_groups(groups: &[Vec<(PathBuf, Stamp)>]) -> Moved {
    let mut done = Moved::default();
    for group in groups {
        let Some((first, _)) = group.first() else { continue };
        let folder = match new_folder(first) {
            Ok(f) => f,
            Err(e) => {
                done.error = Some(e.to_string());
                return done;
            }
        };
        done.created.push(folder.clone());
        for (path, stamp) in group {
            let name = path.file_name().unwrap_or_default();
            let shown = name.to_string_lossy();
            if files::stamp(path) != Some(*stamp) {
                done.skipped.push(trf("{name}: changed since it was compared", &[("name", &shown)]));
                continue;
            }
            let to = folder.join(name);
            // `rename` would silently replace it.
            if to.symlink_metadata().is_ok() {
                done.skipped.push(trf("{name}: already in the folder", &[("name", &shown)]));
                continue;
            }
            if let Err(e) = std::fs::rename(path, &to) {
                done.error = Some(format!("{shown}: {e}"));
                return done;
            }
            done.moves.push((path.clone(), to));
        }
    }
    done
}

/// Moves everything back and removes the folders that are left empty. Returns what couldn't go back.
pub fn undo(moved: &Moved) -> Vec<String> {
    let mut problems = Vec::new();
    for (from, to) in moved.moves.iter().rev() {
        let name = from.file_name().unwrap_or_default().to_string_lossy();
        if from.symlink_metadata().is_ok() {
            problems.push(trf("{name}: its old name is taken", &[("name", &name)]));
        } else if let Err(e) = std::fs::rename(to, from) {
            problems.push(format!("{name}: {e}"));
        }
    }
    for dir in &moved.created {
        // Only succeeds when empty, so nothing the user put there is lost.
        let _ = std::fs::remove_dir(dir);
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, RgbImage};
    use std::io::Cursor;

    fn frame(img: RgbImage) -> Frame {
        Frame { w: img.width(), h: img.height(), pixels: img.into_raw(), alpha: false, delay: Default::default() }
    }

    fn photo(seed: u32) -> RgbImage {
        RgbImage::from_fn(640, 480, |x, y| {
            let (fx, fy) = (x as f32 / 640.0, y as f32 / 480.0);
            let s = seed as f32;
            let v = ((fx * (3.0 + s) + fy * (2.0 + s * 0.7)).sin() * 0.5 + 0.5) * 200.0 + ((fx * fy * 9.0 * (1.0 + s)).cos() * 25.0);
            image::Rgb([v as u8, (v * 0.8) as u8, (255.0 - v) as u8])
        })
    }

    #[test]
    fn copies_are_close_and_different_images_are_far() {
        let original = photo(1);
        let small = image::imageops::resize(&original, 320, 240, image::imageops::FilterType::Triangle);
        let mut jpeg = Vec::new();
        DynamicImage::ImageRgb8(small).write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg).unwrap();
        let copy = image::load_from_memory(&jpeg).unwrap().into_rgb8();
        let (a, b, c) = (phash(&frame(original)), phash(&frame(copy)), phash(&frame(photo(5))));
        assert!(distance(a, b) <= 4, "copy: {}", distance(a, b));
        assert!(distance(a, c) > 20, "different: {}", distance(a, c));
    }

    #[test]
    fn groups_chain_through_neighbours() {
        // a–b and b–c are 4 bits apart, a–c 8: one group at 4; d is far from all.
        let (a, b, c, d) = (0u64, 0xF, 0xFF, u64::MAX);
        assert_eq!(group(&[Some(c), Some(a), None, Some(d), Some(b)], 4), vec![vec![0, 1, 4]]);
        assert_eq!(group(&[Some(a), Some(c), Some(b)], 3), Vec::<Vec<usize>>::new());
        assert_eq!(group(&[Some(a), Some(a), Some(d), Some(d), Some(d)], 0), vec![vec![2, 3, 4], vec![0, 1]]);
    }

    #[test]
    fn suggests_the_gap() {
        let mut d: Vec<Option<u32>> = [0, 1, 1, 2, 3].map(Some).to_vec();
        d.extend([27, 29, 30, 31, 33, 35].map(Some));
        let t = suggest(&d);
        assert!((3..=12).contains(&t), "{t}");
        assert_eq!(suggest(&[Some(28), Some(30), Some(31), None]), 6);
    }

    #[test]
    fn moves_into_named_folders_and_undoes() {
        let dir = std::env::temp_dir().join(format!("lightview-dedupe-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("IMG_1")).unwrap();
        let paths: Vec<PathBuf> = ["IMG_1.jpg", "IMG_2.jpg", "IMG_3.jpg"].iter().map(|n| dir.join(n)).collect();
        for p in &paths {
            std::fs::write(p, p.to_string_lossy().as_bytes()).unwrap();
        }
        let stamped = |p: &PathBuf| (p.clone(), files::stamp(p).unwrap());
        let mut group: Vec<_> = paths.iter().map(stamped).collect();
        // One changes after it was compared.
        std::fs::write(&paths[2], b"changed, and longer than before").unwrap();
        group[2].1 = (group[2].1.0, 1);
        let moved = move_groups(&[group]);
        assert_eq!(moved.error, None);
        assert_eq!(moved.created, [dir.join("IMG_1 (2)")], "the taken name gets a number");
        assert_eq!(moved.moves.len(), 2);
        assert_eq!(moved.skipped.len(), 1);
        assert!(dir.join("IMG_1 (2)/IMG_1.jpg").is_file() && dir.join("IMG_1 (2)/IMG_2.jpg").is_file() && paths[2].is_file());

        // Undo puts them back; the folder holding a stray file stays.
        std::fs::write(&paths[1], b"new file with the old name").unwrap();
        let problems = undo(&moved);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(paths[0].is_file() && dir.join("IMG_1 (2)/IMG_2.jpg").is_file());
        std::fs::remove_file(&paths[1]).unwrap();
        std::fs::rename(dir.join("IMG_1 (2)/IMG_2.jpg"), &paths[1]).unwrap();
        assert!(undo(&Moved { created: moved.created.clone(), ..Default::default() }).is_empty());
        assert!(!dir.join("IMG_1 (2)").exists(), "an empty folder is removed");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Timing on a real folder (nothing is moved): `LIGHTVIEW_DEDUPE=<dir> cargo test --release dedupe_folder -- --ignored --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn dedupe_folder() {
    let dir = PathBuf::from(std::env::var("LIGHTVIEW_DEDUPE").expect("LIGHTVIEW_DEDUPE=<dir>"));
    let files = files::list_folder(&dir.join("-"), crate::settings::SortOrder::Name);
    let (tx, rx) = std::sync::mpsc::channel();
    let t0 = std::time::Instant::now();
    hash_all(files.clone(), 1, true, Arc::default(), Notify { tx, ctx: Arc::default() });
    let mut hashes = vec![None; files.len()];
    for _ in 0..files.len() {
        if let Msg::Hashed(_, i, Ok((h, ..))) = rx.recv().unwrap() {
            hashes[i] = Some(h);
        }
    }
    let took = t0.elapsed();
    let nearest = nearest(&hashes);
    let t = suggest(&nearest);
    let groups = group(&hashes, t);
    println!("{} images, {} hashed in {took:.1?} ({:.1} ms each); suggested {t}: {} groups", files.len(), hashes.iter().flatten().count(), took.as_secs_f64() * 1000.0 / files.len().max(1) as f64, groups.len());
    for g in groups.iter().take(10) {
        println!("  {:?}", g.iter().map(|&i| files[i].file_name().unwrap().to_string_lossy()).collect::<Vec<_>>());
    }
}
