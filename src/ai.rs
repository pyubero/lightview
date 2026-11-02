//! AI background removal (optionally cropped to the subject). ONNX Runtime and the models aren't
//! shipped: Tools → Download AI Runtime… / Download … Model… install them on request.
use crate::i18n::{tr, tr_key, trf};
use crate::settings::AiSettings;
use eframe::egui::{self, Pos2, Rect};
use image::{DynamicImage, GrayImage, RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Microsoft's CPU build of ONNX Runtime, the version `ort` (api-31) is made for.
#[cfg(not(windows))]
const RUNTIME_URL: &str = "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-linux-x64-1.31.1.tgz";
#[cfg(not(windows))]
const RUNTIME_SHA256: &str = "c849e36a1ef3e22d4ff217b9d1ce4a3cf5b9467b975610913ae959ad5b16bdc9";
/// The library inside the archive, and its name once installed.
#[cfg(not(windows))]
const RUNTIME_MEMBER: &str = "onnxruntime-linux-x64-1.31.1/lib/libonnxruntime.so.1.31.1";
#[cfg(not(windows))]
const RUNTIME_FILE: &str = "libonnxruntime.so.1.31.1";
#[cfg(windows)]
const RUNTIME_URL: &str = "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-win-x64-1.31.1.zip";
#[cfg(windows)]
const RUNTIME_SHA256: &str = "5869e26bb25314375f2d5ef1877e3ea06b1543d6c710e6e4a1ec2f7177ab4b2c";
#[cfg(windows)]
const RUNTIME_MEMBER: &str = "onnxruntime-win-x64-1.31.1/lib/onnxruntime.dll";
#[cfg(windows)]
const RUNTIME_FILE: &str = "onnxruntime.dll";

fn data_dir() -> Result<PathBuf, String> {
    eframe::storage_dir("lightview").ok_or_else(|| tr("no data folder").to_string())
}

/// `~/.local/share/lightview/lib/libonnxruntime.so.1.31.1` (on Windows, `onnxruntime.dll` in lightview's AppData folder).
fn runtime_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("lib").join(RUNTIME_FILE))
}

/// The CPU runtime, or the GPU support (whose ONNX Runtime also runs on the CPU).
pub fn runtime_installed() -> bool {
    runtime_path().is_ok_and(|p| p.is_file()) || gpu_installed()
}

/// ONNX Runtime's CUDA 12 build (Microsoft) and the NVIDIA libraries it needs (NVIDIA's own
/// PyPI packages), pinned by checksum. CUDA 12, not 13: 13 no longer runs on Pascal cards
/// such as the GTX 10xx. Installed into `~/.local/share/lightview/gpu`.
struct GpuPart {
    url: &'static str,
    sha256: &'static str,
    /// tar members of the ONNX Runtime archive, or None for a wheel (its `nvidia/*/lib` or `bin` files).
    members: Option<&'static [&'static str]>,
}

#[cfg(not(windows))]
const GPU_PARTS: [GpuPart; 5] = [
    GpuPart {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-linux-x64-gpu_cuda12-1.31.1.tgz",
        sha256: "57cd51f677eca19a269cdba31b0c16c15d73a8feea41f170aa8b750447ab1f70",
        members: Some(&[
            "onnxruntime-linux-x64-gpu_cuda12-1.31.1/lib/libonnxruntime.so.1.31.1",
            "onnxruntime-linux-x64-gpu_cuda12-1.31.1/lib/libonnxruntime_providers_cuda.so",
            "onnxruntime-linux-x64-gpu_cuda12-1.31.1/lib/libonnxruntime_providers_shared.so",
        ]),
    },
    GpuPart {
        url: "https://files.pythonhosted.org/packages/bc/46/a92db19b8309581092a3add7e6fceb4c301a3fd233969856a8cbf042cd3c/nvidia_cuda_runtime_cu12-12.9.79-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl",
        sha256: "25bba2dfb01d48a9b59ca474a1ac43c6ebf7011f1b0b8cc44f54eb6ac48a96c3",
        members: None,
    },
    GpuPart {
        url: "https://files.pythonhosted.org/packages/af/eb/ff4b8c503fa1f1796679dce648854d58751982426e4e4b37d6fce49d259c/nvidia_cublas_cu12-12.6.4.1-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl",
        sha256: "08ed2686e9875d01b58e3cb379c6896df8e76c75e0d4a7f7dace3d7b6d9ef8eb",
        members: None,
    },
    GpuPart {
        url: "https://files.pythonhosted.org/packages/73/1b/44a01c4e70933637c93e6e1a8063d1e998b50213a6b65ac5a9169c47e98e/nvidia_curand_cu12-10.3.7.77-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl",
        sha256: "a42cd1344297f70b9e39a1e4f467a4e1c10f1da54ff7a85c12197f6c652c8bdf",
        members: None,
    },
    GpuPart {
        url: "https://files.pythonhosted.org/packages/36/24/50e8018ddf113473dcebbc7c109d2843ec948740f5abed89f1885c9a982e/nvidia_cudnn_cu12-9.5.0.50-py3-none-manylinux2014_x86_64.whl",
        sha256: "518d6cffad7d707871dfb9af667ddb619144f78d85b07f64c1e52449e6da6b9c",
        members: None,
    },
];

