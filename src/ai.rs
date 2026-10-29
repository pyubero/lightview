//! AI background removal and object extraction. ONNX Runtime and the models aren't
//! shipped: Tools → Download AI Runtime… / Download … Model… install them on request.
use eframe::egui::{Pos2, Rect};
use image::{DynamicImage, GrayImage, RgbaImage, imageops::FilterType};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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

/// A segmentation model (U²-Net family, as used by rembg): one RGB input, a saliency map out.
pub struct Model {
    pub name: &'static str,
    file: &'static str,
    url: &'static str,
    sha256: &'static str,
    /// Input width and height.
    size: u32,
    mean: [f32; 3],
    std: [f32; 3],
}

/// Remove Background: small and fast (43 MB).
pub const BACKGROUND: Model = Model {
    name: "background removal",
    file: "silueta.onnx",
    url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/silueta.onnx",
    sha256: "75da6c8d2f8096ec743d071951be73b4a8bc7b3e51d9a6625d63644f90ffeedb",
    size: 320,
    mean: [0.485, 0.456, 0.406],
    std: [0.229, 0.224, 0.225],
};

/// Extract Object: higher resolution, cleaner edges (179 MB).
pub const OBJECT: Model = Model {
    name: "object extraction",
    file: "isnet-general-use.onnx",
    url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx",
    sha256: "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a",
    size: 1024,
    mean: [0.485, 0.456, 0.406],
    std: [1.0, 1.0, 1.0],
};

impl Model {
    /// `~/.local/share/lightview/models/<file>`.
    fn path(&self) -> Result<PathBuf, String> {
        Ok(data_dir()?.join("models").join(self.file))
    }

    pub fn installed(&self) -> bool {
        self.path().is_ok_and(|p| p.is_file())
    }

    /// Downloads the model (blocking). Writes to a temporary file first, so a failed or
    /// tampered download never leaves a model behind.
    pub fn download(&self) -> Result<(), String> {
        let path = self.path()?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("onnx.part");
        fetch(self.url, self.sha256, &tmp)?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    /// The foreground mask (255 = keep) for `img`, at its size.
    pub fn mask(&self, img: &RgbaImage) -> Result<GrayImage, String> {
        let input = self.input(img);
        let model = self.path()?;
        let mut session = ort::session::Session::builder(environment()?).and_then(|mut b| b.commit_from_file(model)).map_err(|e| e.to_string())?;
        let s = self.size as usize;
        let tensor = ort::value::Tensor::from_array(([1usize, 3, s, s], input)).map_err(|e| e.to_string())?;
        let outputs = session.run(ort::inputs![tensor]).map_err(|e| e.to_string())?;
        let (_, pred) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let pred = pred.get(..s * s).ok_or("unexpected model output")?;
        Ok(to_mask(pred, self.size, img.width(), img.height()))
    }

    /// The model's input: resized, scaled to 0–1, normalised, in channel-first order.
    fn input(&self, img: &RgbaImage) -> Vec<f32> {
        let small = DynamicImage::ImageRgba8(img.clone()).resize_exact(self.size, self.size, FilterType::Triangle).into_rgb8();
        let plane = (self.size * self.size) as usize;
        let mut input = vec![0f32; 3 * plane];
        for (i, px) in small.pixels().enumerate() {
            for c in 0..3 {
                input[c * plane + i] = (px[c] as f32 / 255.0 - self.mean[c]) / self.std[c];
            }
        }
        input
    }
}

/// Stretches the prediction to 0–255 (like rembg) and scales it to `w`×`h`.
fn to_mask(pred: &[f32], size: u32, w: u32, h: u32) -> GrayImage {
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
        let mask = to_mask(&[0.1, 0.1, 0.1, 0.9], 2, 4, 4);
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
        let m = Model { size: 2, ..BACKGROUND };
        let input = m.input(&img);
        assert_eq!(input.len(), 3 * 4);
        assert!(input[..4].iter().all(|&v| v > 2.0) && input[4..].iter().all(|&v| v < -1.0));
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
    /// (needs the runtime and model; `LIGHTVIEW_AI_MODEL=object` for the other one); writes `photo-cutout.png` to the temp folder.
    #[test]
    #[ignore]
    fn remove_background() {
        let path = PathBuf::from(std::env::var("LIGHTVIEW_AI").unwrap());
        let img = image::open(&path).unwrap().into_rgba8();
        let t = std::time::Instant::now();
        let model = if std::env::var("LIGHTVIEW_AI_MODEL").is_ok_and(|m| m == "object") { &OBJECT } else { &BACKGROUND };
        let mask = model.mask(&img).unwrap();
        eprintln!("mask in {:.0} ms, foreground {:?}", t.elapsed().as_secs_f64() * 1000.0, bbox(&mask));
        let out = std::env::temp_dir().join(format!("{}-cutout.png", path.file_stem().unwrap().to_string_lossy()));
        cutout(&img, &mask).save(&out).unwrap();
        eprintln!("wrote {}", out.display());
    }
}
