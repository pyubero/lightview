//! AI background removal (optionally cropped to the subject). ONNX Runtime and the models aren't
//! shipped: Tools → Download AI Runtime… / Download … Model… install them on request.
use crate::settings::AiSettings;
use eframe::egui::{self, Pos2, Rect};
use image::{DynamicImage, GrayImage, RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Microsoft's CPU build of ONNX Runtime, the version `ort` (api-31) is made for.
const RUNTIME_URL: &str = "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-linux-x64-1.31.1.tgz";
const RUNTIME_SHA256: &str = "c849e36a1ef3e22d4ff217b9d1ce4a3cf5b9467b975610913ae959ad5b16bdc9";
/// The library inside the archive, and its name once installed.
const RUNTIME_MEMBER: &str = "onnxruntime-linux-x64-1.31.1/lib/libonnxruntime.so.1.31.1";
const RUNTIME_FILE: &str = "libonnxruntime.so.1.31.1";

fn data_dir() -> Result<PathBuf, String> {
    eframe::storage_dir("lightview").ok_or_else(|| "no data folder".to_string())
}

/// `~/.local/share/lightview/lib/libonnxruntime.so.1.31.1`.
fn runtime_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("lib").join(RUNTIME_FILE))
}

pub fn runtime_installed() -> bool {
    runtime_path().is_ok_and(|p| p.is_file())
}

/// Downloads ONNX Runtime and installs just its library (blocking).
pub fn download_runtime() -> Result<(), String> {
    let path = runtime_path()?;
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let archive = dir.join("onnxruntime.tgz.part");
    fetch(RUNTIME_URL, RUNTIME_SHA256, &archive)?;
    let unpacked = dir.join("onnxruntime.part");
    let result = (|| {
        std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
        // shortcut: unpacks with the system's tar; use the tar/flate2 crates for Windows.
        run(std::process::Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&unpacked).arg(RUNTIME_MEMBER))?;
        std::fs::rename(unpacked.join(RUNTIME_MEMBER), &path).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&archive);
    let _ = std::fs::remove_dir_all(&unpacked);
    result
}

