# lightview — Roadmap v3

*Status: proposal, 2026-11-01. Nothing here is decided or built. v1 is in `PROJECT_BRIEF.md`, v2 in `ROADMAP-v2.md`.*

## The aim
Close the gaps that make people pick another viewer, without becoming a different kind of program.

- **It's a viewer with a good toolbox.** Opening and looking at an image stays the main job. Every tool is for a quick job on one image.
- **It isn't a gallery manager.** No catalogue, database, albums, tags, ratings or face grouping.
- **It isn't Photoshop.** No layers beyond markup elements, no RAW development, no painting tools, no generative fill.

## What still holds
- **Fast.** P1a (window under 200 ms), P2 (next/previous in one frame) and P3 (the UI never waits on decoding) still apply.
- **The original is safe.** Lightview never changes the image file you opened. Rename and Delete are file actions you ask for, and Delete can be undone.
- **Mostly Rust.** Any new C library is loaded at run time and is optional, like libjpeg-turbo.
- **Private.** No telemetry, no account, no background network use, and images never leave the machine.
- **Opening a file does no harm.** Neighbours are decoded unopened, so every new decoder runs on files the user never chose. Each one gets the same limits as `load` and a malicious-file test.
- **No new setting without the user's approval.** Items that need one are marked **[setting]**.

## Where lightview stands
Based on the competitor search of 2026-11-01 (project pages and reviews; no rival was installed or timed).

| Area | Position | Closest rivals |
|---|---|---|
| Local AI tools | **Ahead.** No rival checked has object removal, denoise, upscaling and low-light together in one viewer. | BetterViewer (background removal, OCR); Windows Photos (closed) |
| Native speed | **Possibly ahead, but unproven.** Rivals give no numbers, and neither does lightview's README. | Oculante, Lumia, ImageGlass |
| Privacy and safety | **Ahead or level.** Few rivals state it this clearly. | PicView, BetterViewer |
| Edit depth | **Level.** Curves and perspective match BetterViewer; a few everyday adjustments are missing. | BetterViewer, gThumb, Gwenview |
| Markup | **Behind BetterViewer** (no blur, counters or stamps). | BetterViewer, COSMIC Viewer, Gwenview |
| Formats | **Behind.** No HEIC, AVIF, JPEG XL or RAW. This is the biggest gap. | Oculante, nomacs, COSMIC Viewer |
| Colour accuracy | **Behind.** ICC profiles are shown in the inspector but not applied. | avis-imgv, ImageGlass |
| Viewer basics | **Behind.** No drag and drop, slideshow or print. | Nearly all of them |
| Reach | **Behind.** Six languages, `.deb` and `.msi` only, no macOS. | BetterViewer (16 languages, rpm, AppImage, macOS) |

## Milestones
Numbering continues from v2, in the suggested order: the gaps people hit in their first minutes come first. Size is a rough guide: **S** days, **M** a week or two, **L** several weeks.

### DONT IMPLEMENT: 22. Prove the speed (S)
The cheapest way to stand out: no rival checked publishes measured times.
- DONT IMPLEMENT: A script runs `--bench` on lightview and times rivals (Gwenview, qimgv, Oculante, nomacs, BetterViewer) on the same files, interleaved, on one machine.
- DONT IMPLEMENT: Publish the method and the results in the README: cold start, first image, next image.
- DONT IMPLEMENT: **Done when** anyone can rerun the script and get the same ranking.

### 23. Everyday viewer basics (M)
- Drag a file or folder onto the window to open it (qimgv). Works on X11 and Windows. **Not supported on Wayland yet:** winit 0.30 doesn't deliver drops there.
- File → Save copy (Ctrl + S).
- Mouse back and forward buttons go to the previous and next image (BetterViewer).
- Home and End go to the first and last image.
- DONT IMPLEMENT: Slideshow, with F5 to start, Esc to stop and a delay **[setting]** (ImageGlass).
- Copy Path, Show in File Manager, and Rename with F2 trhough a small dialog (PicView, COSMIC Viewer).
- Reload when the open file changes on disk, unless it has unsaved edits after notifying the user.
- A navigator thumbnail in the corner when zoomed in (BetterViewer). 
- A pixel grid at high zoom (a checkbox in settings) **[setting]**
- A tool "Copy color" where you copy the color hex value when clicking any pixel with (Oculante, ImageGlass).
- **Done when** each has a test where it can be driven headlessly, and P1a and P2 are unchanged.