#[cfg(windows)]
const GPU_PARTS: [GpuPart; 5] = [
    GpuPart {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-win-x64-gpu_cuda12-1.31.1.zip",
        sha256: "5cbf65edc93dc7bb3296249b1b6e1b2228e575902af64d6163d0da2103cef9d5",
        members: Some(&[
            "onnxruntime-win-x64-gpu_cuda12-1.31.1/lib/onnxruntime.dll",
            "onnxruntime-win-x64-gpu_cuda12-1.31.1/lib/onnxruntime_providers_cuda.dll",
            "onnxruntime-win-x64-gpu_cuda12-1.31.1/lib/onnxruntime_providers_shared.dll",
        ]),
    },
    GpuPart {
        url: "https://files.pythonhosted.org/packages/59/df/e7c3a360be4f7b93cee39271b792669baeb3846c58a4df6dfcf187a7ffab/nvidia_cuda_runtime_cu12-12.9.79-py3-none-win_amd64.whl",
        sha256: "8e018af8fa02363876860388bd10ccb89eb9ab8fb0aa749aaf58430a9f7c4891",
        members: None,
    },
    GpuPart {
        url: "https://files.pythonhosted.org/packages/84/f7/985e9bdbe3e0ac9298fcc8cfa51a392862a46a0ffaccbbd56939b62a9c83/nvidia_cublas_cu12-12.6.4.1-py3-none-win_amd64.whl",
        sha256: "9e4fa264f4d8a4eb0cdbd34beadc029f453b3bafae02401e999cf3d5a5af75f8",
        members: None,
    },
    GpuPart {
        url: "https://files.pythonhosted.org/packages/a9/a8/0cd0cec757bd4b4b4ef150fca62ec064db7d08a291dced835a0be7d2c147/nvidia_curand_cu12-10.3.7.77-py3-none-win_amd64.whl",
        sha256: "6d6d935ffba0f3d439b7cd968192ff068fafd9018dbf1b85b37261b13cfc9905",
        members: None,
    },
    GpuPart {
        url: "https://files.pythonhosted.org/packages/66/b8/4ace228fae5a3b6a3b3972156fe1392f7f91199feaae9a46c7eb60303274/nvidia_cudnn_cu12-9.5.0.50-py3-none-win_amd64.whl",
        sha256: "508c801f70baa8496e79b3338a55f7816488da815c0d55158a7adfc4e073a11e",
        members: None,
    },
];

/// Download size of `GPU_PARTS`, for the menu.
pub const GPU_DOWNLOAD_MB: u32 = if cfg!(windows) { 1440 } else { 1470 };

/// The NVIDIA libraries the CUDA provider needs, loaded before it in this order: it has no
/// search path of its own, and a library that's already loaded satisfies it by name.
#[cfg(not(windows))]
const CUDA_LIBS: [&str; 5] = ["libcudart.so.12", "libcublasLt.so.12", "libcublas.so.12", "libcurand.so.10", "libcudnn.so.9"];
#[cfg(windows)]
const CUDA_LIBS: [&str; 5] = ["cudart64_12.dll", "cublasLt64_12.dll", "cublas64_12.dll", "curand64_10.dll", "cudnn64_9.dll"];

/// The GPU build of ONNX Runtime and its CUDA provider, as installed.
const GPU_RUNTIME: &str = if cfg!(windows) { "onnxruntime.dll" } else { "libonnxruntime.so.1.31.1" };
const GPU_PROVIDER: &str = if cfg!(windows) { "onnxruntime_providers_cuda.dll" } else { "libonnxruntime_providers_cuda.so" };

const NVIDIA_CUDA_LICENSE: License = License { project: "NVIDIA CUDA runtime, cuBLAS, cuRAND", spdx: "NVIDIA proprietary licence", url: "https://docs.nvidia.com/cuda/eula/", text: include_str!("../licenses/nvidia-cuda.txt") };
const NVIDIA_CUDNN_LICENSE: License = License { project: "NVIDIA cuDNN", spdx: "NVIDIA proprietary licence", url: "https://docs.nvidia.com/deeplearning/cudnn/latest/reference/eula.html", text: include_str!("../licenses/nvidia-cudnn.txt") };
/// Shown one after the other before the GPU download.
pub const GPU_LICENSES: [License; 3] = [RUNTIME_LICENSE, NVIDIA_CUDA_LICENSE, NVIDIA_CUDNN_LICENSE];

