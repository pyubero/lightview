# Changelog

## 0.2.1 (2026-11-01)

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