### 24. Modern formats (L)
All view only at first, with the same zoom, edit and Save As path as other formats.
- **Cheap wins:** QOI, TGA, PNM, DDS, HDR and EXR (`image` crate features, HDR and EXR tone-mapped to 8 bits), APNG playback (`PngDecoder::apng`), and every page of a TIFF (nomacs).
- **JPEG XL:** pure Rust, so it works on both systems. `jxl-oxide` (0.12.6, has `image` integration) is the likely pick. `jxl-rs` (BSD-3-Clause) says Chrome and Firefox use it, but its GitHub page lists no releases. Choose by measured decode time and licence.
- **HEIC and AVIF on Linux:** load the system's `libheif.so.1` at run time, the same way as `libturbojpeg.so.0`. One loader covers both formats and adds no build dependency. The `.deb` gets a `Recommends:` line. `image`'s `avif-native` is ruled out because it builds against dav1d, a C library.
- **HEIC and AVIF on Windows:** check whether Windows' own codecs can be used through WIC. Don't ship an HEVC decoder in the `.msi`: libde265 is LGPL, but HEVC carries patent concerns, and Mageia keeps it in its tainted repository. For AVIF, `zenavif` (pure Rust, based on rav1d, version 0.1) is a fallback to evaluate.
- `xdg-mime` and the `.msi` register the new types. The new formats join set-as-default only after they've been tried by hand.
- **Done when** a test file of each format decodes, malicious-file tests pass with the `load` limits, and a missing `libheif` gives a clear message instead of a failure.

### 25. Colour management (M)
- Convert images with an ICC profile to sRGB for display, using `moxcms` (pure Rust, BSD-3-Clause or Apache-2.0). Without this, wide-gamut photos look dull, and iPhone HEICs are the common case, so this should land with milestone 24.
- Skip the conversion when an image has no profile or is already sRGB, so most JPEGs cost nothing.
- Assume an sRGB monitor. Let this be a checkbox in settings, default: ON.
- To settle first: convert on decode (simple, but changes what edits and Save see) or at display time (exact, but touches both the CPU and GPU paths). Either way, a saved file must keep or embed a correct profile.
- **Done when** a Display P3 and an Adobe RGB test image match a reference render, and `--bench` shows no slowdown on sRGB JPEGs.

### 26. Camera RAW previews (M)
- Show the JPEG preview embedded in RAW files (CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2). This gives instant display with no demosaicing, which is what a viewer needs. Reuse the approach of the bounds-checked EXIF reader.
- View only, and the inspector shows the EXIF as usual.
- A real RAW decode with `rawler` (LGPL-2.1, so compatible) is possible later, but it's slower, and RAW development is a non-goal.
- **Done when** sample files from each maker open at preview size, with EXIF orientation applied.

### 27. Better saving (M)
- DONT IMPLEMENT: **Lossless JPEG rotate, flip and crop** when those are the only edits, using `tjTransform` from the libjpeg-turbo already loaded at run time (Oculante, FastStone). Saves to a copy, like everything else.
- DONT IMPLEMENT: **JPEG quality** slider with an estimated file size, plus "Reduce to a target size" for email and web forms (PicView, Lumia). **[setting]** Reopens M6's fixed quality of 90.
- DONT IMPLEMENT: **Remove all metadata** option in Save As. This supports the privacy promise. Reopens M6, which keeps GPS when saving.
- **EXIF editing beyond JPEG:** PNG, WebP and TIFF.
- DONT IMPLEMENT: **Save animations:** crop, resize, rotate and colour apply to every frame, then write GIF or WebP (BetterViewer). Lifts the "animated images can't be saved" limit.
- **Copy File** to the clipboard, as a file for pasting into chats and file managers, not as pixels.
- DONT IMPLEMENT: **Save as AVIF:** the `image` crate's `avif` feature (rav1e) is pure Rust. Check its binary size and speed first. Lossy WebP is left out: the pure-Rust encoder is lossless only, so it would need libwebp.
- **Done when** a lossless rotate leaves JPEG pixel data bit-identical to a reference, and each option has a save-and-reopen test.

