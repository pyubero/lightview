# lightview — Project Brief

*Status: agreed, 2026-10-28.*

## What it is
A simple, very fast image viewer written fully in Rust. It becomes the default image app on the user's Linux desktop. Windows comes later.

## Goals
1. **Fast.** The first image appears almost at once, and moving through a folder feels instant.
2. **Simple.** It views images. It does only the few light edits listed below.
3. **Fully Rust.** No C libraries for decoding or for the UI.

## Non-goals (v1)
- Thumbnail grid or gallery mode
- Deleting or moving files
- AVIF, HEIC, SVG and camera RAW formats
- Flatpak, Snap, AUR or any other packaging
- Windows builds (the code shouldn't block them, but they aren't tested yet)
- Draw: a separate, later phase

## Target platform
- **Now:** Kubuntu (KDE Plasma, Wayland, plus X11 as a fallback)
- **Later:** Windows

## Tech stack
- **UI:** `egui` through `eframe`. It runs on Linux and Windows, and gives us menus and dialogs.
- **Decoding:** the `image` crate, with only the format features we need (png, jpeg, gif, webp, bmp)
- **Rendering:** eframe's OpenGL backend (`glow`) for all drawing. We don't use wgpu.
- **Image scaling:** `fast_image_resize` on the CPU by default, or the GPU when the setting is on (see Settings)
- **Background work:** decoding happens on a separate thread, so the window never waits on it
- **Saved settings:** eframe's built-in persistence, which keeps the settings and the recent-files list between runs

## Features (v1)
### Opening images
- Open a file passed on the command line: `lightview photo.jpg`
- Open with no argument → an empty window with File → Open
- A `.desktop` file that registers lightview for the supported MIME types, so it can be the default app (set with `xdg-mime default`)
- File → Open uses a file dialog (`rfd`, which uses the system dialog through the XDG portal)
- File → Open Recent

### Navigating
- ← / → go to the previous/next image in the same folder, in the order set in Settings (by file name by default)
- Navigation stops at the first and last image; it doesn't wrap around
- Name order works like Dolphin's file manager: it ignores upper/lower case, and `img2` comes before `img10`
- The images next to the current one are decoded ahead of time, so switching feels instant

### Viewing
- Fit to window is the default. It always keeps the image's proportions, and fills the rest of the viewport with the background colour (black).
- Small images are also enlarged to fit the window
- The window always opens at the default size (1280 × 720). It doesn't remember its last size or position.
- The window never resizes itself when you move to another image
- Zoom in and out with the mouse wheel and Ctrl +/−
- Pan by dragging
- Actual size (1:1)
- Fullscreen with F11 (Esc leaves fullscreen)
- Animated GIF and WebP play
- The title bar shows `filename — W×H — file size`

### Light editing
- Rotate 90° in either direction, and flip horizontally or vertically
- **View only:** rotate and flip never change the file

### Menus (v1, to be refined once the concept works)
- **File:** Open, Open Recent, Close, Info
- **Edit:** Rotate, Flip
- **View:** Zoom In, Zoom Out, Fit to Window, Actual Size, Fullscreen
- **Settings:** opens the Settings dialog

The later phases add File → Save As and Export, and Edit → Draw and Edit EXIF.

### Settings dialog
| Setting | Choices | Default |
|---------|---------|---------|
| GPU image scaling | Off: the CPU resizes the image to its size on screen. On: the full image is uploaded to the GPU once and the GPU scales it, so zooming is smoother. | Off |
| Interpolation | Nearest, Bilinear, Bicubic, Lanczos3. With GPU scaling on, only Nearest and Bilinear are available. | Bilinear |
| Background colour | A colour picker | Black |
| Default window size | Width × height | 1280 × 720 |
| Sort order | By file name, or by date modified | By file name |

## Performance targets (each one gets measured)
| # | Target | How we check it |
|---|--------|-----------------|
| P1a | Window on screen in **under 200 ms** from launch | `--bench` timing from process start to the first frame |
| P1b | A 12 MP JPEG on screen **as fast as decoding allows**. A preview shows first when the photo has one (see milestone 2), and the sharp image replaces it once decoding finishes. | `--bench` breakdown: decode / resize / first image frame |
| P2 | Next/previous image **in under 16 ms** (one frame) when the image was already decoded ahead | The same logging |
| P3 | The UI never freezes during decoding | Decoding never runs on the UI thread |

Binary size and memory aren't goals for now; we'll optimize them if they become a problem.

## Milestones
1. **Spike** ✅ (2026-10-28): an eframe window that shows one JPEG passed on the command line. Result: eframe takes 115–200 ms to the first frame and a 12 MP JPEG takes ~360–410 ms to decode, so the image appears at ~420 ms. The original 100 ms target was split into P1a and P1b.
2. **Viewer** ✅ (2026-10-28; checked by hand by the user). Measured with `--bench` on a 4000×2252 Samsung photo: window 121 ms (P1a ✅), preview 124 ms, full image 243 ms (medians of 10). Also applies EXIF orientation. Menus move to milestone 3. The plan was to: show the small preview built into the JPEG's EXIF data straight away, then replace it with the sharp image. Also navigating, zoom and pan, fit and 1:1, fullscreen, the title bar, animation.
3. **Integration** ✅ (2026-10-28; checked by the user). Menu bar (File: Open, Open Recent, Close, Info; View: zoom/fit/actual size/fullscreen), hidden in fullscreen. `lightview.desktop` and `install.sh` make it the default viewer. Recent = files opened explicitly, last 10, saved between runs.
4. **Light editing** ✅ (2026-10-28; checked by the user). Edit menu: Rotate Left (Ctrl+L), Rotate Right (Ctrl+R), Flip Horizontally, Flip Vertically. View only; resets when you switch images.
5. **Settings dialog** ✅ (2026-10-28; checked by the user). Settings button in the menu bar; every setting in the table above; saved between runs. Changes apply right away, except the window size, which applies at the next start.
6. **Save As** ✅ (2026-10-28). One File → Save As… (Ctrl+Shift+S), no separate Export. The format follows the extension (JPEG at quality 90, PNG, lossless WebP, BMP). EXIF is kept, with the orientation reset and the old thumbnail unlinked. Overwriting needs the system dialog's confirmation. Writes through a temporary file. Animated images can't be saved yet.
7. **EXIF editing** ✅ (2026-10-29). Tools → Edit EXIF… (JPEG only). Editable: Description, Artist, Copyright, Date taken, camera Make and Model; the location can be removed. All other tags are shown read-only. Always saved to a copy (the original is never modified). Only the EXIF block is rewritten, never the pixels. MakerNote is dropped.
8. *(Later)* Draw.

## Decided
- **GPU checkbox (Q1):** it's called *GPU image scaling* and controls only how the image gets scaled. The UI always draws with OpenGL.
- **Startup (Q2):** milestone 1 missed 100 ms because decoding is the bottleneck, not egui. We keep egui and split P1 into P1a (window) and P1b (image). The EXIF preview comes in milestone 2.
- **Editing (Q3):** v1 is view only. Saving comes later, and Draw and EXIF editing after that.
- **Interpolation (Q4):** the four common modes listed in the Settings table.
- **Preferences (Q5):** the settings in the Settings table. Wrapping, zoom mode, enlarging small images and window size are fixed behaviours, not settings.

## Open questions
- None right now. Any new setting needs the user's approval before it's added.