/// Downloads `url` to `dest`, keeping it only if its SHA-256 is `sha256`.
// shortcut: uses the system's curl and sha256sum rather than HTTP/hash crates; add them if those can be missing (Windows).
fn fetch(url: &str, sha256: &str, dest: &Path) -> Result<(), String> {
    let result = run(std::process::Command::new("curl").args(["-L", "--fail", "--silent", "--show-error", "-o"]).arg(dest).arg(url)).and_then(|()| {
        let out = std::process::Command::new("sha256sum").arg(dest).output().map_err(|e| format!("sha256sum: {e}"))?;
        let sum = String::from_utf8_lossy(&out.stdout);
        if sum.split_whitespace().next() == Some(sha256) { Ok(()) } else { Err("the download didn't match its checksum".into()) }
    });
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

fn run(cmd: &mut std::process::Command) -> Result<(), String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd.output().map_err(|e| format!("{name}: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("{name}: {}", String::from_utf8_lossy(&out.stderr).trim())) }
}

/// Loads ONNX Runtime the first time an AI tool runs, never at startup.
fn environment() -> Result<&'static ort::environment::Environment, String> {
    static ENV: OnceLock<Result<ort::environment::Environment, String>> = OnceLock::new();
    ENV.get_or_init(|| {
        let builder = ort::init_from(runtime_path()?).map_err(|e| e.to_string())?;
        builder.build().map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// How a model wants its input, and whether its output needs a sigmoid (BiRefNet gives logits).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preprocess {
    /// Input width and height.
    pub size: u32,
    pub mean: [f32; 3],
    pub std: [f32; 3],
    pub sigmoid: bool,
}

const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

impl Preprocess {
    /// For a model lightview doesn't know: its input size, and the usual normalisation.
    pub fn auto(size: u32) -> Self {
        Preprocess { size, mean: IMAGENET_MEAN, std: IMAGENET_STD, sigmoid: false }
    }
}

const U2NET: Preprocess = Preprocess { size: 320, mean: IMAGENET_MEAN, std: IMAGENET_STD, sigmoid: false };
const ISNET: Preprocess = Preprocess { size: 1024, mean: IMAGENET_MEAN, std: [1.0; 3], sigmoid: false };
const BIREFNET: Preprocess = Preprocess { size: 1024, mean: IMAGENET_MEAN, std: IMAGENET_STD, sigmoid: true };

/// What a model is for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Task {
    /// Remove Background (and Crop): a mask.
    Background,
    /// Image to image, faithful.
    Denoise,
    /// Image to image, generative.
    Enhance,
}

impl Task {
    pub const ALL: [Task; 3] = [Task::Background, Task::Denoise, Task::Enhance];

    pub fn title(self) -> &'static str {
        match self {
            Task::Background => "Remove Background",
            Task::Denoise => "Denoise",
            Task::Enhance => "Enhance Image",
        }
    }
}

/// A model lightview can download, pinned by checksum.
pub struct Known {
    pub file: &'static str,
    pub label: &'static str,
    pub task: Task,
    /// Download size, both files together.
    pub mb: u32,
    sha256: &'static str,
    /// Where `file` (and `data`) are downloaded from.
    base: &'static str,
    /// The weights, when the `.onnx` keeps them in a separate file: its name and sha256.
    data: Option<(&'static str, &'static str)>,
    /// Background removal's input settings (image-to-image models take the image as is).
    pre: Option<Preprocess>,
    /// Who made the ONNX file, when it isn't the authors.
    pub converted_by: Option<&'static str>,
    /// Shown, and accepted by the user, before the download starts.
    pub licenses: &'static [License],
}

/// A third-party licence, embedded so it can be read before downloading (and offline).
pub struct License {
    pub project: &'static str,
    pub spdx: &'static str,
    pub url: &'static str,
    pub text: &'static str,
}

const U2NET_LICENSE: License = License { project: "U²-Net", spdx: "Apache-2.0", url: "https://github.com/xuebinqin/U-2-Net", text: include_str!("../licenses/u2net.txt") };
const DIS_LICENSE: License = License { project: "IS-Net (DIS)", spdx: "Apache-2.0", url: "https://github.com/xuebinqin/DIS", text: include_str!("../licenses/dis.txt") };
const ANIME_LICENSE: License = License { project: "anime-segmentation", spdx: "Apache-2.0", url: "https://github.com/SkyTNT/anime-segmentation", text: include_str!("../licenses/anime-segmentation.txt") };
const BIREFNET_LICENSE: License = License { project: "BiRefNet", spdx: "MIT", url: "https://github.com/ZhengPeng7/BiRefNet", text: include_str!("../licenses/birefnet.txt") };
const REMBG_LICENSE: License = License { project: "rembg", spdx: "MIT", url: "https://github.com/danielgatis/rembg", text: include_str!("../licenses/rembg.txt") };
const SCUNET_LICENSE: License = License { project: "SCUNet", spdx: "Apache-2.0", url: "https://github.com/cszn/SCUNet", text: include_str!("../licenses/scunet.txt") };
const SWINIR_LICENSE: License = License { project: "SwinIR", spdx: "Apache-2.0", url: "https://github.com/JingyunLiang/SwinIR", text: include_str!("../licenses/swinir.txt") };
/// The licence files that come with the ONNX conversions (SCUNet's is KAIR's MIT licence).
const HELIOSOPH_SCUNET_LICENSE: License = License { project: "Heliosoph/scunet-onnx (the ONNX conversion)", spdx: "MIT", url: "https://huggingface.co/Heliosoph/scunet-onnx", text: include_str!("../licenses/heliosoph-scunet.txt") };
const HELIOSOPH_SWINIR_LICENSE: License = License { project: "Heliosoph/swinir-onnx (the ONNX conversion)", spdx: "Apache-2.0", url: "https://huggingface.co/Heliosoph/swinir-onnx", text: include_str!("../licenses/heliosoph-swinir.txt") };
pub const RUNTIME_LICENSE: License = License { project: "ONNX Runtime", spdx: "MIT", url: "https://github.com/microsoft/onnxruntime", text: include_str!("../licenses/onnxruntime.txt") };

const RELEASES: &str = "https://github.com/danielgatis/rembg/releases/download/v0.0.0";
// Pinned to a revision, so the files (and their checksums) can't change under us.
const SCUNET: &str = "https://huggingface.co/Heliosoph/scunet-onnx/resolve/6d11417ee2fbcc73783c502a238ac115097754fe";
const SWINIR: &str = "https://huggingface.co/Heliosoph/swinir-onnx/resolve/a4a77e2c33c32cfa4f39953b2aa87afec7f34aef";
const HELIOSOPH: Option<&str> = Some("Converted to ONNX by Heliosoph LLC from the authors' published weights.");
const SCUNET_LICENSES: &[License] = &[SCUNET_LICENSE, HELIOSOPH_SCUNET_LICENSE];

/// In order of preference within each task: the first one installed is that task's default.
pub const CATALOG: [Known; 13] = [
    Known { file: "silueta.onnx", label: "Silueta — small and fast", task: Task::Background, mb: 44, sha256: "75da6c8d2f8096ec743d071951be73b4a8bc7b3e51d9a6625d63644f90ffeedb", base: RELEASES, data: None, pre: Some(U2NET), converted_by: None, licenses: &[U2NET_LICENSE, REMBG_LICENSE] },
    Known { file: "u2netp.onnx", label: "U²-Net small — fastest", task: Task::Background, mb: 5, sha256: "309c8469258dda742793dce0ebea8e6dd393174f89934733ecc8b14c76f4ddd8", base: RELEASES, data: None, pre: Some(U2NET), converted_by: None, licenses: &[U2NET_LICENSE] },
    Known { file: "u2net.onnx", label: "U²-Net — general", task: Task::Background, mb: 176, sha256: "8d10d2f3bb75ae3b6d527c77944fc5e7dcd94b29809d47a739a7a728a912b491", base: RELEASES, data: None, pre: Some(U2NET), converted_by: None, licenses: &[U2NET_LICENSE] },
    Known { file: "u2net_human_seg.onnx", label: "U²-Net people — portraits", task: Task::Background, mb: 176, sha256: "01eb6a29a5c4d8edb30b56adad9bb3a2a0535338e480724a213e0acfd2d1c73c", base: RELEASES, data: None, pre: Some(U2NET), converted_by: None, licenses: &[U2NET_LICENSE] },
    Known { file: "isnet-general-use.onnx", label: "IS-Net — general, clean edges", task: Task::Background, mb: 179, sha256: "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a", base: RELEASES, data: None, pre: Some(ISNET), converted_by: None, licenses: &[DIS_LICENSE] },
    Known { file: "isnet-anime.onnx", label: "IS-Net anime — illustrations", task: Task::Background, mb: 176, sha256: "f15622d853e8260172812b657053460e20806f04b9e05147d49af7bed31a6e99", base: RELEASES, data: None, pre: Some(ISNET), converted_by: None, licenses: &[ANIME_LICENSE] },
    Known { file: "BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx", label: "BiRefNet lite — best quality, slow", task: Task::Background, mb: 224, sha256: "5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333", base: RELEASES, data: None, pre: Some(BIREFNET), converted_by: None, licenses: &[BIREFNET_LICENSE] },
    Known { file: "scunet_color_real_psnr.onnx", label: "SCUNet — noise and JPEG artifacts (recommended)", task: Task::Denoise, mb: 77, sha256: "231be201ab413dbc999d7951caa9844846b93a12a40a41e037d6b5888ed4e88c", base: SCUNET, data: Some(("scunet_color_real_psnr.onnx.data", "98825ea1210b641c71e5f052f582c70c49fd44b35387ebe2c034268c17df3feb")), pre: None, converted_by: HELIOSOPH, licenses: SCUNET_LICENSES },
    Known { file: "scunet_color_15.onnx", label: "SCUNet σ15 — light noise", task: Task::Denoise, mb: 77, sha256: "228dfb72f8b5e23d35c0a92048b46ada31676b34e90a1ced6d221377861ffd6b", base: SCUNET, data: Some(("scunet_color_15.onnx.data", "4f2cdeeade2744b8fefdae9d572c8c71d84bc153e4863579c5f8fe680a3b1c74")), pre: None, converted_by: HELIOSOPH, licenses: SCUNET_LICENSES },
    Known { file: "scunet_color_25.onnx", label: "SCUNet σ25 — moderate noise", task: Task::Denoise, mb: 77, sha256: "76170f1fcb84d3b3d38c4e7133a658bbbb70504e3dc5e4e4d1f7b0c395c093be", base: SCUNET, data: Some(("scunet_color_25.onnx.data", "f46aa0f533e5bb8b2485a6531a205e270f4a7be2447880a12d97f5f00b1e70c9")), pre: None, converted_by: HELIOSOPH, licenses: SCUNET_LICENSES },
    Known { file: "scunet_color_50.onnx", label: "SCUNet σ50 — heavy noise", task: Task::Denoise, mb: 77, sha256: "691b995af0700809ddf882265de4a68373b2d2af9d4a89daca662b828686229d", base: SCUNET, data: Some(("scunet_color_50.onnx.data", "a12e0931304ea43d3367d5f5848099e46fdafe07fd2126ed207ceca02989d446")), pre: None, converted_by: HELIOSOPH, licenses: SCUNET_LICENSES },
    Known { file: "swinir_denoising_color_25.onnx", label: "SwinIR σ25 — moderate noise, very slow", task: Task::Denoise, mb: 55, sha256: "5fb427215c6a893b4c4aa27b6ab0aad66742fb9cb7ab24f65eff27b09cc2ea74", base: SWINIR, data: None, pre: None, converted_by: HELIOSOPH, licenses: &[SWINIR_LICENSE, HELIOSOPH_SWINIR_LICENSE] },
    Known { file: "scunet_color_real_gan.onnx", label: "SCUNet GAN — sharper, may invent detail", task: Task::Enhance, mb: 77, sha256: "e2bf014ad711f99322f2a94304431ffd6766b79ff00bcebb87d1a4f71c127b2d", base: SCUNET, data: Some(("scunet_color_real_gan.onnx.data", "1e8d582046abf4a773215445c248d0f54751a57d69d2a2312ec59f1150256b79")), pre: None, converted_by: HELIOSOPH, licenses: SCUNET_LICENSES },
];

fn models_dir() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("models"))
}

