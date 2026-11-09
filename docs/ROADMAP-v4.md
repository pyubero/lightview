# lightview — Roadmap v4

*Closed with 1.0 (2026-11-09): no further features are planned; items still marked “Open” or “Perhaps” here are not planned.*

*Status: started 2026-11-02. Milestone 32 (more local AI), 35 (Group Duplicates in Folder) and the code rework are done, released in 0.4.0; what's left of each is listed under "Not implemented". `ROADMAP-v3.md`'s aim, principles and non-goals still apply.*

## 32. More local AI (L)
Each model goes through the existing catalog, licence and pin flow; "done" means a real test photo, CPU fallback, and clean cancelling.

**Done (2026-11-03):**
- **AMD and Intel GPUs:** ONNX Runtime's WebGPU plugin (`onnxruntime-ep-webgpu` from PyPI, 6 MB) as a second, experimental download in AI Models… (Vulkan on Linux, D3D12 on Windows; DirectML was skipped as Microsoft steers away from it). On the dev machine it matched the CPU's output, ran 2.5× faster on a GTX 1050 and was slower on an Intel UHD 630. CUDA is preferred when both are installed.
- **Blur and Replace Background** reuse Remove Background's mask, so no new model: blur is a blurred copy under the cut-out, replace an opaque fill in the background colour.
- **Select Object:** MobileSAM (Heliosoph's pinned Apache-2.0 copy of Acly's export) from a dragged box; the largest part's outer outline becomes a lasso selection, for Crop, Remove Object, Delete or Pixelate. Reopened v2's "no smart lasso".
- **A Local AI menu** (File · Edit · View · Image · Local AI · Tools) holds every AI tool, AI Models… and Download AI Runtime…; Read QR Code stays in Tools.

