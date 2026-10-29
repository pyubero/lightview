# lightview — Roadmap v2

*Status: built, 2026-10-29 (milestones 8–15 done; checked by unit tests, not yet by hand). Source: `future_draft.md`. v1 (milestones 1–7) is in `PROJECT_BRIEF.md`.*

## What still holds
- **Fast.** P1a (window under 200 ms), P2 (next/previous in one frame) and P3 (the UI never waits on decoding) still apply. New features must not slow down startup or navigation.
- **The original is safe.** Lightview never changes the file you opened. Edits go to a copy through Save As or Export.
- **Mostly Rust.** Any new C library is loaded at run time and is optional, like libjpeg-turbo.
- **No new setting without the user's approval.**

## Decided (2026-10-29)
- **Edit list.** Edits (crop, resize, colour, rotate/flip, markup) are stored as a list on the current image, with undo/redo. They show on screen right away and are applied to the full image only when you save a copy.
- **AI tools** come last, behind an optional cargo feature (`ai`). The default build doesn't include them.
- **No JPEG 2000.** There's no pure-Rust encoder for it. TIFF gets added (it's a feature of the `image` crate).
- **Selection tools:** rectangle, ellipse and lasso. No smart lasso.
- The **inspector sidebar** replaces the Info dialog and takes in the Edit EXIF dialog.
- **Rotate/flip** become saved edits (this replaces M4's view-only rule).
- **GIF export** is added.
- **Unsaved edits:** lightview asks before switching image or closing.
- **Paste** adds a movable element, so Selection comes after Draw.
- **New dependencies approved:** `arboard`, `tiny-skia`, `ab_glyph`, `ort`.

## Milestones
Numbering continues from v1.

### 8. Menu restructure + View ✅ (2026-10-29)
- Menus follow the draft: File, Edit, View, Tools, Draw, Settings. A menu item appears only once its feature works. Items that don't exist yet are left out, not greyed out.
- View → Previous / Next Image (← / →), Zoom In / Out (Ctrl + / Ctrl −), Actual Size, Fit to Window, Full Screen.
- View → Animation: Play, Pause, Next Frame, Previous Frame (for GIF/WebP).
- Where: `app.rs` (`menu_bar`, `Action`, `SHORTCUTS`).
- **Done when** every menu item works and `cargo test` passes.

### 9. Edit list, undo/redo, Crop, Resize, Export ✅ (2026-10-29)
- An `Edit` enum and a list of edits on the current image. Undo (Ctrl+Z) and Redo (Ctrl+Shift+Z).
- Rotate/flip become entries in the list and replace the view-only `view::Transform` from M4, so they get saved.
- **Crop:** drag a rectangle, then press Enter to apply it.
- **Resize…:** width, height, keep aspect ratio, by percentage, resolution (DPI).
- `save.rs` replays the list on the full decoded image.
- **Export…:** Save As with a format submenu: JPEG, PNG, WebP, BMP, TIFF, GIF.
- Unsaved edits ask for confirmation before you move to another image or close the window.
- **Done when** unit tests replay edit lists on a small image and check the pixels, and saving a copy and opening it again gives the same pixels.

### 10. Colour adjustments ✅ (2026-10-29)
- A panel with Exposure, Contrast, Saturation and Tint sliders, plus Auto Levels and Reset Color Adjustments.
- Edit → Adjust Color… and Tools → Adjust → … all open the same panel. Auto Levels and Reset apply at once, without opening it.
- On screen the adjustment runs on the small resized output (the same trick as M4, so dragging stays cheap). At save time it runs on the full image. The whole panel is one `Edit` entry.
- **Done when** the pixel maths has unit tests, and dragging a slider on a 12 MP photo stays at 60 fps.

### 11. Inspector sidebar ✅ (2026-10-29)
- Opens and closes from View → Show Image Information.
- **General:** file name, format, file size, dimensions, resolution, colour profile.
- **EXIF:** camera and lens, exposure, date taken, GPS. Editable for JPEGs (reuses `exif_edit.rs`).
- **Location:** latitude/longitude, "Open in map" (`xdg-open` with an OpenStreetMap or Google Maps URL), and wipe location.
- **Accessibility:** the image description (EXIF ImageDescription), editable like in M7.
- Saving still always writes a copy, as decided in M7.
- **Done when** the values shown match `exiftool` on the test photos.

### 12. Measure ✅ (2026-10-29)
- Tools → Measure. A temporary overlay: it's never saved and never changes the image.
- Crosshair cursor. Drag from one point to another to draw a line with endpoints. Endpoints can be dragged again. ΔX, ΔY and the distance (`d = √(Δx² + Δy²)`) update live.
- Values are in the original image's pixels at any zoom and rotation (uses the screen-to-image maths in `view.rs`).
- One measurement per image.
- **Done when** a unit test gets the same distance at several zoom levels and rotations.

### 13. Draw / Markup toolbar ✅ (2026-10-29)
- Draw → Show Drawing Toolbar opens the toolbar, in this order: Select, Sketch, Shapes (rectangle, rounded rectangle, ellipse, line, arrow), Text, Highlight, Magnifier, Measure, Image Description, Style, Undo, Redo, Done.
- Elements are vector items in the edit list. They can be selected, moved, resized and edited until you save. Saving flattens them into the image.
- Style controls depend on what's selected (stroke, thickness, fill, opacity, font, magnification, …).
- Keys: Esc cancels the current tool, Delete/Backspace deletes the selected element, double-click edits text.
- Flattening needs a rasterizer: `tiny-skia`, plus `ab_glyph` for text.
- **Done when** a flattened PNG matches the markup on screen, in tests with simple shapes.

### 14. Selection + clipboard ✅ (2026-10-29)
- Tools → Rectangular Selection, Elliptical Selection, Lasso.
- Edit → Select All, Deselect, Copy, Cut, Paste, Delete Selection, Copy Selected Region, Create Image from Selection.
- Cut / Delete fill the selection with transparency, or with the background colour for formats without alpha.
- Clipboard: the `arboard` crate (supports Wayland).
- Paste adds a movable element (reuses M13's element model).
- **Done when** the selection masks have unit tests, and copying into another app (e.g. GIMP or Dolphin) works.

### 15. AI tools (optional `ai` feature) ✅ (2026-10-29)
- Tools → Remove Background, Extract Object, Download Background Removal Model…, Download Object Extraction Model…
- Models go in `~/.local/share/lightview/models`. An item is greyed out while its model is missing.
- Runtime: the `ort` crate (ONNX Runtime). Not in the default build or in `install.sh`.
- **Done when** `cargo tree` shows no ONNX code in the default build, and the `ai` build removes the background from a test photo.

### 16. AI runtime on request ✅ (2026-10-29)
- One binary for everyone: the `ai` cargo feature is gone. `ort` loads ONNX Runtime at run time (load-dynamic), the first time an AI tool runs, never at startup.
- Tools → Download AI Runtime… fetches Microsoft's ONNX Runtime 1.31.1 for Linux x64 (11 MB) and installs only `libonnxruntime.so.1.31.1` in `~/.local/share/lightview/lib`. Remove Background / Extract Object stay greyed out until the runtime and their model are installed.
- Every download (runtime and models) is checked against a SHA-256 pinned in the code; a mismatch is deleted.
- Linux only for now (system `curl`, `sha256sum`, `tar`).
- **Done when** the lean binary links no ONNX Runtime (`ldd`), the runtime installs through the app's own code, and background removal gives the same result as the built-in version.

### 17. Choose and add AI models ✅ (2026-10-29)
- Tools → Remove Background and Remove Background and Crop (crops to the subject) use the same model.
- Tools → AI Models… lists the `.onnx` files in `~/.local/share/lightview/models` and picks that model from a drop-down (saved in Settings). Without a choice: the first installed catalog model, else any installed one.
- Download from a catalog of rembg models, each pinned by SHA-256: Silueta, U²-Net small, U²-Net, U²-Net people, IS-Net, IS-Net anime, BiRefNet lite.
- Add model file… copies your own `.onnx` in (never overwrites); Open models folder.
- Every download (models and the AI runtime) first shows its upstream licence in full (embedded from `licenses/`) and starts only after Accept and Download.
- Each model's preprocessing (input size, mean, std, sigmoid) can be edited and is saved per file. Unknown models read their input size from the model and use the usual normalisation; Detect size re-reads it.
- **Done when** every catalog model and a renamed (unknown) model run on a test photo, and the unknown one gives the same result as its catalog twin.

### 18. Denoise and Enhance Image ✅ (2026-10-29)
- Tools → **Denoise** (faithful: removes noise and JPEG artifacts) and **Enhance Image** (generative: sharper, may invent detail). No settings.
- Models: SCUNet real-PSNR for Denoise (default; SCUNet σ15/25/50 and SwinIR σ25 as alternatives) and SCUNet real-GAN for Enhance. Apache-2.0 (SCUNet, SwinIR); ONNX conversions by Heliosoph LLC on Hugging Face, pinned to a revision and SHA-256, licence files of the conversions shown too. SUPIR (non-commercial licence, multi-GB, GPU) and BSRGAN (4× upscaler only) were dropped.
- CPU speed (i7-8750H, turbo off): SCUNet ≈ 57 s per megapixel, SwinIR ≈ 4× slower. So each tool first previews the visible area (≤ 256 × 256, one tile, ≈ 5 s with SCUNet) as a before/after with a draggable line; **Apply to Whole Image** (with a time estimate) or **Apply to Selection** then runs in the background with a progress bar, time left and Cancel. The result is one undoable edit.
- The first Enhance shows a notice that it may alter fine details.
- Tools → AI Models… is grouped by task (Remove Background, Denoise, Enhance Image), each with its own model drop-down and its downloads sorted by size.
- **Done when** tiling reproduces an identity model exactly, a two-file download is all-or-nothing, and the real models clean a noisy test crop without visible tile seams.

### 19. AI on the GPU (NVIDIA) ✅ (2026-10-29)
- Optional **Download GPU Support** in Tools → AI Models… (only shown with an NVIDIA driver): ONNX Runtime 1.31.1's CUDA 12 build from Microsoft plus cudart, cuBLAS, cuRAND and cuDNN from NVIDIA's official PyPI packages — ~1.5 GB download, 2.1 GB in `~/.local/share/lightview/gpu`, every file pinned by SHA-256, all three licences (ONNX Runtime MIT, NVIDIA CUDA EULA, NVIDIA cuDNN SLA) shown one after the other first. CUDA 12, because CUDA 13 no longer supports Pascal (GTX 10xx).
- "Use the GPU" switch (on by default). If the GPU can't be used, it falls back to the CPU and the dialog says why. Installing it needs a restart (ONNX Runtime is loaded once).
- Measured on a GTX 1050 (4 GB): Denoise/Enhance ≈ 5.8 s/MP vs ≈ 57 s/MP on the CPU (≈ 10×; a 9 MP photo in ~70 s instead of ~12 min, peak 3.1 GB graphics memory); same output as the CPU (max 1/255 difference).
- **Done when** it installs through the app's own code, runs Denoise on the GPU with the same result as the CPU, and falls back cleanly with no usable GPU.

### 18b. Enhance Image = BSRGAN 4× ✅ (2026-10-29)
- Enhance now upscales 4× with BSRGAN (cszn/BSRGAN, Apache-2.0), using the half-precision ONNX export `FuryTMP/BSRGANx4_fp16` from Hugging Face (33 MB, pinned by revision and SHA-256; its origin isn't verified). SCUNet real-GAN is gone from the catalog.
- Same flow as Denoise: a 256 px preview (a 1024 px result), then Apply to Whole Image, which makes the image 4× bigger as one undoable edit. Apply to Selection isn't offered for upscalers.
- Tiles are 256 px for upscalers (a 512 px tile needs a 2 GB buffer, too much for a 4 GB GPU), and tile outputs are written straight into the result instead of blended (a 4× result of a 12 MP photo is already ~770 MB).
- Measured: ~1.6 s per 256 px tile on a GTX 1050, ~21 s on the CPU; a 12 MP photo is ~250 tiles (≈ 7 min GPU, > 1 h CPU). A pasted image is shown at most 4096 px across on screen.
- Later: for photos over 4 MP, offer to shrink 2× or 4× first, for a 2× or 1× result.

## Future ideas
- Convert the best FOSS restoration and upscaling models (BSRGAN, Real-ESRGAN, …) to ONNX ourselves, with documented scripts, and publish them from a separate repo, instead of relying on third-party uploads.

## Non-goals (v2)
- JPEG 2000
- Smart lasso
- Changing the original file in place
- Layers beyond markup elements
- macOS shortcuts (the draft's Command+Z): Ctrl is used

## Open questions
- None right now.