fn gpu_dir() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("gpu"))
}

pub fn gpu_installed() -> bool {
    gpu_dir().is_ok_and(|d| d.join(GPU_RUNTIME).is_file() && d.join(GPU_PROVIDER).is_file())
}

/// Whether an NVIDIA driver is loaded; without one the GPU support can't do anything.
pub fn nvidia_driver() -> bool {
    if cfg!(windows) {
        // The driver installs its CUDA library into System32.
        std::env::var_os("SystemRoot").is_some_and(|root| Path::new(&root).join("System32").join("nvcuda.dll").exists())
    } else {
        Path::new("/proc/driver/nvidia/version").exists()
    }
}

/// Downloads and installs the GPU support (blocking; ~1.5 GB). Everything is fetched, checked
/// and unpacked into a temporary folder first, which only replaces the old one at the end.
pub fn download_gpu() -> Result<(), String> {
    let dir = gpu_dir()?;
    let tmp = dir.with_extension("part");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let result = (|| {
        for (i, part) in GPU_PARTS.iter().enumerate() {
            let archive = tmp.join(format!("download-{i}"));
            fetch(part.url, part.sha256, &archive)?;
            #[cfg(windows)]
            crate::platform::unzip(&archive, &tmp, part.members.unwrap_or(&["nvidia/*/bin/*.dll"]))?;
            // The system's tools: GNU tar reads the ONNX Runtime .tgz, unzip the wheels.
            #[cfg(not(windows))]
            match part.members {
                Some(members) => run(crate::platform::command("tar").arg("-xf").arg(&archive).arg("-C").arg(&tmp).arg("--strip-components=2").args(members))?,
                None => run(crate::platform::command("unzip").args(["-q", "-j", "-o"]).arg(&archive).arg("nvidia/*/lib/lib*.so*").arg("-d").arg(&tmp))?,
            }
            // The archives are big: drop each one once it's unpacked.
            std::fs::remove_file(&archive).map_err(|e| e.to_string())?;
        }
        if !CUDA_LIBS.iter().all(|l| tmp.join(l).is_file()) {
            return Err(tr("a GPU library was missing from the download").into());
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&tmp, &dir).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    result
}

/// Whether sessions should try the GPU (the user's setting).
static USE_GPU: AtomicBool = AtomicBool::new(true);

pub fn set_use_gpu(on: bool) {
    USE_GPU.store(on, Ordering::Relaxed);
}

/// Why the last session couldn't use the GPU, if it couldn't.
static GPU_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn gpu_error() -> Option<String> {
    GPU_ERROR.lock().ok()?.clone()
}

/// ONNX Runtime's CUDA errors are long C++ traces; keep the "CUDA failure …" part.
fn short_error(e: &str) -> String {
    let e = e.find("CUDA failure").map_or(e, |i| &e[i..]);
    let e = e.split(" ;").next().unwrap_or(e).trim();
    if e.chars().count() > 160 { format!("{}…", e.chars().take(160).collect::<String>()) } else { e.to_string() }
}

/// Whether the loaded ONNX Runtime is the GPU build (decided once, when it's first loaded).
static GPU_LOADED: AtomicBool = AtomicBool::new(false);

/// The GPU support is installed but lightview started with the CPU runtime: it needs a restart.
pub fn gpu_needs_restart() -> bool {
    gpu_installed() && ENV_LOADED.load(Ordering::Relaxed) && !GPU_LOADED.load(Ordering::Relaxed)
}

static ENV_LOADED: AtomicBool = AtomicBool::new(false);

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
        #[cfg(windows)]
        crate::platform::unzip(&archive, &unpacked, &[RUNTIME_MEMBER])?;
        #[cfg(not(windows))]
        run(crate::platform::command("tar").arg("-xf").arg(&archive).arg("-C").arg(&unpacked).arg("--strip-components=2").arg(RUNTIME_MEMBER))?;
        std::fs::rename(unpacked.join(RUNTIME_FILE), &path).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&archive);
    let _ = std::fs::remove_dir_all(&unpacked);
    result
}

