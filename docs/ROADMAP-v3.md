# lightview — Roadmap v3

*Closed with 1.0 (2026-11-09): no further features are planned; items still marked “Open” or “Perhaps” here are not planned.*

*Status: proposed 2026-11-01, mostly built for 0.3.0 (2026-11-02; see `CHANGELOG.md`). Each milestone below is split into what's done and what isn't. v1 is in `PROJECT_BRIEF.md`, v2 in `ROADMAP-v2.md`, milestone 32 and later in `ROADMAP-v4.md`.*

## The aim
Close the gaps that make people pick another viewer, without becoming a different kind of program.

- **A viewer with a good toolbox.** Opening and looking at an image stays the main job; every tool is a quick job on one image.
- **Not a gallery manager.** No catalogue, database, albums, tags, ratings or face grouping.
- **Not Photoshop.** No layers beyond markup elements, no RAW development, no painting tools, no generative fill.

## Principles (still hold)
- **Fast.** P1a (window under 200 ms), P2 (next/previous in one frame) and P3 (the UI never waits on decoding).
- **The original is safe.** lightview never changes the image file you opened. Rename, Delete and Group Duplicates are file actions you ask for, and can be undone.
- **Mostly Rust.** Any new C library is loaded at run time and is optional.
- **Private.** No telemetry, no account, no background network use; images never leave the machine.
- **Opening a file does no harm.** Neighbours are decoded unopened, so every decoder runs on files the user never chose: each gets the same limits as `load` and a malicious-file test.
- **No new setting without the user's approval.**

## Where lightview stood (2026-11-01, before v3)
From a competitor search (project pages and reviews; no rival was installed or timed). Formats, colour, viewer basics and reach were the gaps; 0.3.0 closed most of them.

| Area | Position then | Closest rivals |
|---|---|---|
| Local AI tools | Ahead: no rival checked has object removal, denoise, upscaling and low-light together | BetterViewer, Windows Photos |
| Native speed | Possibly ahead, unproven: nobody publishes numbers | Oculante, Lumia, ImageGlass |
| Privacy and safety | Ahead or level | PicView, BetterViewer |
| Edit depth | Level | BetterViewer, gThumb, Gwenview |
| Markup | Behind BetterViewer (no blur, counters or stamps) | BetterViewer, COSMIC Viewer, Gwenview |
| Formats | Behind: no HEIC, AVIF, JPEG XL or RAW | Oculante, nomacs, COSMIC Viewer |
| Colour accuracy | Behind: ICC profiles shown, not applied | avis-imgv, ImageGlass |
| Viewer basics | Behind: no drag and drop, slideshow or print | nearly all |
| Reach | Behind: six languages, `.deb` and `.msi` only, no macOS | BetterViewer |

## Milestones
Numbering continues from v2. Size: **S** days, **M** a week or two, **L** several weeks.

### 23. Everyday viewer basics (M)
**Done:**
- Drag a file or folder onto the window (X11 and Windows).
- File → Save Copy (Ctrl+S); Copy Path, Show in File Manager, Rename (F2).
- Mouse back/forward buttons; Home and End.
- Reload when the file changes on disk, asking first with unsaved edits.
- Navigator thumbnail when zoomed in; pixel grid **[setting]**; Copy Color.

**Not implemented:**
- *Open:* drag and drop on **Wayland**: winit 0.30 delivers no drops there, and a listener of our own failed on KDE (removed). A newer winit should bring it.
- *Dropped:* Slideshow (F5, delay setting).

### 24. Modern formats (L)
**Done** (view only; Save As offers PNG):
- QOI, TGA, PNM, DDS, HDR and EXR (tone-mapped), APNG, every TIFF page.
- JPEG XL with `jxl` (jxl-rs): 0.8 s vs jxl-oxide's 2 s on a 12 MP lossy file.
- HEIC and AVIF: system `libheif.so.1` loaded at run time on Linux (`Recommends:` in the `.deb`), WIC on Windows with no HEVC decoder shipped. Decoded only when shown, never ahead.

**Not implemented:**
- *Open:* the new types are offered under Open With (`lightview.desktop`) but not claimed by set-as-default (`install.sh`, the first-launch question); add them once each has been tried by hand.
- *Open, never evaluated:* an AVIF fallback on Windows without the AV1 extension: `zenavif` (pure Rust, rav1d, version 0.1).

### 25. Colour management (M)
**Done:** ICC → sRGB on decode with `moxcms`, so edits, Copy, AI and Save see sRGB; sRGB and untagged images cost nothing; a Colour management setting (on by default) turns it off, and Save As then embeds the source's profile. The monitor is assumed sRGB.

**Not implemented:**
- *Open:* Windows' WIC path (HEIC/AVIF) doesn't convert colours.
- *Open, not planned:* monitor profiles; for a wide-gamut monitor, turn the setting off.

### 26. Camera RAW previews (M)
**Done:** CR2, CR3, NEF, ARW, DNG, RAF, ORF and RW2 open from their biggest embedded JPEG preview (one format-agnostic scan), rotated as shot; EXIF shown read only.

