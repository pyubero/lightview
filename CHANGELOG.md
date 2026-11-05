# Changelog

## 0.5.0 (2026-11-05)

- Image → Resize Optimally shrinks the image to the size its detail actually needs, measured on its sharpest part, so a blurry or upscaled photo stops wasting pixels. It never enlarges, running it again changes nothing, and Ctrl+Z undoes it.
- Nineteen more languages, 28 in all: Japanese, Chinese (Simplified and Traditional), Korean, Ukrainian, Turkish, Indonesian, Vietnamese, Czech, Swedish, Hungarian, Romanian, Greek, Danish, Finnish, Norwegian (Bokmål), Catalan (also for Valencian), Basque and Galician. These are machine translations, so corrections are welcome: see `docs/TRANSLATING.md`. Chinese, Japanese and Korean use your system's CJK font, such as Noto Sans CJK (`fonts-noto-cjk`).
- Adjust Color and Adjust Curves now also apply to the results of Enhance, Denoise, Remove Object, Remove Red Eye and Pixelate Selection; before, a colour change made after them showed nothing and saved nothing.
- The whole-image time shown in the Denoise/Enhance preview is now close to the real time.
- An AI result that arrives after you've changed the edits, or moved to another image, is dropped with a message instead of being pasted in the wrong place.
- A crash inside an AI tool or a save now reports an error instead of leaving the tool disabled or an empty copy behind.
- EXIF copies of PNG files keep their XMP metadata, as JPEG copies do.

## 0.4.0 (2026-11-03)

- Tools → Group Duplicates in Folder… compares every image in the folder and moves each group of look-alikes, the first one included, into a new folder named after the group's first image. A histogram shows how close the images are and suggests a threshold; a Smallest group slider leaves groups under that size where they are, so you can move only the big bursts; the groups update as you move the sliders, before anything moves. A burst of shots stays in one group. Nothing is deleted or stored on disk, the viewer stays in the folder, and Ctrl+Z moves everything back. Subfolders aren't looked at, and rotated copies and heavy crops aren't found.
- A new Local AI menu holds every AI tool, AI Models… and Download AI Runtime…. Select Object moved there from Tools; Read QR Code stays in Tools, since it needs no download.
- Local AI → Blur Background and Replace Background reuse the Remove Background model: the subject stays sharp over a blurred copy of the photo, with a strength slider that starts at 0 % while you watch, or over the background colour from Settings.
- Local AI → Select Object (MobileSAM) turns a box you drag around an object into a selection of its outline, ready for Crop, Remove Object, Delete or Pixelate.
- AMD, Intel and other GPUs (experimental): an optional 6 MB WebGPU download in AI Models… runs the AI tools on any graphics card with Vulkan (Linux) or Direct3D 12 (Windows), falling back to the CPU when it can't.
- Themes: Edit → Settings → Theme picks Dark, Light, High contrast, or follows the system's dark/light setting. Each theme is an XML file you can copy and edit in the themes folder.

## 0.3.0 (2026-11-02)

