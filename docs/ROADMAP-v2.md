# lightview — Roadmap v2

*Status: done (2026-10-29 to 2026-10-31). Editing, markup, the inspector, the AI tools, translations and the Windows port. v1 (milestones 1–7) is in `PROJECT_BRIEF.md`; v3 continues in `ROADMAP-v3.md`.*

## Principles (still hold)
- **Fast.** P1a (window under 200 ms), P2 (next/previous in one frame) and P3 (the UI never waits on decoding). New features must not slow down startup or navigation.
- **The original is safe.** lightview never changes the file you opened. Edits go to a copy.
- **Mostly Rust.** Any new C library is loaded at run time and is optional, like libjpeg-turbo.
- **No new setting without the user's approval.**

## Decisions (2026-10-29)
- **Edit list:** edits (crop, resize, colour, rotate/flip, markup) live on the current image with undo/redo, show at once, and are applied to the full image only when saving a copy.
- **Selection tools:** rectangle, ellipse and lasso. (No smart lasso: reopened by Select Object, `ROADMAP-v4.md`.)
- **No JPEG 2000:** no pure-Rust encoder. TIFF added instead.
- The **inspector sidebar** replaces the Info dialog and takes in Edit EXIF.
- **Rotate/flip are saved edits** (replaces M4's view-only rule). **GIF export** added.
- **Unsaved edits:** lightview asks before switching image or closing.
- **Paste** adds a movable element, so Selection came after Draw.
- **Dependencies approved:** `arboard`, `tiny-skia`, `ab_glyph`, `ort`.
- AI behind an optional `ai` cargo feature: **reversed in M16**, one binary for everyone.

## Done
Numbering continues from v1. Menu names are given as they are now (see `MENUS.md`); the menus were reorganised in M8 and again on 2026-11-03 (Local AI menu).

### 8. Menus and View ✅ (2026-10-29)
- Previous / Next Image, Zoom In / Out, Actual Size, Fit to Window, Full Screen; Animation: Play, Pause, Next / Previous Frame.
- Each action appears in exactly one menu. No Export submenu (Save As picks the format); Crop crops to the selection when there is one; Settings in Edit (Ctrl+,); EXIF editing and the image description through the inspector (Ctrl+I).

### 9. Edit list, undo/redo, Crop, Resize, Save As ✅ (2026-10-29)
- Undo (Ctrl+Z) and Redo (Ctrl+Shift+Z) over snapshots of the edits. Crop (drag, Enter); Resize… (width, height, aspect ratio, percentage, DPI).
- `save.rs` replays the edits on the full decoded image. Save As writes JPEG, PNG, WebP, BMP, TIFF or GIF.

### 10. Colour adjustments ✅ (2026-10-29)
- Exposure, Contrast, Saturation and Tint, plus Auto Levels and Reset. On screen they run on the small resized output; at save time on the full image. The panel is one undo step.

### 11. Inspector sidebar ✅ (2026-10-29)
- General (name, format, size, dimensions, resolution, colour profile), EXIF (camera, lens, exposure, date, GPS; editable, always saving a copy), Location (Open in map, remove location), Accessibility (image description).

### 12. Measure ✅ (2026-10-29)
- A temporary overlay, never saved: drag a line, ΔX, ΔY and distance in image pixels at any zoom and rotation. One measurement per image.

### 13. Markup ✅ (2026-10-29)
- Drawing toolbar: Select, Sketch, Shapes, Text, Highlight, Magnifier, Measure, Description, style controls, Undo, Redo, Done. Elements stay editable until saving, which flattens them with `tiny-skia` and `ab_glyph`.

### 14. Selection and clipboard ✅ (2026-10-29)
- Rectangle, ellipse and lasso; Select All, Deselect, Cut, Copy, Paste, Delete Selection. Cut and Delete leave transparency, or the background colour where the format has no alpha. Paste adds a movable element. Clipboard through eframe (`arboard`, Wayland included).

### 15–16. AI runtime on request ✅ (2026-10-29)
- Remove Background (and Crop). `ort` loads ONNX Runtime at run time (load-dynamic), first used by an AI tool, never at startup. Local AI → Download AI Runtime… installs ONNX Runtime 1.31.1 in `~/.local/share/lightview/lib`. Every download is pinned by SHA-256; a mismatch is deleted.

### 17. Choose and add AI models ✅ (2026-10-29)
- Local AI → AI Models…: a catalog of rembg models (Silueta, U²-Net small/full/people, IS-Net, IS-Net anime, BiRefNet lite), a model drop-down per task (saved in Settings), Add model file… (never overwrites), Open models folder.
- Every download first shows its upstream licence (embedded from `licenses/`) and starts only after Accept.
- Per-file preprocessing (input size, mean, std, sigmoid), editable and saved; unknown models read their input size from the file.

### 18. Denoise and Enhance Image ✅ (2026-10-29)
- Denoise: SCUNet real-PSNR (default; SCUNet σ15/25/50 and SwinIR σ25 as alternatives). Apache-2.0, Heliosoph LLC's ONNX conversions pinned by revision and SHA-256. SUPIR dropped (non-commercial, multi-GB, GPU).
- CPU speed (i7-8750H, turbo off): SCUNet ≈ 57 s/MP, SwinIR ≈ 4× slower. So each tool previews the visible area (≤ 256 px, ≈ 5 s) as a before/after with a draggable divider, then Apply to Whole Image or to Selection runs in the background with progress, time left and Cancel, as one undoable edit.
- The first Enhance shows a notice that it may alter fine detail (per model, once).

### 18b. Enhance Image = BSRGAN 4× ✅ (2026-10-29)
- BSRGAN (cszn, Apache-2.0) from the half-precision ONNX export `FuryTMP/BSRGANx4_fp16` (33 MB, pinned; its origin isn't verified). SCUNet real-GAN removed.
- 256 px tiles (a 512 px tile needs a 2 GB buffer); tile outputs written straight into the result, not blended (a 4× 12 MP result is ~770 MB). Apply to Selection isn't offered for upscalers.
- Measured: ~1.6 s per 256 px tile on a GTX 1050, ~21 s on the CPU (12 MP ≈ 250 tiles: ≈ 7 min GPU, > 1 h CPU). Pasted images are shown at most 4096 px across.

### 18c. Self-converted models: DarkIR and Real-ESRGAN ✅ (2026-10-30)
- Converted in the separate `lightview-models` repo, which publishes no ONNX files: users convert them and copy them in, and lightview recognises them by file name (`SELF_CONVERTED`), listed under Enhance (see `README.md`).
- DarkIR-m (CVPR 2025; code MIT, weights CC-BY-4.0): brightens night photos at the same size. Its FFT block was rewritten as real matrix products (matches to ~4e-6); it needs 1024 px tiles (`Known::max_tile`): 1.2 GB on a 4 GB GPU, ~5.4 s/MP there, ~23 s/MP on the CPU.
- Real-ESRGAN x4plus and realesr-general-x4v3 (fp32 and fp16), on 256 px tiles.

### 19. AI on NVIDIA GPUs ✅ (2026-10-29)
- Optional GPU download in AI Models… (shown with an NVIDIA driver): ONNX Runtime 1.31.1's CUDA 12 build plus cudart, cuBLAS, cuRAND and cuDNN from NVIDIA's PyPI packages, ~1.5 GB download, 2.1 GB installed, each file pinned, three licences shown first. CUDA 12 because 13 drops Pascal (GTX 10xx).
- "Use the GPU" (on by default), with CPU fallback and the reason shown; installing needs a restart.
- GTX 1050: Denoise ≈ 5.8 s/MP vs ≈ 57 s/MP on the CPU (≈ 10×), same output (max 1/255 difference), peak 3.1 GB.

### 20. Multilingual UI ✅ (2026-10-30)
- English text as the key (`tr`/`trf`, `locales/*.tsv`, `tr_key` in `const` tables); six languages at first, nine since 0.3.0. First launch follows the system language. Licences, OS error detail, shortcuts and format names stay English.

### 21. Windows ✅, experimental (2026-10-31)
- One code base; per-OS parts in `platform.rs` (links, folders, helper programs without console windows, PowerShell for downloads and unzipping, since a real Windows 11 PC had no `curl.exe`) and per-OS constants in `ai/`. Checksums in Rust (`sha2`).
- `turbojpeg.dll` ships next to `lightview.exe` (loaded only from there); the exe has its icon embedded (`build.rs`).
- GitHub Actions builds the `.deb` and the `.msi` (WiX v5, Program Files, Start menu, file associations for Settings → Default apps).
- Taking it out of experimental is milestone 34 in `ROADMAP-v3.md`.

## Not implemented
**Open:**
- **Shrink before upscaling** (from 18b): for photos over 4 MP, offer to shrink 2× or 4× first, for a 2× or 1× result. Listed again in `ROADMAP-v4.md`, where it's marked not to do.
- **Convert BSRGAN ourselves** in `lightview-models`, like DarkIR and Real-ESRGAN, instead of relying on the third-party upload.

**Dropped, revisit only if things change:**
- **Diffusion restorers (DiffBIR, HYPIR),** looked at 2026-10-29. Both are XPixelGroup's, on Stable Diffusion 2.1.
  - HYPIR (SIGGRAPH 2025): one pass per tile, a ~1 GB LoRA; its README adds a *Non-Commercial Use Only* declaration from SupPixel (SUPIR's company). As ONNX: UNet with the LoRA merged plus the VAE, fixed prompt so no text encoder; ≈ 1.9 GB fp16 / 3.8 GB fp32.
  - DiffBIR v2.1 (ECCV 2024): Apache-2.0 code, SD 2.1 base (OpenRAIL++); 50 steps per tile plus a SwinIR pre-pass, so a sampler in Rust; ≈ 2.6 GB fp16 / 5 GB fp32.
  - Why not: gigabyte downloads for a lightweight viewer; the fp32 UNet doesn't fit a 4 GB GPU, and fp16 is slow on Pascal; estimated (not measured) on a GTX 1050: HYPIR tens of seconds per 512 px tile, DiffBIR 10+ minutes.
  - Revisit with a modern GPU, or if a small one-step restorer under an open licence appears. Conversions would go in `lightview-models`.

## Non-goals (v2)
- JPEG 2000
- Changing the original file in place
- Layers beyond markup elements
- macOS shortcuts (Command+Z): Ctrl is used
- ~~Smart lasso~~: reopened by Select Object (`ROADMAP-v4.md`, 2026-11-03)