**Not implemented:**
- *Open:* Select Object re-runs the encoder for every box (≈ 2 s on the CPU); keep its output per image if people redo boxes often.
- *Dropped:* **Restore faces:** GFPGAN is Apache-2.0 except code from NVIDIA's StyleGAN2 (non-commercial) and DFDNet (CC BY-NC-SA 4.0); CodeFormer's S-Lab licence is non-commercial too (user's choice, 2026-11-03).
- *Dropped:* auto-find the corners in Correct Perspective; 2× upscaling (shrink first, then upscale); colourising old photos (DDColor; its weights' licence was never checked); Describe Image (a local caption model for the Accessibility description).

## 35. Group Duplicates in Folder (M)
Reopens v3's "duplicate finding" non-goal, with the user's approval. User-facing behaviour and limits are in `FEATURES.md`; the code is `src/dedupe.rs` (hash, group, move, undo, tests) and `src/app/group.rs` (dialog).

**Done (2026-11-03), with the user's choices:**
- Tools → **Group Duplicates in Folder…**: "deduplicate" would suggest deleting, and it only groups. Needs 2+ images; asks first about unsaved edits.
- **Scope:** the folder's images as next/previous lists them; subfolders are ignored, so group folders from an earlier run are never entered (a re-run makes ` (2)` folders, never merges). Everything `load` opens, through `load::small` with the same limits: JPEGs at libjpeg-turbo's 1/8, first frame or page otherwise, RAW previews. **HEIC/AVIF are included** (the user picked the folder), the one exception to "never decoded ahead". Files that fail are "not compared" and never move.
- **pHash:** 32×32 box resize, greyscale, 2-D DCT, the 8×8 lowest frequencies, bits above their median (as `imagehash`). Hamming distance: unrelated photos ≈ 28–36, resized or recompressed copies ≤ 4. Not the EXIF thumbnail, which can be stale or padded. Three hashing threads of its own, so viewing stays responsive.
- **Grouping:** single linkage (union-find): a chain of neighbours within the threshold is one group, so a **burst stays together** even when its first and last shots differ. The cost: a slow series (timelapse, panorama sweep, app screenshots) can merge into one big group; the list shows the biggest groups first.
- **Choosing:** a histogram of each image's nearest-neighbour distance (0–32, plus > 32); a threshold slider 0–20 starting at the suggested value (Otsu's split clamped 2–12, 6 when there's no pile of duplicates), with words for what it means; a **Smallest group** slider (2–50, starts at 2) so a pair can stay put while big bursts move; a live summary and the groups as 96 px thumbnails with each one's distance. Both sliders last only while the dialog is open.
- **Moving:** each group, **first image included**, into `<first image's name without extension>/`, ` (2)`… when taken, trailing dots and spaces trimmed for Windows. Targets checked before `fs::rename`; a file changed since it was compared is skipped and reported; the first error stops the run.
- **The viewer stays in the original folder** on the next image left, as after Delete; Open Recent entries follow the moved files.
- **Undo:** Ctrl+Z (after edit undo, sharing one stack with Trash) moves everything back and removes the folders only if empty; a taken old name is reported. Session only, so no journal on disk.
- **No cache:** hashes and thumbnails live in the dialog and are freed when it closes. A second run starts from zero; only the OS page cache makes it faster, and dropping that would need root (`echo 3 | sudo tee /proc/sys/vm/drop_caches` for benchmarks).
- **Progress:** "Comparing images: n of total", a progress bar, the time left at the pace so far (after 3 images and 1 s), and Cancel, which stops the workers and drops everything; nothing moves before Move.
- **Measured:** 10.9 ms per image on 30 generated 12 MP JPEGs (3 threads), ≈ 11 s per 1 000. Timing test: `LIGHTVIEW_DEDUPE=<dir> cargo test --release dedupe_folder -- --ignored --nocapture`.

**Not implemented:**
- *Open:* time it on a folder of real photos; the aim was under 10 s per 1 000.
- *Open:* a HEIC copy of a JPEG in the end-to-end test, and a check of the dialog by hand.
- *Open:* clicking a thumbnail while the shown image has unsaved edits puts the unsaved-edits question over the dialog; untested.
- *Open:* move sidecar files (`.xmp`, `.aae`) with their image; they stay behind now.
- *Ideas, not decided:* add new look-alikes to the group folders of an earlier run; compare a whole folder tree. Both raise which folder wins, and files moving between subfolders.
- *Not supported (pHash):* rotated or mirrored copies (unless only the EXIF flag differs), crops over ~10–15 %, big edits. Very plain images (blank pages) can group together.

## Rework: keeping the code maintainable (2026-11-02)
A review after milestone 34 found the architecture sound (modules by concern, pure tested cores, decoding off the UI thread, `Doc` as a plain value): **no rewrite needed**. The friction was facts written twice and state reset by hand, nearly all in `src/app*`.

**Done (2026-11-03):**
1. **One enable rule per menu action:** `App::enabled`, used by the menu and by shortcuts; labels written once in `menu::bindable()`. Never rename an `Action` variant: its `Debug` text is the saved shortcut key.
2. **Each tool's state lives in its `Tool` variant**, so leaving a tool drops it.
3. **The image on screen's state is one `Shown` struct** reset with `Shown::default()` (a hand-written reset had already missed `curve_panel`). Reopened 2026-10-30's "one flat `App` struct"; a private sub-struct keeps child modules' access.
4. **One pure function picks draw's starting pixels**, with tests per case; the caches stay as they were.
5. **Splits, code moves only:** `ai.rs` into `src/ai/` (runtime, catalog, infer, ocr, ui), TIFF/EXIF byte helpers to `tiff.rs`, file operations to `files.rs`, Red Eye/Pixelate/Auto Levels to `retouch.rs`, Read QR Code next to OCR, `app.rs`'s tests to `src/app/tests.rs`.

**Not implemented:**
- *Open, only while working nearby:* the confirmations that can never be open together (discard edits, delete, changed on disk, flatten before perspective) could become one `Option<Confirm>` instead of separate flags. Settings, Shortcuts and AI Models can stack, so they keep their own.

**Keep as it is:**
- Domain modules own their `*_ui` panels (`edit::adjust_ui`, `curves::curves_ui`, `ai::models_ui`, `exif_edit::sections_ui`), next to the data they edit.
- No trait for formats or decoders: each format has its own quirk, so a trait would be a layer with one implementer per format.
- `Doc` as a cloneable value with snapshot undo; command-style undo would be more code for no gain at these sizes.
- No workspace or crate split: one binary, one maintainer, and it would widen the `pub` surface.
- The `Msg` channel + `Notify`: `App::handle` is long because it has many cases, not because it's tangled.

**Rules for refactors:** `cargo test` green, `cargo check --target x86_64-pc-windows-gnu` clean, no user-visible change, `--bench` times unchanged over interleaved runs, no new dependencies, locale files untouched unless a string's text changes, and every `tr` string above its file's first `#[cfg(test)]`.

## Earlier decisions reopened (each with the user's approval)
| Decision | Reopened by |
|---|---|
| No smart lasso (v2) | Select Object, milestone 32 |
| No duplicate finding (v3 non-goal) | Group Duplicates in Folder, milestone 35 |
| One flat `App` struct (2026-10-30) | The rework's `Shown` struct |