- Image → Recognize Text reads the text in the selection, or the whole image, offline, and shows it in a dialog where you can edit and copy it. It needs the AI runtime and a text model (PP-OCRv5, Latin or Chinese/Japanese/English), downloaded from Tools → AI Models….
- Tools → Read QR Code shows what a QR code says. Links are never opened, and nothing needs downloading.
- Polish, Dutch and Russian. These are machine translations, so corrections are welcome: see `docs/TRANSLATING.md`.
- Screen readers can read the menus and dialogs.
- Settings → Change Keyboard Shortcuts… gives any menu action a shortcut. A combination that's already taken or reserved is refused, and the action that has it is named.
- Image → Straighten…: turn the image by any angle from −180° to 180° under a grid. The canvas grows to fit the turned image, and the new corners take the background colour, or are transparent if the image has transparency. Crop afterwards if you want straight edges. Ctrl+Z undoes it.
- The crop snaps to 1:1, 4:3, 3:2 or 16:9 (either way round) when it's within 3%, and shows the ratio.
- Image → Greyscale and Invert Colors. Greyscale is Saturation at −1, so it shows in Adjust Color.
- Image → Remove Red Eye and Pixelate Selection work on the selected area. Pixelation uses a randomly placed grid and random noise, so the same picture never pixelates the same way twice. For text, a solid shape is still safer.
- EXIF editing now works on PNG and TIFF files too, multi-page TIFFs and APNGs included, and always saves a copy in the same format. A PNG copy uses the standard `eXIf` chunk. WebP metadata is shown but not edited, because the library lightview uses damages WebP files when it writes them. Every copy is checked against the original's pixels before it's saved.
- File → Copy File puts the image file itself on the clipboard, for pasting into a chat, an email or a file manager. Like a copied image, it stays on the clipboard while lightview runs.
- Camera RAW files (CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2) open instantly from the JPEG preview the camera stores in them, rotated as shot. The inspector shows their EXIF, read only. Save As writes a PNG of the preview.
- Colour management: photos with a colour profile (Display P3 from phones, Adobe RGB from cameras) are converted to sRGB, so they no longer look dull. sRGB images cost nothing extra. This covers JPEG, PNG, WebP and TIFF (every page), animations, JPEG XL, and HEIC and AVIF on Linux, including the coded colours iPhones use. Settings → Colour management turns it off for wide-gamut monitors; Save As then keeps the original profile.
- Drag a file or folder onto the window to open it, on X11 and Windows. Not supported on Wayland yet.
- File → Save Copy (Ctrl+S) writes `photo-edited.jpg` next to the original with no dialog, and never replaces a file.
- The mouse's back and forward buttons go to the previous and next image, and Home and End to the first and last.
- File → Copy Path, Show in File Manager and Rename (F2). Rename never replaces another file.
- The open image reloads when its file changes on disk. If it has unsaved edits, lightview asks first.
- A navigator thumbnail in the corner while zoomed in: click or drag on it to move around.
- Settings → Pixel grid shows the edges of the pixels from 8× zoom up.
- Tools → Copy Color copies the hex value of the pixel you click.
- New formats, view only (Save As offers PNG): JPEG XL, HEIC and AVIF, APNG (animated), QOI, TGA, PNM, DDS, and HDR and EXR (tone-mapped to 8 bits).
- HEIC and AVIF use the system's decoders: libheif and its plugins on Linux, and Microsoft's HEIF and HEVC or AV1 Video Extensions on Windows. A clear message says what's missing. Because those decoders are native code, these files are only decoded when they're shown, never ahead.
- Multi-page TIFFs show every page: Page Down and Page Up, or View → Pages. Thumbnail pages are skipped, and a file with several pages can't be saved yet.

## 0.2.1 (2026-11-01)

- Image → Perspective Correction: drag four corners onto a document, façade or screen shot at an angle, press Enter, and it's straightened and cropped to that shape. Asks first if the image has markup, which is merged into the image. Ctrl+Z undoes it.

Security hardening: opening a malicious file, or browsing past one in a folder, can no longer freeze the app or exhaust memory through lightview's own code.

- SVGs only show embedded images. They no longer read linked files, which on Windows could have sent your login hash to a remote server. Gzip-compressed SVGs are refused.
- Only regular files up to 1 GiB open, so a disguised device file or FIFO can't hang the app.
- Oversized GIF and WebP animations are refused or cut short: very long ones stop at 1 GiB of frames and play what fits.
- EXIF that links to itself is refused instead of freezing the app, and a file that crashes its decoder now shows an error.
- Windows: `turbojpeg.dll` loads only from the app's own folder, never from the folder of the photo being opened.
- Save As erases the old EXIF thumbnail, so cropped or erased content can't be recovered from it. A preview stored in a camera's MakerNote still survives.
- Remove Location also removes XMP segments that contain GPS data.
- Saving keeps the file's permissions and no longer follows a symlink planted at the temporary path.
- File names that aren't valid UTF-8 can now be opened.
- AI Models warns to add models only from sources you trust, and absurd model sizes are refused.
- Release builds use locked dependencies, pinned actions and a `cargo audit` check, and only the publishing job can write to the repository.

## 0.2.0 (2026-10-31)

First public release.

- Opens JPEG, PNG, WebP, GIF, BMP and TIFF, and view-only SVG and ICO, with instant next/previous and animation playback. SVGs stay sharp as you zoom in.
- Zoom, pan, rotate, flip, fullscreen and an inspector sidebar with EXIF, location and colour profile.
- Non-destructive editing: crop, resize, colour adjustments, auto levels, curves in RGB, HSL or HSV, drawing, selections and clipboard. Edits are only written when you save a copy.
- Markup shows at image resolution, as it will be saved.
- EXIF editing for JPEGs, always on a copy of the file.
- Optional local AI tools: Remove Background, Remove Object, Denoise and Enhance Image, with optional NVIDIA GPU support.
- Delete an image to the system Trash, with a confirmation; Ctrl+Z restores it.
- Six languages: English, Spanish, French, German, Italian and Portuguese, and a UI font size setting.
- Experimental Windows builds (`.msi`), alongside the Linux `.deb`.
