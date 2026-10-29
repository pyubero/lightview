# lightview — Roadmap v2

*Status: draft, 2026-10-29. Source: `future_draft.md`. v1 (milestones 1–7) is in `PROJECT_BRIEF.md`.*

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

### 8. Menu restructure + View
- Menus follow the draft: File, Edit, View, Tools, Draw, Settings. A menu item appears only once its feature works. Items that don't exist yet are left out, not greyed out.
- View → Previous / Next Image (← / →), Zoom In / Out (Ctrl + / Ctrl −), Actual Size, Fit to Window, Full Screen.
- View → Animation: Play, Pause, Next Frame, Previous Frame (for GIF/WebP).
- Where: `app.rs` (`menu_bar`, `Action`, `SHORTCUTS`).
- **Done when** every menu item works and `cargo test` passes.

### 9. Edit list, undo/redo, Crop, Resize, Export
- An `Edit` enum and a list of edits on the current image. Undo (Ctrl+Z) and Redo (Ctrl+Shift+Z).
- Rotate/flip become entries in the list and replace the view-only `view::Transform` from M4, so they get saved.
- **Crop:** drag a rectangle, then press Enter to apply it.
- **Resize…:** width, height, keep aspect ratio, by percentage, resolution (DPI).
- `save.rs` replays the list on the full decoded image.
- **Export…:** Save As with a format submenu: JPEG, PNG, WebP, BMP, TIFF, GIF.
- Unsaved edits ask for confirmation before you move to another image or close the window.
- **Done when** unit tests replay edit lists on a small image and check the pixels, and saving a copy and opening it again gives the same pixels.

### 10. Colour adjustments
- A panel with Exposure, Contrast, Saturation and Tint sliders, plus Auto Levels and Reset Color Adjustments.
- Edit → Adjust Color… and Tools → Adjust → … all open the same panel. Auto Levels and Reset apply at once, without opening it.
- On screen the adjustment runs on the small resized output (the same trick as M4, so dragging stays cheap). At save time it runs on the full image. The whole panel is one `Edit` entry.
- **Done when** the pixel maths has unit tests, and dragging a slider on a 12 MP photo stays at 60 fps.

### 11. Inspector sidebar
- Opens and closes from View → Show Image Information.
- **General:** file name, format, file size, dimensions, resolution, colour profile.
- **EXIF:** camera and lens, exposure, date taken, GPS. Editable for JPEGs (reuses `exif_edit.rs`).
- **Location:** latitude/longitude, "Open in map" (`xdg-open` with an OpenStreetMap or Google Maps URL), and wipe location.
- **Accessibility:** the image description (EXIF ImageDescription), editable like in M7.
- Saving still always writes a copy, as decided in M7.
- **Done when** the values shown match `exiftool` on the test photos.

### 12. Measure
- Tools → Measure. A temporary overlay: it's never saved and never changes the image.
- Crosshair cursor. Drag from one point to another to draw a line with endpoints. Endpoints can be dragged again. ΔX, ΔY and the distance (`d = √(Δx² + Δy²)`) update live.
- Values are in the original image's pixels at any zoom and rotation (uses the screen-to-image maths in `view.rs`).
- One measurement per image.
- **Done when** a unit test gets the same distance at several zoom levels and rotations.

### 13. Draw / Markup toolbar
- Draw → Show Drawing Toolbar opens the toolbar, in this order: Select, Sketch, Shapes (rectangle, rounded rectangle, ellipse, line, arrow), Text, Highlight, Magnifier, Measure, Image Description, Style, Undo, Redo, Done.
- Elements are vector items in the edit list. They can be selected, moved, resized and edited until you save. Saving flattens them into the image.
- Style controls depend on what's selected (stroke, thickness, fill, opacity, font, magnification, …).
- Keys: Esc cancels the current tool, Delete/Backspace deletes the selected element, double-click edits text.
- Flattening needs a rasterizer: `tiny-skia`, plus `ab_glyph` for text.
- **Done when** a flattened PNG matches the markup on screen, in tests with simple shapes.

### 14. Selection + clipboard
- Tools → Rectangular Selection, Elliptical Selection, Lasso.
- Edit → Select All, Deselect, Copy, Cut, Paste, Delete Selection, Copy Selected Region, Create Image from Selection.
- Cut / Delete fill the selection with transparency, or with the background colour for formats without alpha.
- Clipboard: the `arboard` crate (supports Wayland).
- Paste adds a movable element (reuses M13's element model).
- **Done when** the selection masks have unit tests, and copying into another app (e.g. GIMP or Dolphin) works.

### 15. AI tools (optional `ai` feature)
- Tools → Remove Background, Extract Object, Download Background Removal Model…, Download Object Extraction Model…
- Models go in `~/.local/share/lightview/models`. An item is greyed out while its model is missing.
- Runtime: the `ort` crate (ONNX Runtime). Not in the default build or in `install.sh`.
- **Done when** `cargo tree` shows no ONNX code in the default build, and the `ai` build removes the background from a test photo.

## Non-goals (v2)
- JPEG 2000
- Smart lasso
- Changing the original file in place
- Layers beyond markup elements
- macOS shortcuts (the draft's Command+Z): Ctrl is used

## Open questions
- None right now.