/// Downloads `url` to `dest`, keeping it only if its SHA-256 is `sha256`.
fn fetch(url: &str, sha256: &str, dest: &Path) -> Result<(), String> {
    let result = crate::platform::download(url, dest).and_then(|()| {
        if sha256_file(dest)? == sha256 { Ok(()) } else { Err(tr("the download didn't match its checksum").into()) }
    });
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

/// The file's SHA-256, in lowercase hex.
fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match std::io::Read::read(&mut file, &mut buf).map_err(|e| e.to_string())? {
            0 => break,
            n => hasher.update(&buf[..n]),
        }
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub fn run(cmd: &mut std::process::Command) -> Result<(), String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd.output().map_err(|e| format!("{name}: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("{name}: {}", String::from_utf8_lossy(&out.stderr).trim())) }
}

/// Loads ONNX Runtime the first time an AI tool runs, never at startup.
/// With the GPU support installed, its ONNX Runtime is loaded instead (it runs on the CPU too).
fn environment() -> Result<&'static ort::environment::Environment, String> {
    static ENV: OnceLock<Result<ort::environment::Environment, String>> = OnceLock::new();
    ENV.get_or_init(|| {
        ENV_LOADED.store(true, Ordering::Relaxed);
        let lib = if gpu_installed() {
            let dir = gpu_dir()?;
            // Windows doesn't look for a DLL's dependencies next to it, and cuDNN loads its parts
            // by name: let the GPU folder be searched for the rest of the process.
            #[cfg(windows)]
            {
                use std::os::windows::ffi::OsStrExt;
                let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
                unsafe { windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW(wide.as_ptr()) };
            }
            // Kept loaded for the life of the process, for the CUDA provider to find.
            for name in CUDA_LIBS {
                match unsafe { libloading::Library::new(dir.join(name)) } {
                    Ok(lib) => std::mem::forget(lib),
                    Err(e) => *GPU_ERROR.lock().unwrap() = Some(format!("{name}: {e}")),
                }
            }
            GPU_LOADED.store(true, Ordering::Relaxed);
            dir.join(GPU_RUNTIME)
        } else {
            runtime_path()?
        };
        let builder = ort::init_from(lib).map_err(|e| e.to_string())?;
        builder.build().map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// The biggest input side a model may ask for: it comes from the file, and it's squared into an allocation.
const MAX_INPUT_SIDE: i64 = 4096;

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
    /// Fills in a selected area.
    Inpaint,
    /// Finds and reads text: a recogniser, with the detector and character list as its `data`.
    Ocr,
}

impl Task {
    pub const ALL: [Task; 5] = [Task::Background, Task::Denoise, Task::Enhance, Task::Inpaint, Task::Ocr];

    pub fn title(self) -> &'static str {
        match self {
            Task::Background => tr("Remove Background"),
            Task::Denoise => tr("Denoise"),
            Task::Enhance => tr("Enhance Image"),
            Task::Inpaint => tr("Remove Object"),
            Task::Ocr => tr("Recognize Text"),
        }
    }
}

/// A model lightview knows by file name: downloadable from `CATALOG`, pinned by checksum, or
/// copied in by the user (`SELF_CONVERTED`).
pub struct Known {
    pub file: &'static str,
    pub label: &'static str,
    pub task: Task,
    /// Size, both files together.
    pub mb: u32,
    sha256: &'static str,
    /// Where `file` (and `data`) are downloaded from.
    base: &'static str,
    /// The other files it needs, by name and sha256: the weights, when the `.onnx` keeps them
    /// separately, or Recognize Text's detector and character list.
    data: &'static [(&'static str, &'static str)],
    /// Background removal's input settings (image-to-image models take the image as is).
    pre: Option<Preprocess>,
    /// Who made the ONNX file, when it isn't the authors.
    pub converted_by: Option<&'static str>,
    /// How many times bigger the output is (an upscaler), else 1.
    pub scale: u32,
    /// What the notice before its first use says (Enhance models), when not the generative warning.
    pub notice: Option<&'static str>,
    /// The largest tile it runs on (image-to-image models). Bigger tiles give models that look at
    /// the whole picture (DarkIR) fewer seams; upscalers need smaller ones to fit a 4 GB GPU.
    pub max_tile: u32,
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
const BSRGAN_LICENSE: License = License { project: "BSRGAN", spdx: "Apache-2.0", url: "https://github.com/cszn/BSRGAN", text: include_str!("../licenses/bsrgan.txt") };
const LAMA_LICENSE: License = License { project: "LaMa", spdx: "Apache-2.0", url: "https://github.com/advimman/lama", text: include_str!("../licenses/lama.txt") };
const MIGAN_LICENSE: License = License { project: "MI-GAN", spdx: "MIT", url: "https://github.com/Picsart-AI-Research/MI-GAN", text: include_str!("../licenses/migan.txt") };
const PADDLEOCR_LICENSE: License = License { project: "PaddleOCR (PP-OCRv5)", spdx: "Apache-2.0", url: "https://github.com/PaddlePaddle/PaddleOCR", text: include_str!("../licenses/paddleocr.txt") };
const SWINIR_LICENSE: License = License { project: "SwinIR", spdx: "Apache-2.0", url: "https://github.com/JingyunLiang/SwinIR", text: include_str!("../licenses/swinir.txt") };
/// The licence files that come with the ONNX conversions (SCUNet's is KAIR's MIT licence).
const HELIOSOPH_SCUNET_LICENSE: License = License { project: "Heliosoph/scunet-onnx (the ONNX conversion)", spdx: "MIT", url: "https://huggingface.co/Heliosoph/scunet-onnx", text: include_str!("../licenses/heliosoph-scunet.txt") };
const HELIOSOPH_SWINIR_LICENSE: License = License { project: "Heliosoph/swinir-onnx (the ONNX conversion)", spdx: "Apache-2.0", url: "https://huggingface.co/Heliosoph/swinir-onnx", text: include_str!("../licenses/heliosoph-swinir.txt") };
pub const RUNTIME_LICENSE: License = License { project: "ONNX Runtime", spdx: "MIT", url: "https://github.com/microsoft/onnxruntime", text: include_str!("../licenses/onnxruntime.txt") };

const RELEASES: &str = "https://github.com/danielgatis/rembg/releases/download/v0.0.0";
// Pinned to a revision, so the files (and their checksums) can't change under us.
const SCUNET: &str = "https://huggingface.co/Heliosoph/scunet-onnx/resolve/6d11417ee2fbcc73783c502a238ac115097754fe";
const SWINIR: &str = "https://huggingface.co/Heliosoph/swinir-onnx/resolve/a4a77e2c33c32cfa4f39953b2aa87afec7f34aef";
const BSRGAN: &str = "https://huggingface.co/FuryTMP/BSRGANx4_fp16/resolve/9359c20a3c91f930cdba220a31129ff9f8997791";
const LAMA: &str = "https://huggingface.co/Carve/LaMa-ONNX/resolve/c3c0c9e468934d62e79c329e35d82dd09ff8c444";
const MIGAN: &str = "https://huggingface.co/andraniksargsyan/migan/resolve/406830d0fa60666da0071c342ad2fbc8f30c5c64";
const PPOCR: &str = "https://huggingface.co/nakamura196/ppocrv5-mobile-web-onnx/resolve/dc7e2271c00d961a4c0a38dcebe23beb735ba33e";
const LAMA_FILE: &str = "lama_fp32.onnx";
/// Recognize Text's detector, shared by its recognisers.
const OCR_DETECTOR: (&str, &str) = ("PP-OCRv5_mobile_det.onnx", "6bdf743af375b13fbbeeb47a91a40063af5bfb7a7f197f6df8cb990dc827e5d6");
const NAKAMURA: Option<&str> = Some(tr_key("Converted to ONNX by Satoru Nakamura (University of Tokyo) from PaddlePaddle's published models."));
const HELIOSOPH: Option<&str> = Some(tr_key("Converted to ONNX by Heliosoph LLC from the authors' published weights."));
const SCUNET_LICENSES: &[License] = &[SCUNET_LICENSE, HELIOSOPH_SCUNET_LICENSE];

/// In order of preference within each task: the first one installed is that task's default.
pub const CATALOG: [Known; 17] = [
    Known { file: "silueta.onnx", label: tr_key("Silueta — small and fast"), task: Task::Background, mb: 44, sha256: "75da6c8d2f8096ec743d071951be73b4a8bc7b3e51d9a6625d63644f90ffeedb", base: RELEASES, data: &[], pre: Some(U2NET), converted_by: None, scale: 1, max_tile: 512, notice: None, licenses: &[U2NET_LICENSE, REMBG_LICENSE] },
    Known { file: "u2netp.onnx", label: tr_key("U²-Net small — fastest"), task: Task::Background, mb: 5, sha256: "309c8469258dda742793dce0ebea8e6dd393174f89934733ecc8b14c76f4ddd8", base: RELEASES, data: &[], pre: Some(U2NET), converted_by: None, scale: 1, max_tile: 512, notice: None, licenses: &[U2NET_LICENSE] },
    Known { file: "u2net.onnx", label: tr_key("U²-Net — general"), task: Task::Background, mb: 176, sha256: "8d10d2f3bb75ae3b6d527c77944fc5e7dcd94b29809d47a739a7a728a912b491", base: RELEASES, data: &[], pre: Some(U2NET), converted_by: None, scale: 1, max_tile: 512, notice: None, licenses: &[U2NET_LICENSE] },
    Known { file: "u2net_human_seg.onnx", label: tr_key("U²-Net people — portraits"), task: Task::Background, mb: 176, sha256: "01eb6a29a5c4d8edb30b56adad9bb3a2a0535338e480724a213e0acfd2d1c73c", base: RELEASES, data: &[], pre: Some(U2NET), converted_by: None, scale: 1, max_tile: 512, notice: None, licenses: &[U2NET_LICENSE] },
    Known { file: "isnet-general-use.onnx", label: tr_key("IS-Net — general, clean edges"), task: Task::Background, mb: 179, sha256: "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a", base: RELEASES, data: &[], pre: Some(ISNET), converted_by: None, scale: 1, max_tile: 512, notice: None, licenses: &[DIS_LICENSE] },
    Known { file: "isnet-anime.onnx", label: tr_key("IS-Net anime — illustrations"), task: Task::Background, mb: 176, sha256: "f15622d853e8260172812b657053460e20806f04b9e05147d49af7bed31a6e99", base: RELEASES, data: &[], pre: Some(ISNET), converted_by: None, scale: 1, max_tile: 512, notice: None, licenses: &[ANIME_LICENSE] },
    Known { file: "BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx", label: tr_key("BiRefNet lite — best quality, slow"), task: Task::Background, mb: 224, sha256: "5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333", base: RELEASES, data: &[], pre: Some(BIREFNET), converted_by: None, scale: 1, max_tile: 512, notice: None, licenses: &[BIREFNET_LICENSE] },
    Known { file: "scunet_color_real_psnr.onnx", label: tr_key("SCUNet — noise and JPEG artifacts (recommended)"), task: Task::Denoise, mb: 77, sha256: "231be201ab413dbc999d7951caa9844846b93a12a40a41e037d6b5888ed4e88c", base: SCUNET, data: &[("scunet_color_real_psnr.onnx.data", "98825ea1210b641c71e5f052f582c70c49fd44b35387ebe2c034268c17df3feb")], pre: None, converted_by: HELIOSOPH, scale: 1, max_tile: 512, notice: None, licenses: SCUNET_LICENSES },
    Known { file: "scunet_color_15.onnx", label: tr_key("SCUNet σ15 — light noise"), task: Task::Denoise, mb: 77, sha256: "228dfb72f8b5e23d35c0a92048b46ada31676b34e90a1ced6d221377861ffd6b", base: SCUNET, data: &[("scunet_color_15.onnx.data", "4f2cdeeade2744b8fefdae9d572c8c71d84bc153e4863579c5f8fe680a3b1c74")], pre: None, converted_by: HELIOSOPH, scale: 1, max_tile: 512, notice: None, licenses: SCUNET_LICENSES },
    Known { file: "scunet_color_25.onnx", label: tr_key("SCUNet σ25 — moderate noise"), task: Task::Denoise, mb: 77, sha256: "76170f1fcb84d3b3d38c4e7133a658bbbb70504e3dc5e4e4d1f7b0c395c093be", base: SCUNET, data: &[("scunet_color_25.onnx.data", "f46aa0f533e5bb8b2485a6531a205e270f4a7be2447880a12d97f5f00b1e70c9")], pre: None, converted_by: HELIOSOPH, scale: 1, max_tile: 512, notice: None, licenses: SCUNET_LICENSES },
    Known { file: "scunet_color_50.onnx", label: tr_key("SCUNet σ50 — heavy noise"), task: Task::Denoise, mb: 77, sha256: "691b995af0700809ddf882265de4a68373b2d2af9d4a89daca662b828686229d", base: SCUNET, data: &[("scunet_color_50.onnx.data", "a12e0931304ea43d3367d5f5848099e46fdafe07fd2126ed207ceca02989d446")], pre: None, converted_by: HELIOSOPH, scale: 1, max_tile: 512, notice: None, licenses: SCUNET_LICENSES },
    Known { file: "swinir_denoising_color_25.onnx", label: tr_key("SwinIR σ25 — moderate noise, very slow"), task: Task::Denoise, mb: 55, sha256: "5fb427215c6a893b4c4aa27b6ab0aad66742fb9cb7ab24f65eff27b09cc2ea74", base: SWINIR, data: &[], pre: None, converted_by: HELIOSOPH, scale: 1, max_tile: 512, notice: None, licenses: &[SWINIR_LICENSE, HELIOSOPH_SWINIR_LICENSE] },
    Known { file: "BSRGANx4_fp16.onnx", label: tr_key("BSRGAN 4× — upscales and sharpens, may invent detail"), task: Task::Enhance, mb: 33, sha256: "9682732b50a4ab4d78a3c84220702b4063e11cccb2bf50ca27ea2b572dedd173", base: BSRGAN, data: &[], pre: None, converted_by: Some(tr_key("ONNX export (half precision) by FuryTMP on Hugging Face; its origin isn't verified.")), scale: 4, max_tile: 256, notice: None, licenses: &[BSRGAN_LICENSE] },
    Known { file: LAMA_FILE, label: tr_key("LaMa — best quality, slower"), task: Task::Inpaint, mb: 208, sha256: "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6", base: LAMA, data: &[], pre: None, converted_by: Some(tr_key("ONNX export by Carve on Hugging Face; its origin isn't verified.")), scale: 1, max_tile: 512, notice: None, licenses: &[LAMA_LICENSE] },
    Known { file: "migan_pipeline_v2.onnx", label: tr_key("MI-GAN — fast"), task: Task::Inpaint, mb: 28, sha256: "6f1f3530a1a2324b19752018ce756088b07973cda8d7d890034ace5c8a48c40b", base: MIGAN, data: &[], pre: None, converted_by: None, scale: 1, max_tile: 512, notice: None, licenses: &[MIGAN_LICENSE] },
    Known { file: "latin_PP-OCRv5_mobile_rec.onnx", label: tr_key("PP-OCRv5 Latin — European languages"), task: Task::Ocr, mb: 13, sha256: "46449fd0606cd07d856744703211ea6826e4642d4c803d91167c53cde01d3b94", base: PPOCR, data: &[OCR_DETECTOR, ("latin_PP-OCRv5_mobile_rec.dict.json", "705592cc40b955ac82cdfdb049ec75f0a9041824d524eeb946a913a34d285409")], pre: None, converted_by: NAKAMURA, scale: 1, max_tile: 512, notice: None, licenses: &[PADDLEOCR_LICENSE] },
    Known { file: "PP-OCRv5_mobile_rec.onnx", label: tr_key("PP-OCRv5 — Chinese, Japanese and English"), task: Task::Ocr, mb: 22, sha256: "1205c2e14cc3ecaf7a57fe14736521ded32489603c40d98e36fe4212a70d8be5", base: PPOCR, data: &[OCR_DETECTOR, ("PP-OCRv5_mobile_rec.dict.json", "02c0a0121c577dfa211f11de21084e3abd23478a77ea4fbdf71ee7a3e990a717")], pre: None, converted_by: NAKAMURA, scale: 1, max_tile: 512, notice: None, licenses: &[PADDLEOCR_LICENSE] },
];

const FROM_LIGHTVIEW_MODELS: Known = Known { file: "", label: "", task: Task::Enhance, mb: 0, sha256: "", base: "", data: &[], pre: None, converted_by: None, scale: 4, max_tile: 256, notice: None, licenses: &[] };

/// Where users convert the `SELF_CONVERTED` models.
pub const LIGHTVIEW_MODELS_URL: &str = "https://github.com/pyubero/lightview-models";

/// Models nobody publishes as ONNX: each user converts them with lightview-models and copies the
/// files into the models folder, so they're never offered for download. Lightview only recognises
/// them by file name, for their task, tile size and notice. In order of preference, after `CATALOG`.
pub const SELF_CONVERTED: [Known; 5] = [
    Known { file: "RealESRGAN_x4plus_fp32.onnx", label: tr_key("Real-ESRGAN 4× — upscales, smooth, may invent detail"), mb: 67, ..FROM_LIGHTVIEW_MODELS },
    Known { file: "RealESRGAN_x4plus_fp16.onnx", label: tr_key("Real-ESRGAN 4× half precision"), mb: 34, ..FROM_LIGHTVIEW_MODELS },
    Known { file: "realesr-general-x4v3_fp32.onnx", label: tr_key("Real-ESRGAN general 4× — fast, softer"), mb: 5, ..FROM_LIGHTVIEW_MODELS },
    Known { file: "realesr-general-x4v3_fp16.onnx", label: tr_key("Real-ESRGAN general 4× half precision"), mb: 3, ..FROM_LIGHTVIEW_MODELS },
    Known { file: "DarkIR_384.onnx", label: tr_key("DarkIR — dark and night photos, same size"), mb: 14, scale: 1, max_tile: 1024, notice: Some(tr_key("DarkIR brightens dark and night photos and removes their noise and blur. On a well-lit photo it overexposes.")), ..FROM_LIGHTVIEW_MODELS },
];

fn models_dir() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("models"))
}

/// Every model lightview knows by name, in order of preference.
fn recognised() -> impl Iterator<Item = &'static Known> {
    CATALOG.iter().chain(&SELF_CONVERTED)
}