pub fn known(file: &str) -> Option<&'static Known> {
    CATALOG.iter().find(|k| k.file == file)
}

/// The `*.onnx` files in the models folder, sorted.
pub fn installed() -> Vec<String> {
    models_dir().map(|d| list_models(&d)).unwrap_or_default()
}

fn list_models(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.to_ascii_lowercase().ends_with(".onnx"))
        // A catalog model whose weights file is missing isn't usable.
        .filter(|f| known(f).and_then(|k| k.data).is_none_or(|(data, _)| dir.join(data).is_file()))
        .collect();
    files.sort();
    files
}

/// The installed models that can do `task`: its catalog models, and any model lightview doesn't know.
pub fn for_task(task: Task, installed: &[String]) -> Vec<String> {
    installed.iter().filter(|f| known(f).is_none_or(|k| k.task == task)).cloned().collect()
}

/// The model `task` uses: the user's choice if it's installed, else the first installed
/// catalog model for it; background removal also falls back to any model it can use.
pub fn choose(choice: Option<&String>, task: Task, installed: &[String]) -> Option<String> {
    choice
        .filter(|f| installed.contains(f))
        .cloned()
        .or_else(|| CATALOG.iter().find(|k| k.task == task && installed.iter().any(|f| f == k.file)).map(|k| k.file.to_string()))
        .or_else(|| if task == Task::Background { for_task(task, installed).into_iter().next() } else { None })
}

