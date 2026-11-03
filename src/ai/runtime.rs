use super::*;

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

const NVIDIA_CUDA_LICENSE: License = License { project: "NVIDIA CUDA runtime, cuBLAS, cuRAND", spdx: "NVIDIA proprietary licence", url: "https://docs.nvidia.com/cuda/eula/", text: include_str!("../../licenses/nvidia-cuda.txt") };
const NVIDIA_CUDNN_LICENSE: License = License { project: "NVIDIA cuDNN", spdx: "NVIDIA proprietary licence", url: "https://docs.nvidia.com/deeplearning/cudnn/latest/reference/eula.html", text: include_str!("../../licenses/nvidia-cudnn.txt") };
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
pub(super) static USE_GPU: AtomicBool = AtomicBool::new(true);

pub fn set_use_gpu(on: bool) {
    USE_GPU.store(on, Ordering::Relaxed);
}

/// Why the last session couldn't use the GPU, if it couldn't.
pub(super) static GPU_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn gpu_error() -> Option<String> {
    GPU_ERROR.lock().ok()?.clone()
}

/// ONNX Runtime's CUDA errors are long C++ traces; keep the "CUDA failure …" part.
pub(super) fn short_error(e: &str) -> String {
    let e = e.find("CUDA failure").map_or(e, |i| &e[i..]);
    let e = e.split(" ;").next().unwrap_or(e).trim();
    if e.chars().count() > 160 { format!("{}…", e.chars().take(160).collect::<String>()) } else { e.to_string() }
}

/// Whether the loaded ONNX Runtime is the GPU build (decided once, when it's first loaded).
pub(super) static GPU_LOADED: AtomicBool = AtomicBool::new(false);

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
pub(super) fn fetch(url: &str, sha256: &str, dest: &Path) -> Result<(), String> {
    let result = crate::platform::download(url, dest).and_then(|()| {
        if sha256_file(dest)? == sha256 { Ok(()) } else { Err(tr("the download didn't match its checksum").into()) }
    });
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

/// The file's SHA-256, in lowercase hex.
pub(super) fn sha256_file(path: &Path) -> Result<String, String> {
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
pub(super) fn environment() -> Result<&'static ort::environment::Environment, String> {
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