pub fn known(file: &str) -> Option<&'static Known> {
    recognised().find(|k| k.file == file)
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
        // A catalog model with a file missing isn't usable.
        .filter(|f| known(f).is_none_or(|k| k.data.iter().all(|(data, _)| dir.join(data).is_file())))
        // Nor is another model's part (the OCR detector) a model of its own.
        .filter(|f| !recognised().any(|k| k.data.iter().any(|(data, _)| data == f)))
        .collect();
    files.sort();
    files
}

/// The installed models that can do `task`: those it knows for `task`, and any model it doesn't
/// know, except for Inpaint and Recognize Text, whose inputs differ per model.
pub fn for_task(task: Task, installed: &[String]) -> Vec<String> {
    installed.iter().filter(|f| known(f).map_or(!matches!(task, Task::Inpaint | Task::Ocr), |k| k.task == task)).cloned().collect()
}

/// The model `task` uses: the user's choice if it's installed, else the first installed model
/// known for it; background removal also falls back to any model it can use.
pub fn choose(choice: Option<&String>, task: Task, installed: &[String]) -> Option<String> {
    choice
        .filter(|f| installed.contains(f))
        .cloned()
        .or_else(|| recognised().find(|k| k.task == task && installed.iter().any(|f| f == k.file)).map(|k| k.file.to_string()))
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
    let files: Vec<(&str, &str)> = std::iter::once((k.file, k.sha256)).chain(k.data.iter().copied()).collect();
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
    let name = src.file_name().and_then(|n| n.to_str()).filter(|n| n.to_ascii_lowercase().ends_with(".onnx")).ok_or(tr("pick an .onnx file"))?.to_string();
    let dir = models_dir()?;
    let dest = dir.join(&name);
    if dest.exists() {
        return Err(trf("{name} is already installed", &[("name", &name)]));
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
        crate::platform::open(dir);
    }
}