/// The preprocessing for `file`: the user's override, else the catalog preset; None means
/// "read the size from the model".
pub fn preset(file: &str, overrides: &HashMap<String, Preprocess>) -> Option<Preprocess> {
    overrides.get(file).copied().or_else(|| known(file).and_then(|k| k.pre))
}

/// Downloads a catalog model (blocking). Writes to a temporary file first, so a failed or
/// tampered download never leaves a model behind.
pub fn download(k: &Known) -> Result<(), String> {
    download_to(&models_dir()?, k)
}

fn download_to(dir: &Path, k: &Known) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let files: Vec<(&str, &str)> = std::iter::once((k.file, k.sha256)).chain(k.data).collect();
    let part = |name: &str| dir.join(format!("{name}.part"));
    // Fetch and check every file before installing any, so a failure leaves nothing half-installed.
    for (name, sha) in &files {
        if let Err(e) = fetch(&format!("{}/{name}", k.base), sha, &part(name)) {
            files.iter().for_each(|(n, _)| drop(std::fs::remove_file(part(n))));
            return Err(e);
        }
    }
    // The weights go in first: the .onnx only counts as installed once they're there.
    for (name, _) in files.iter().rev() {
        std::fs::rename(part(name), dir.join(name)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Copies a model file the user picked into the models folder; never replaces one.
pub fn add_model(src: &Path) -> Result<String, String> {
    let name = src.file_name().and_then(|n| n.to_str()).filter(|n| n.to_ascii_lowercase().ends_with(".onnx")).ok_or("pick an .onnx file")?.to_string();
    let dir = models_dir()?;
    let dest = dir.join(&name);
    if dest.exists() {
        return Err(format!("{name} is already installed"));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("{name}.part"));
    let result = std::fs::copy(src, &tmp).map_err(|e| e.to_string()).and_then(|_| std::fs::rename(&tmp, &dest).map_err(|e| e.to_string()));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map(|()| name)
}

/// Opens the models folder in the file manager.
pub fn open_folder() {
    if let Ok(dir) = models_dir() {
        let _ = std::fs::create_dir_all(&dir);
        // shortcut: xdg-open is Linux only.
        drop(std::process::Command::new("xdg-open").arg(dir).spawn());
    }
}

fn session(file: &str) -> Result<ort::session::Session, String> {
    let path = models_dir()?.join(file);
    // One thread per physical core was ~10% faster than one per hardware thread here.
    // shortcut: assumes two hardware threads per core; read the real core count if that's wrong somewhere.
    let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
    let builder = ort::session::Session::builder(environment()?).map_err(|e| e.to_string())?;
    let mut builder = builder.with_intra_threads(threads).map_err(|e| e.to_string())?;
    builder.commit_from_file(path).map_err(|e| e.to_string())
}

/// Denoise / Enhance: runs an image-to-image model over `img` in overlapping tiles.
/// `progress` gets 0–1 after each tile; setting `cancel` stops it between tiles.
pub fn restore(file: &str, img: &RgbaImage, progress: &dyn Fn(f32), cancel: &AtomicBool) -> Result<RgbaImage, String> {
    let mut session = session(file)?;
    // A model exported with a fixed size only takes tiles of that size.
    let fixed = session.inputs().first().and_then(|i| i.dtype().tensor_shape()).and_then(|shape| match shape.iter().copied().collect::<Vec<i64>>().as_slice() {
        [.., h, w] if h == w && *h > 0 => Some(*h as usize),
        _ => None,
    });
    // Big tiles: per pixel they cost the same. A smaller image gets a tile just big enough
    // (a multiple of 64, which these models accept), so the preview isn't padded out.
    let long = img.width().max(img.height()) as usize;
    let tile = fixed.unwrap_or(long.div_ceil(64).max(1) * 64).min(fixed.unwrap_or(512));
    let run = |input: Vec<f32>, t: usize| -> Result<Vec<f32>, String> {
        let tensor = ort::value::Tensor::from_array(([1usize, 3, t, t], input)).map_err(|e| e.to_string())?;
        let outputs = session.run(ort::inputs![tensor]).map_err(|e| e.to_string())?;
        let (_, out) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        out.get(..3 * t * t).map(<[f32]>::to_vec).ok_or_else(|| "the model's output isn't the size of its input".to_string())
    };
    tiled(img, tile, tile / 8, run, progress, cancel)
}

/// Runs `run` (RGB planes in 0–1, `tile`×`tile`) over `img` in tiles overlapping by `overlap`,
/// blending the overlaps with linear ramps. Alpha is kept from `img`.
fn tiled(img: &RgbaImage, tile: usize, overlap: usize, mut run: impl FnMut(Vec<f32>, usize) -> Result<Vec<f32>, String>, progress: &dyn Fn(f32), cancel: &AtomicBool) -> Result<RgbaImage, String> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let starts = |len: usize| -> Vec<usize> {
        if len <= tile {
            return vec![0];
        }
        // As few tiles as cover `len` with at least `overlap` between neighbours, spread evenly.
        let n = (len - overlap).div_ceil(tile - overlap);
        (0..n).map(|i| i * (len - tile) / (n - 1)).collect()
    };
    let (xs, ys) = (starts(w), starts(h));
    let (mut acc, mut weights) = (vec![0f32; 3 * w * h], vec![0f32; w * h]);
    let plane = tile * tile;
    // Full weight in the middle, fading towards an edge that another tile overlaps.
    let ramp = |i: usize, before: bool, after: bool| {
        let r = |d: usize| (d + 1) as f32 / (overlap + 1) as f32;
        let mut v = 1f32;
        if before {
            v = v.min(r(i));
        }
        if after {
            v = v.min(r(tile - 1 - i));
        }
        v
    };
    for (n, (&y0, &x0)) in ys.iter().flat_map(|y| xs.iter().map(move |x| (y, x))).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
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
        let out = run(input, tile)?;
        for ty in 0..tile.min(h - y0) {
            let wy = ramp(ty, y0 > 0, y0 + tile < h);
            for tx in 0..tile.min(w - x0) {
                let wt = wy * ramp(tx, x0 > 0, x0 + tile < w);
                let i = (y0 + ty) * w + x0 + tx;
                for c in 0..3 {
                    acc[3 * i + c] += out[c * plane + ty * tile + tx] * wt;
                }
                weights[i] += wt;
            }
        }
        progress((n + 1) as f32 / (xs.len() * ys.len()) as f32);
    }
    let mut result = img.clone();
    for (i, px) in result.pixels_mut().enumerate() {
        for c in 0..3 {
            px[c] = (acc[3 * i + c] / weights[i] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Ok(result)
}

/// The model's input size, from its first input's shape (…, H, W); 1024 when it's dynamic.
pub fn input_size(file: &str) -> Result<u32, String> {
    let session = session(file)?;
    let shape = session.inputs().first().and_then(|i| i.dtype().tensor_shape()).ok_or("the model has no image input")?;
    let dims: Vec<i64> = shape.iter().copied().collect();
    Ok(match dims.as_slice() {
        [.., h, w] if h == w && *h > 0 => *h as u32,
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
    let pred = pred.get(..s * s).ok_or("the model's output doesn't match its input size")?;
    Ok(to_mask(pred, pre.size, pre.sigmoid, img.width(), img.height()))
}

/// The model's input: resized, scaled to 0–1, normalised, in channel-first order.
fn input(pre: &Preprocess, img: &RgbaImage) -> Vec<f32> {
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

/// The licence prompt's body: Some(true) to accept and download, Some(false) to cancel.
/// One licence of a download (`step` of `count`, from 0). Some(true) accepts it, Some(false) cancels.
/// The text scrolls within the window's height, so the buttons stay on screen.
pub fn license_ui(ui: &mut egui::Ui, title: &str, l: &License, step: usize, count: usize, note: Option<&str>) -> Option<bool> {
    let of = if count > 1 { format!(" ({} of {count})", step + 1) } else { String::new() };
    ui.heading(format!("Licence{of} — {title}"));
    ui.label("Please read and accept the licence before downloading.");
    if let Some(note) = note {
        ui.weak(note);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.strong(format!("{} — {}", l.project, l.spdx));
        if ui.button("Open project page").clicked() {
            // shortcut: xdg-open is Linux only.
            drop(std::process::Command::new("xdg-open").arg(l.url).spawn());
        }
    });
    // Room for the heading, notes and buttons around the text.
    let height = (ui.ctx().content_rect().height() - 260.0).clamp(100.0, 360.0);
    egui::ScrollArea::vertical().id_salt(("license", step)).max_height(height).show(ui, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(l.text).monospace()).wrap());
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let accept = ui.button(if step + 1 < count { "Accept and Continue" } else { "Accept and Download" }).clicked();
        let cancel = ui.button("Cancel").clicked();
        accept.then_some(true).or(cancel.then_some(false))
    })
    .inner
}

/// What the AI Models dialog asks for this frame.
#[derive(Default)]
pub struct ModelsOut {
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
    ui.heading("AI Models");
    if !runtime {
        ui.colored_label(ui.visuals().warn_fg_color, "The AI runtime isn't installed yet: Tools → Download AI Runtime…");
    }
    let name = |f: &str| {
        let mb = models_dir().ok().and_then(|d| std::fs::metadata(d.join(f)).ok()).map_or(0, |m| m.len().div_ceil(1_000_000));
        match known(f) {
            Some(k) => format!("{} ({} MB)", k.label, k.mb),
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
                ui.weak("No model installed yet.");
            } else {
                ui.horizontal(|ui| {
                    ui.label("Model");
                    egui::ComboBox::from_id_salt(("ai-model", task.title())).selected_text(current.as_deref().map(name).unwrap_or_default()).show_ui(ui, |ui| {
                        for f in &models {
                            if ui.selectable_label(current.as_ref() == Some(f), name(f)).on_hover_text(f).clicked() {
                                *s.choice_mut(task) = Some(f.clone());
                            }
                        }
                    });
                });
                if task == Task::Background {
                    ui.weak("Used by Remove Background and Remove Background and Crop.");
                    for f in &models {
                        egui::CollapsingHeader::new(format!("Preprocessing: {f}")).id_salt(("pre", f)).show(ui, |ui| preprocess_ui(ui, s, f, runtime, &mut out));
                    }
                }
            }
            let mut available: Vec<(usize, &Known)> = CATALOG.iter().enumerate().filter(|(_, k)| k.task == task && !installed.iter().any(|f| f == k.file)).collect();
            available.sort_by_key(|(_, k)| k.mb);
            egui::Grid::new(("catalog-grid", task.title())).num_columns(3).spacing([16.0, 6.0]).show(ui, |ui| {
                for (i, k) in available {
                    ui.label(k.label);
                    ui.label(format!("{} MB", k.mb));
                    let downloading = busy.contains(k.file);
                    if ui.add_enabled(!downloading, egui::Button::new(if downloading { "Downloading…" } else { "Download" })).clicked() {
                        out.download = Some(i);
                    }
                    ui.end_row();
                }
            });
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        out.add = ui.button("Add model file…").clicked();
        if ui.button("Open models folder").clicked() {
            open_folder();
        }
        out.close = ui.button("Close").clicked();
    });
    out
}

/// One model's preprocessing; any change becomes that file's override.
fn preprocess_ui(ui: &mut egui::Ui, s: &mut AiSettings, f: &str, runtime: bool, out: &mut ModelsOut) {
    let current = preset(f, &s.overrides);
    if current.is_none() {
        ui.weak("Size read from the model when it runs; the usual normalisation.");
    }
    let mut p = current.unwrap_or(Preprocess::auto(1024));
    let before = p;
    egui::Grid::new(("pre-grid", f)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Input size");
        ui.add(egui::DragValue::new(&mut p.size).range(32..=4096).suffix(" px"));
        ui.end_row();
        for (label, values, range) in [("Mean (R, G, B)", &mut p.mean, 0.0..=1.0), ("Std (R, G, B)", &mut p.std, 0.001..=10.0)] {
            ui.label(label);
            ui.horizontal(|ui| {
                for v in values.iter_mut() {
                    ui.add(egui::DragValue::new(v).range(range.clone()).speed(0.001).fixed_decimals(3));
                }
            });
            ui.end_row();
        }
        ui.label("Output");
        ui.checkbox(&mut p.sigmoid, "Apply sigmoid (the model outputs logits)");
        ui.end_row();
    });
    if p != before {
        s.overrides.insert(f.to_string(), p);
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(runtime, egui::Button::new("Detect size")).on_hover_text("Read the input size from the model").clicked() {
            out.detect = Some(f.to_string());
        }
        if ui.add_enabled(s.overrides.contains_key(f), egui::Button::new("Reset to default")).clicked() {
            s.overrides.remove(f);
        }
    });
}