### DONT IMPLEMENT: 28. Print and wallpaper (M)
- DONT IMPLEMENT: File → Print, with fit to page or actual size and a preview (gThumb, and every system viewer). Use the system's print portal on Linux and the shell on Windows, so no print dialog is built by hand.
- DONT IMPLEMENT: Set as Wallpaper (COSMIC Viewer), through the portal on Linux and the system call on Windows.
- DONT IMPLEMENT: **Done when** both work on KDE and Windows 11. Both need checking by hand.

### 29. Everyday adjustments (M)
- **Straighten:** rotate by any angle, with a grid, then auto-crop (BetterViewer, gThumb).
- **Crop snaps:** quicksnaps to 1:1, 4:3, 3:2, 16:9 whenever within a 3% margin.
- DONT IMPLEMENT: Temperature, Highlights and Shadows in Adjust Color (Preview, BetterViewer).
- DONT IMPLEMENTLevels, with black, mid and white points on the histogram that Curves already draws (Preview, BetterViewer).
- DONT IMPLEMENTSharpen and Vignette (BetterViewer).
- Red-eye removal (Gwenview, gThumb).
- Greyscale and Invert colors.
- Keep it to these. No presets gallery, no filter packs, no masks.
- **Watch out:** colour runs on the cached resized pixels, and sharpening doesn't scale, so view and saved file can differ. It may need to use `edit::flat` once a change ends, as markup does (up to 16 MP).
- **Done when** the pixel maths has unit tests, sliders stay at 60 fps on a 12 MP photo, and the saved file matches the view.


### 30. Markup for sharing screenshots (M)
- DONT IMPLEMENT: **Redact:** Blur, Pixelate and solid Black Out on a rectangle (BetterViewer). Black Out is the default for sensitive text, because pixelated text can sometimes be recovered. The UI should say so.
- DONT IMPLEMENT: Numbered counters that go up by one with each click (BetterViewer).
- DONT IMPLEMENT: Spotlight: darken everything except one area (BetterViewer).
- DONT IMPLEMENT: Stamps and emoji, and text with an outline or background (Gwenview, BetterViewer).
- DONT IMPLEMENT: Saved signature, drawn once and reused (Preview).
- DONT IMPLEMENT: **Done when** a flattened PNG matches the screen in tests, and redacted pixels can't be recovered from the saved file, including its EXIF thumbnail.
- Add a tool to "Pixelate selection" (for example for faces), use a non obvious randomizer algorithm so that they cannot be recovered.









### 31. Read what's in the image (L)
- **Text recognition:** new AI tool: select and copy text from an image, offline (BetterViewer, Preview). Use PaddleOCR's PP-OCRv5 as ONNX on the existing AI runtime, with the same licence prompt and SHA-256 pin. The upstream model licence needs checking: only a third-party repack listing Apache-2.0 was found. It needs a detection model plus one recognition model per script.
- **QR codes:** detect and offer to copy the contents, never to open them automatically (BetterViewer). `rqrr` is pure Rust; check its licence and maintenance. No model download.
- **Done when** printed text in the six UI languages is recognised on test images, and a QR code decodes offline.


### 32. More local AI — postponed to `ROADMAP-v4.md` (2026-11-02)

### DONT IMPLEMENT: 33. Compare two images (M)
- DONT IMPLEMENT: Two images side by side or with the before/after divider, with zoom and pan linked (Oculante). The draggable divider already exists for AI previews.
- DONT IMPLEMENT: An optional difference view.
- **Done when** two different-sized images stay aligned at any zoom.

