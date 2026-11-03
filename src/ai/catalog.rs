use super::*;

/// The biggest input side a model may ask for: it comes from the file, and it's squared into an allocation.
pub(super) const MAX_INPUT_SIDE: i64 = 4096;

/// How a model wants its input, and whether its output needs a sigmoid (BiRefNet gives logits).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preprocess {
    /// Input width and height.
    pub size: u32,
    pub mean: [f32; 3],
    pub std: [f32; 3],
    pub sigmoid: bool,
}

pub(super) const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
pub(super) const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

impl Preprocess {
    /// For a model lightview doesn't know: its input size, and the usual normalisation.
    pub fn auto(size: u32) -> Self {
        Preprocess { size, mean: IMAGENET_MEAN, std: IMAGENET_STD, sigmoid: false }
    }
}

pub(super) const U2NET: Preprocess = Preprocess { size: 320, mean: IMAGENET_MEAN, std: IMAGENET_STD, sigmoid: false };
const ISNET: Preprocess = Preprocess { size: 1024, mean: IMAGENET_MEAN, std: [1.0; 3], sigmoid: false };
const BIREFNET: Preprocess = Preprocess { size: 1024, mean: IMAGENET_MEAN, std: IMAGENET_STD, sigmoid: true };

/// A model lightview knows by file name: downloadable from `CATALOG`, pinned by checksum, or
/// copied in by the user (`SELF_CONVERTED`).
pub struct Known {
    pub file: &'static str,
    pub label: &'static str,
    pub task: Task,
    /// Size, both files together.
    pub mb: u32,
    pub(super) sha256: &'static str,
    /// Where `file` (and `data`) are downloaded from.
    pub(super) base: &'static str,
    /// The other files it needs, by name and sha256: the weights, when the `.onnx` keeps them
    /// separately, or Recognize Text's detector and character list.
    pub(super) data: &'static [(&'static str, &'static str)],
    /// Background removal's input settings (image-to-image models take the image as is).
    pub(super) pre: Option<Preprocess>,
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

const U2NET_LICENSE: License = License { project: "U²-Net", spdx: "Apache-2.0", url: "https://github.com/xuebinqin/U-2-Net", text: include_str!("../../licenses/u2net.txt") };
const DIS_LICENSE: License = License { project: "IS-Net (DIS)", spdx: "Apache-2.0", url: "https://github.com/xuebinqin/DIS", text: include_str!("../../licenses/dis.txt") };
const ANIME_LICENSE: License = License { project: "anime-segmentation", spdx: "Apache-2.0", url: "https://github.com/SkyTNT/anime-segmentation", text: include_str!("../../licenses/anime-segmentation.txt") };
const BIREFNET_LICENSE: License = License { project: "BiRefNet", spdx: "MIT", url: "https://github.com/ZhengPeng7/BiRefNet", text: include_str!("../../licenses/birefnet.txt") };
const REMBG_LICENSE: License = License { project: "rembg", spdx: "MIT", url: "https://github.com/danielgatis/rembg", text: include_str!("../../licenses/rembg.txt") };
const SCUNET_LICENSE: License = License { project: "SCUNet", spdx: "Apache-2.0", url: "https://github.com/cszn/SCUNet", text: include_str!("../../licenses/scunet.txt") };
const BSRGAN_LICENSE: License = License { project: "BSRGAN", spdx: "Apache-2.0", url: "https://github.com/cszn/BSRGAN", text: include_str!("../../licenses/bsrgan.txt") };
const LAMA_LICENSE: License = License { project: "LaMa", spdx: "Apache-2.0", url: "https://github.com/advimman/lama", text: include_str!("../../licenses/lama.txt") };
const MIGAN_LICENSE: License = License { project: "MI-GAN", spdx: "MIT", url: "https://github.com/Picsart-AI-Research/MI-GAN", text: include_str!("../../licenses/migan.txt") };
const PADDLEOCR_LICENSE: License = License { project: "PaddleOCR (PP-OCRv5)", spdx: "Apache-2.0", url: "https://github.com/PaddlePaddle/PaddleOCR", text: include_str!("../../licenses/paddleocr.txt") };
const SWINIR_LICENSE: License = License { project: "SwinIR", spdx: "Apache-2.0", url: "https://github.com/JingyunLiang/SwinIR", text: include_str!("../../licenses/swinir.txt") };
/// The licence files that come with the ONNX conversions (SCUNet's is KAIR's MIT licence).
const HELIOSOPH_SCUNET_LICENSE: License = License { project: "Heliosoph/scunet-onnx (the ONNX conversion)", spdx: "MIT", url: "https://huggingface.co/Heliosoph/scunet-onnx", text: include_str!("../../licenses/heliosoph-scunet.txt") };
const HELIOSOPH_SWINIR_LICENSE: License = License { project: "Heliosoph/swinir-onnx (the ONNX conversion)", spdx: "Apache-2.0", url: "https://huggingface.co/Heliosoph/swinir-onnx", text: include_str!("../../licenses/heliosoph-swinir.txt") };
pub const RUNTIME_LICENSE: License = License { project: "ONNX Runtime", spdx: "MIT", url: "https://github.com/microsoft/onnxruntime", text: include_str!("../../licenses/onnxruntime.txt") };

const RELEASES: &str = "https://github.com/danielgatis/rembg/releases/download/v0.0.0";
// Pinned to a revision, so the files (and their checksums) can't change under us.
const SCUNET: &str = "https://huggingface.co/Heliosoph/scunet-onnx/resolve/6d11417ee2fbcc73783c502a238ac115097754fe";
const SWINIR: &str = "https://huggingface.co/Heliosoph/swinir-onnx/resolve/a4a77e2c33c32cfa4f39953b2aa87afec7f34aef";
const BSRGAN: &str = "https://huggingface.co/FuryTMP/BSRGANx4_fp16/resolve/9359c20a3c91f930cdba220a31129ff9f8997791";
const LAMA: &str = "https://huggingface.co/Carve/LaMa-ONNX/resolve/c3c0c9e468934d62e79c329e35d82dd09ff8c444";
const MIGAN: &str = "https://huggingface.co/andraniksargsyan/migan/resolve/406830d0fa60666da0071c342ad2fbc8f30c5c64";
const PPOCR: &str = "https://huggingface.co/nakamura196/ppocrv5-mobile-web-onnx/resolve/dc7e2271c00d961a4c0a38dcebe23beb735ba33e";
pub(super) const LAMA_FILE: &str = "lama_fp32.onnx";
/// Recognize Text's detector, shared by its recognisers.
pub(super) const OCR_DETECTOR: (&str, &str) = ("PP-OCRv5_mobile_det.onnx", "6bdf743af375b13fbbeeb47a91a40063af5bfb7a7f197f6df8cb990dc827e5d6");
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

pub(super) fn models_dir() -> Result<PathBuf, String> {
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

pub(super) fn list_models(dir: &Path) -> Vec<String> {
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

pub(super) fn download_to(dir: &Path, k: &Known) -> Result<(), String> {
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