**Not implemented:**
- *Open, not planned:* a real RAW decode with `rawler` (LGPL-2.1): slower, and RAW development is a non-goal. Sony ARW previews are only ~1616×1080.

### 27. Better saving (M)
**Done:**
- EXIF editing for PNG and TIFF (multi-page and APNG included), always to a copy, checked against the original's pixels.
- File → Copy File puts the file itself on the clipboard.

**Not implemented:**
- *Blocked:* WebP EXIF editing: `little_exif` 0.6.23 corrupts WebP files when writing, so WebP stays read only until it's fixed.
- *Dropped:* lossless JPEG rotate/flip/crop (`tjTransform`); JPEG quality slider and target size (M6's quality 90 stays); Remove all metadata in Save As (Save As keeps GPS); saving animations; Save as AVIF.

### 29. Everyday adjustments (M)
**Done:**
- Straighten (−180°…180° under a grid; the canvas grows and cropping is left to the user, instead of the auto-crop proposed here).
- Crop snaps to 1:1, 4:3, 3:2 or 16:9 within 3%.
- Remove Red Eye; Greyscale and Invert Colors.

**Not implemented:**
- *Dropped:* Temperature, Highlights and Shadows; Levels; Sharpen and Vignette. No presets, filter packs or masks.

### 30. Markup for sharing screenshots (M)
**Done:** Image → Pixelate Selection, on a random grid with noise in each block, with a hint that a solid shape is safer for text.

**Not implemented:**
- *Dropped:* Redact tool (blur, pixelate, black out on a rectangle), numbered counters, spotlight, stamps and emoji, outlined text, saved signature.

### 31. Read what's in the image (L)
**Done:**
- Local AI → Recognize Text: PP-OCRv5 mobile (detector plus Latin, or Chinese/Japanese/English, recogniser) from nakamura196's pinned Apache-2.0 conversion; upright text only; detector capped at 1920 px.
- Tools → Read QR Code with `rqrr`; contents shown, never opened.

**Not implemented:**
- *Dropped:* a Live Text style overlay on the image: a dialog with editable text was chosen instead.
- *Not supported:* tilted and vertical text (straighten first).

### 32. More local AI → `ROADMAP-v4.md`

### 34. Reach (L)
**Done:**
- Polish, Dutch and Russian (machine translations) and `TRANSLATING.md`.
- Screen readers through eframe's `accesskit` (median window 271 vs 273 ms over 12 interleaved runs, +5.6 MB).
- Themes: Dark, Light, High contrast and Follow system, plus user XML files.
- Custom shortcuts for any menu action.

**Not implemented:**
- *Done in 1.0:* **Windows out of experimental:** retested on Windows 11. *Not planned:* signing the `.msi` (SignPath Foundation signs open-source projects for free), winget and Scoop. The suspected `TrashItem.name` extension bug awaits a Windows retest.
- *Open:* **Chinese, Japanese and Korean UI:** Roboto lacks the glyphs; eframe's `system_fonts` feature could supply them.
- *Open, separate decision:* **macOS** needs a Mac, signing and notarisation; Preview is free there.
- *Dropped:* Flathub (the sandbox may hide neighbouring files, and AI downloads and run-time libraries may not work inside it); rpm, AppImage and AUR.

### Dropped milestones
- **22. Prove the speed:** a script timing lightview against rivals, published in the README.
- **28. Print and wallpaper.**
- **33. Compare two images** side by side or with the divider, and a difference view.

## Borderline (gallery-like)
- *Perhaps:* **Filmstrip**: one row of thumbnails for the current folder, hidden by default, in memory only, no disk cache, no grid. Reopens v1's non-goal.
- *Dropped:* batch resize and convert; opening images inside ZIP and CBZ.

## Discarded ideas (2026-11-03)
- An app-level log of the latest messages (up to 5 MB).
- Hover hints on every tool.
- A Ctrl+P quick search for tools.

## Non-goals (v3)
- Catalogues, libraries, databases, albums, tags, ratings, face grouping, search, batch renaming
- Thumbnail grids
- Layers beyond markup elements, brushes, cloning and masks
- RAW development
- Generative fill and diffusion models (see `ROADMAP-v2.md`)
- PDF, and video playback
- HDR display output
- JPEG 2000
- Cloud features, sharing services, reverse image search, automatic update checks
- A plug-in system
- Changing the original image in place
- ~~Duplicate finding~~: reopened with the user's approval as Group Duplicates in Folder (`ROADMAP-v4.md`, 2026-11-03)

## Settled questions
- Windows HEIC: Windows' own codecs (WIC), never a shipped HEVC decoder.
- Colour management: on decode, not at display time.
- Licences checked: PP-OCRv5 conversion (Apache-2.0), `rqrr`; `jxl-oxide` wasn't used.
- Earlier decisions this roadmap could have reopened (JPEG quality 90, GPS kept by Save As, `.deb` and `.msi` only) stay as they were, since those items were dropped. Themes reopened v1's rejected Light theme.

## Open questions
1. **macOS:** port or not? Recommended: not now (no Mac to test on).
2. **Filmstrip:** keep it as a "perhaps", or drop it?