### 34. Reach (L)
Each item stands alone and can be done at any time.
- **Windows out of experimental:** finish retesting, and sign the `.msi`. SignPath Foundation offers free signing for open-source projects. Add winget and Scoop (PicView).
- DONT IMPLEMENT: **Flathub:** where many Linux users look first. Rules forbid network access at build time and require minimal permissions. Unknowns to test: the sandbox may hide neighbouring files unless folder access is requested, which next/previous depends on, and AI runtime downloads and `libheif`/`libturbojpeg` loading may not work the same way inside it.
- DONT IMPLEMENT: **More packages:** rpm, AppImage and AUR. Reopens "`.deb` and `.msi` only".
- **More languages:** Polish, Dutch and Russian are the easy ones. Chinese, Japanese and Korean need fonts Roboto lacks, and eframe's `system_fonts` feature could supply them. A translation guide so that volunteers can add `locales/*.tsv` files.
- **Screen readers:** eframe's `accesskit` feature. Measure its startup cost first.
- **Themes:** Dark, Light, High contrast and follow the system, plus user XML themes **[setting]** (BetterViewer).
- **Custom shortcuts** **[setting]** (PicView, ImageGlass).
- **macOS:** every leading cross-platform rival has it, but it needs a Mac, plus signing and notarisation. A separate decision.
- **Done when** each listed item installs and passes a by-hand check on a clean system.


## Borderline
Competitors have these, but they lean towards gallery management. Each is listed with its limit; drop it if that limit doesn't feel right.

- PERHAPS: **Filmstrip** (COSMIC Viewer, PicView): a single row of thumbnails for the current folder, hidden by default, kept in memory only. No thumbnail cache on disk and no grid. Reopens v1's non-goal.
- DONT IMPLEMENT: **Batch resize and convert** (PicView, RustyViewer): a dialog applying one size and one format to a chosen set of files, always writing copies. No renaming and no edit recipes.
- DONT IMPLEMENT: **Open images inside ZIP and CBZ** (nomacs, PicView): read-only and browsed like a folder. This drifts towards being a comic reader, so it's the first one to drop.

## Non-goals (v3)
- Catalogues, libraries, databases, albums, tags, ratings, face grouping, search, duplicate finding, batch renaming
- Thumbnail grids
- Layers beyond markup elements, brushes, cloning and masks
- RAW development
- Generative fill and diffusion models (see Future ideas in `ROADMAP-v2.md`)
- PDF, and video playback
- HDR display output
- JPEG 2000
- Cloud features, sharing services, reverse image search, automatic update checks
- A plug-in system
- Changing the original image in place

## Earlier decisions this would reopen
None of these should change without the user's approval.

| Decision | Reopened by |
|---|---|
| JPEG quality fixed at 90 (M6) | Milestone 27 |
| Save As keeps GPS (M6, 2026-11-01 audit) | Milestone 27, as an opt-in |
| `.deb` and `.msi` only, no tarball (2026-10-31) | Milestone 34 |
| No thumbnail grid or gallery (v1) | Filmstrip, if kept |

## Discarded ideas (2026-11-03)
Raised after milestone 34 and set aside by the user, not planned:

- DONT IMPLEMENT: An app-level log that keeps the latest messages, up to 5 MB.
- DONT IMPLEMENT: Hover hints (tooltips) on every tool.
- DONT IMPLEMENT: A Ctrl+P quick search for tools.

## Open questions
Recommendations first; the answer is the user's in each case.

1. **Order.** Start with 22–25 (speed proof, basics, formats, colour)? Recommended: these remove the most reasons to reject lightview.
2. **Windows HEIC.** Rely on Windows' own codecs? Recommended: yes, if WIC can do it, and never ship an HEVC decoder.
3. **Colour management.** Decode time or display time? Recommended: settle with a short spike, because it decides what saved files contain.
4. **macOS.** Port or not? Recommended: not in v3, because there's no Mac to test on and Preview is already free there.
5. **Borderline items.** Keep, trim or drop? Recommended: keep the filmstrip and batch convert with their limits, drop archives.
6. **Licences to verify before adding:** PP-OCRv5 models, `jxl-oxide`, `rqrr`. Milestone 32's are in `ROADMAP-v4.md`.