/// Stretches the prediction to 0–255 (like rembg) and scales it to `w`×`h`.
fn to_mask(pred: &[f32], size: u32, sigmoid: bool, w: u32, h: u32) -> GrayImage {
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

#[cfg(test)]
mod tests {
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
        // A model that brightens: every pixel is changed exactly once (weights add up).
        // 102 is 0.4; 0.4 + 0.2 is 153.
        let img = RgbaImage::from_pixel(100, 60, image::Rgba([102, 102, 102, 255]));
        let out = tiled(&img, 32, 4, |d, _| Ok(d.iter().map(|v| v + 0.2).collect()), &|_| {}, &no).unwrap();
        assert!(out.pixels().all(|p| p.0 == [153, 153, 153, 255]));
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
            if let Some((data, sha)) = k.data {
                assert_eq!((data, sha.len()), (&*format!("{}.data", k.file), 64));
            }
        }
        // Each task's first entry is its default.
        assert_eq!(CATALOG.iter().find(|k| k.task == Task::Denoise).unwrap().file, "scunet_color_real_psnr.onnx");
        assert_eq!(CATALOG.iter().find(|k| k.task == Task::Enhance).unwrap().file, "scunet_color_real_gan.onnx");
    }

    #[test]
    fn two_file_download_is_all_or_nothing() {
        let root = std::env::temp_dir().join(format!("lightview-dl-{}", std::process::id()));
        let (src, dest) = (root.join("src"), root.join("models"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("m.onnx"), b"graph").unwrap();
        std::fs::write(src.join("m.onnx.data"), b"weights").unwrap();
        let sha = |f: &str| String::from_utf8(std::process::Command::new("sha256sum").arg(src.join(f)).output().unwrap().stdout).unwrap()[..64].to_string();
        let (graph, weights) = (sha("m.onnx").leak() as &str, sha("m.onnx.data").leak() as &str);
        let base = format!("file://{}", src.display()).leak();
        let known = |data_sha: &'static str| Known { file: "m.onnx", label: "", task: Task::Denoise, mb: 0, sha256: graph, base, data: Some(("m.onnx.data", data_sha)), pre: None, converted_by: None, licenses: &[] };
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
        let mixed = installed(&["mine.onnx", "scunet_color_15.onnx", "scunet_color_real_gan.onnx", "scunet_color_real_psnr.onnx", "silueta.onnx"]);
        assert_eq!(choose(None, Task::Denoise, &mixed).as_deref(), Some("scunet_color_real_psnr.onnx"));
        assert_eq!(choose(None, Task::Enhance, &mixed).as_deref(), Some("scunet_color_real_gan.onnx"));
        assert_eq!(choose(None, Task::Denoise, &installed(&["silueta.onnx", "mine.onnx"])), None);
        assert_eq!(choose(Some(&"mine.onnx".into()), Task::Denoise, &mixed).as_deref(), Some("mine.onnx"));
        assert_eq!(for_task(Task::Enhance, &mixed), ["mine.onnx", "scunet_color_real_gan.onnx"]);
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

    /// Installs the runtime the way the menu does: `cargo test install_runtime -- --ignored`.
    #[test]
    #[ignore]
    fn install_runtime() {
        download_runtime().unwrap();
        assert!(runtime_installed());
        environment().unwrap();
    }

    #[test]
    fn bad_checksum_is_refused() {
        let (src, dest) = (std::env::temp_dir().join(format!("lightview-fetch-src-{}", std::process::id())), std::env::temp_dir().join(format!("lightview-fetch-{}", std::process::id())));
        std::fs::write(&src, b"not what was expected").unwrap();
        let result = fetch(&format!("file://{}", src.display()), "0000", &dest);
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
}