/// The last inpainting model, by file and GPU setting. LaMa takes ~18 s to load (at any
/// optimisation level) and 1–4 s to run, and objects tend to be removed in a series.
static INPAINTER: std::sync::Mutex<Option<((String, bool), ort::session::Session)>> = std::sync::Mutex::new(None);

fn session(file: &str) -> Result<ort::session::Session, String> {
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
fn context_rect(region: Rect, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let side = (region.width().max(region.height()) as u32 * 3 / 2).max(512);
    let (cw, ch) = (side.min(w), side.min(h));
    let place = |center: f32, len: u32, max: u32| (center as u32).saturating_sub(len / 2).min(max - len);
    (place(region.center().x, cw, w), place(region.center().y, ch, h), cw, ch)
}

/// Channel-first bytes, as the inpainting models take them.
fn to_planes(rgb: &image::RgbImage) -> Vec<u8> {
    (0..3).flat_map(|c| rgb.pixels().map(move |p| p[c])).collect()
}

fn from_planes(data: &[u8], w: u32, h: u32) -> Result<image::RgbImage, String> {
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
fn fill_context(img: &RgbaImage, region: Rect, hole: &GrayImage, size: Option<u32>, run: impl FnOnce(image::RgbImage, GrayImage) -> Result<image::RgbImage, String>) -> Result<RgbaImage, String> {
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
fn text_boxes(prob: &[f32], w: u32, h: u32) -> Vec<Rect> {
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
fn ctc_decode(probs: &[f32], classes: usize, dict: &[String]) -> String {
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
fn join_lines(mut boxes: Vec<(Rect, String)>) -> String {
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

/// The GPU row of the AI Models dialog.
fn gpu_ui(ui: &mut egui::Ui, use_gpu: &mut bool, downloading: bool, out: &mut ModelsOut) {
    if !nvidia_driver() && !gpu_installed() {
        return;
    }
    ui.add_space(4.0);
    ui.strong(tr("GPU (NVIDIA)"));
    if gpu_installed() {
        ui.checkbox(use_gpu, tr("Use the GPU")).on_hover_text(tr("Denoise and Enhance run about 10× faster on a GTX 1050"));
        if gpu_needs_restart() {
            ui.colored_label(ui.visuals().warn_fg_color, tr("Restart lightview to start using the GPU."));
        } else if let Some(e) = gpu_error().filter(|_| *use_gpu) {
            ui.colored_label(ui.visuals().warn_fg_color, trf("The GPU couldn't be used, so the CPU is: {e}", &[("e", &short_error(&e))]));
        }
    } else {
        ui.horizontal(|ui| {
            ui.label(trf("Run the AI tools on the graphics card ({mb} MB download, NVIDIA's CUDA libraries).", &[("mb", &GPU_DOWNLOAD_MB)]));
            out.download_gpu = ui.add_enabled(!downloading, egui::Button::new(if downloading { tr("Downloading…") } else { tr("Download GPU Support") })).clicked();
        });
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
        ui.colored_label(ui.visuals().warn_fg_color, tr("The AI runtime isn't installed yet: Tools > Download AI Runtime…"));
    }
    gpu_ui(ui, &mut s.use_gpu, busy.contains("gpu"), &mut out);
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
        let font = ab_glyph::FontRef::try_from_slice(include_bytes!("../fonts/Roboto-Regular.ttf")).unwrap();
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
}
