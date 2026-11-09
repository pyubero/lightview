# lightview features

Everything lightview can do, by area. [`MENUS.md`](MENUS.md) lists every menu item and its
keyboard shortcut.

This list is complete as of 1.0: no new features are planned, only bug fixes and translations.

## Viewing

- **Opens quickly.** A small preview appears first (the photo's embedded thumbnail, then a
  reduced-size decode) while the full image is decoded in the background, so the window never
  freezes.
- **Zoom and pan.** Fit to window, actual size (1:1 pixels), zoom at the pointer with the mouse
  wheel or a pinch, and pan by dragging. Right-drag pans with any tool selected.
- **Navigator.** While zoomed in, a thumbnail in the corner shows which part of the image is on
  screen; click or drag on it to move there.
- **Full screen** with F11; the menus are hidden there, and Esc leaves it.
- **Rotated as shot.** Phone and camera photos are turned the way the camera held them (EXIF
  orientation).
- **Colour management.** Photos with a colour profile, such as Display P3 or Adobe RGB, are shown
  in sRGB, so wide-gamut colours look right. It can be turned off in Settings.
- **Camera RAW files** open at once from the preview the camera stores in them.
- **Animations.** Animated GIF, WebP and APNG play, and can be paused and stepped frame by frame.
  Multi-page TIFFs show every page.
- **Sharp SVGs.** Vector images are redrawn at the zoom level, so they stay sharp as you zoom in.
- **Detail work.** A pixel grid shows the edges of the pixels when zoomed far in, and the Copy
  Color tool copies a pixel's colour as `#RRGGBB`.
- **Live reload.** When the open file changes on disk, it reloads at the same zoom. If you have
  unsaved edits, lightview asks whether to reload or keep them.

## Browsing a folder

- **Next and previous without waiting.** The neighbouring images are decoded ahead, so moving
  through a folder feels instant.
- Move with the arrow keys, Home and End, or the mouse's back and forward buttons.
- Sort the folder by file name (natural order, so `2` comes before `10`) or by date modified.
- Drag a file or folder onto the window to open it. This works on X11 and Windows; **it isn't
  supported on Wayland yet.**
- Open Recent lists the last 10 files you opened.

## Editing

Edits never change the original file: they are kept in lightview until you save a copy. Every
edit can be undone and redone.

- **Crop** by dragging a rectangle, or to the current selection. It snaps to 1:1, 4:3, 3:2 or 16:9
  when you're close, and shows the ratio.
- **Correct Perspective.** Drag four corners onto something rectangular, such as a document or a
  painting, and press Enter to straighten it.
- **Straighten.** Turn the image by any angle from −180° to 180°, live under a grid. The canvas
  grows to fit the turned image, and the new corners get the background colour (or stay
  transparent), so you can crop afterwards.
- **Resize** by width and height, or by percentage, with the aspect ratio kept or not, and set the
  resolution (DPI).
- **Resize Optimally** shrinks the image to its *intrinsic* size: the size its detail actually
  needs. A 10 MP photo stretched to 40 MP, or a soft, out-of-focus shot, comes back to about the
  size where it starts losing detail. lightview shrinks and enlarges the image back at 20 sizes,
  measures how much detail each round trip loses, and finds where the losses start rising
  steeply. It measures the sharpest 1024 px area of the image (or of the crop), since flat areas
  like sky have no detail to measure. It takes about a fifth of a second. It never
  enlarges, running it again leaves the image as it is, and Ctrl+Z undoes it. Limits: it's an
  estimate, in steps of about 11%. Images enlarged with an AI upscaler, or with
  nearest-neighbour (blocky pixels), and very noisy photos can look like they already have full
  detail. Only the brightness is measured, so fine colour detail doesn't count.
- **Rotate and flip.**
- **Adjust Color:** exposure, contrast, saturation and tint, with **Auto Levels** to stretch the
  image to its full range.
- **Adjust Curves:** a curve per channel in RGB, HSL or HSV, over a histogram.
- **Greyscale** and **Invert Colors.**
- **Remove Red Eye** from the selected eyes.
- **Pixelate Selection** to hide faces. Each run uses a randomly placed grid and noise, so it's
  harder to undo. For text, covering it with a solid shape is safer.

## Selections and clipboard

- Rectangle, ellipse and lasso selections, Select All and Deselect. With the AI tools, Select Object
  (AI) turns a box dragged around an object into its outline.
- Cut, copy and paste images. Copy takes the selection, or the whole edited image. A pasted image
  becomes an element you can move and resize.
- Delete Selection erases it: it becomes transparent where the format allows, otherwise the
  background colour.
- Copy File puts the file itself on the clipboard, for pasting into a chat or a file manager.

## Markup

Circle, point out or note something without a second app. Open the drawing toolbar from Tools.

- Freehand sketching, rectangles, rounded rectangles, ellipses, lines and arrows.
- Text you can click to place and double-click to edit again.
- Highlights (a translucent rectangle).
- A magnifier lens that enlarges part of the image.
- A measuring tool showing distance, ΔX and ΔY in image pixels.
- Colour, thickness, fill and font size for each element. Elements stay editable until you save,
  and what you see is what gets saved, at the image's own resolution. Remove Red Eye, Pixelate
  Selection and the Local AI tools that change pixels merge markup and colour changes into the
  image, so later colour changes reach their result too; Ctrl+Z undoes that.

## Files and safe deleting

- **Save Copy** (Ctrl+S) writes `name-edited.ext` next to the original, with no dialog.
- **Save As** saves a copy in the format given by the extension: JPEG (with EXIF kept), PNG, WebP
  (lossless), BMP, TIFF or GIF.
- **Rename** with F2. Neither Save Copy nor Rename ever replaces another file.
- Copy the file's path, or show it in your file manager.
- **Deleting** always asks first, then moves the image to the system Trash. Ctrl+Z brings it back.
- Before leaving an image with unsaved edits, lightview asks.
- **Group Duplicates in Folder** (Tools) compares every image in the folder and moves each group of
  look-alikes, the first one included, into a new folder named after the group's first image. A
  histogram shows how close the images are, with a suggested threshold, and a Smallest group slider
  leaves groups under that size (a pair of copies, say) where they are; moving the sliders shows the
  groups before anything moves. Images that only change little by little, like a burst, stay in one
  group. Nothing is deleted or stored on disk, the viewer stays in the folder, and Ctrl+Z moves
  everything back. Limits: subfolders aren't looked at; rotated or mirrored copies and heavy crops
  aren't found; very plain images (blank pages) can group together, and a slow series like a
  timelapse can join into one big group, so lower the slider to split it; sidecar files (`.xmp`)
  stay behind.

Animated images and multi-page TIFFs can be viewed, but not saved yet.

## Metadata

- **Inspector sidebar** (Ctrl+I) with the file's details, camera and lens, exposure, date taken,
  location and the image description.
- **EXIF editing** for JPEG, PNG and TIFF, which always saves to a copy. You can also remove the
  location. WebP and RAW metadata is shown, but not edited.

## AI tools

Background removal, blur or replacement, object removal, denoising, upscaling, low-light enhancement and text
recognition, all running on your own computer, on the CPU or a GPU (NVIDIA, or experimentally
AMD, Intel and others). Nothing is installed
until you ask for it. Denoise and Enhance show a before-and-after preview of the visible area
first, with a divider you can drag. Blur Background and Replace Background use the Remove
Background model: the subject stays sharp over a blurred copy of the photo, whose strength you set
while watching the result, or over the background colour from Settings; a little of the subject's colour can show around its edges in the blur.
Select Object (AI) selects one object at a time from a box you drag around it (about 2 s on the
CPU); only its outer outline is kept, so holes in it, like the space between an arm and the body,
stay selected. Crop, Remove Object, Delete and Pixelate then work on it like on any selection. Read QR Code needs no download, and links are only shown,
never opened.

The models, how to install them and the tips for text recognition are in the README's
[AI tools](../README.md#ai-tools) section.

## Languages, shortcuts and accessibility

- **Twenty-eight languages:** English, Spanish, French, German, Italian, Portuguese, Polish, Dutch,
  Russian, Japanese, Chinese (Simplified and Traditional), Korean, Ukrainian, Turkish, Indonesian,
  Vietnamese, Czech, Swedish, Hungarian, Romanian, Greek, Danish, Finnish, Norwegian (Bokmål),
  Catalan (also used for Valencian), Basque and Galician. lightview
  starts in your system's language. The texts from Polish onwards are machine translations:
  corrections are welcome ([`TRANSLATING.md`](TRANSLATING.md)).
- **Chinese, Japanese and Korean** are drawn with your system's CJK font (on Linux the one
  fontconfig picks, such as Noto Sans CJK from the `fonts-noto-cjk` package; on Windows the one
  built in). Without one, those texts show as boxes. The font is loaded once per start, so after
  switching between two of these languages, restart lightview for the new one's character style.
- **Your own shortcuts.** Any menu item can get a keyboard shortcut, and the default ones can be
  changed, in Edit → Settings → Change Keyboard Shortcuts….
- **Screen readers.** Menus, dialogs and settings can be read by a screen reader such as Orca or
  Narrator. The image itself isn't described yet.
- **Font size** for the interface, from 10 to 20.
- **Themes:** Dark, Light, High contrast, or follow the system's dark/light setting
  ([Themes](#themes)).

## Settings

In Edit → Settings: language, theme, font size, GPU image scaling (smoother zooming, more GPU memory),
interpolation (Nearest, Bilinear, Bicubic or Lanczos3), background colour, pixel grid, colour
management, default window size, sort order, keyboard shortcuts, and whether lightview is your
default image viewer. The version and a link to check for updates are at the bottom; lightview
never checks on its own.

## Themes

Edit → Settings → Theme changes the colours and shapes of menus, panels, dialogs and buttons, and of
the outlines and guides drawn over the image. It can't move, add or hide anything, and it doesn't
change fonts, the font size or the background colour behind the image, which are separate settings.

- **Theme files:** each theme is an `.xml` file in `~/.local/share/lightview/themes` (Windows:
  `%APPDATA%\lightview\data\themes`), which the Open themes folder button under the list opens.
  The first time lightview starts, it writes `dark.xml`, `light.xml` and `high-contrast.xml` there.
  Edit them, or copy one under a new name to make your own; a file's name is its name in the list.
  lightview never overwrites or recreates these files, so edits and deletions stay.
- **Choosing a theme again** reloads its file, so you can edit it and look at the result without
  restarting.
- **Follow system** (the default) uses `dark.xml` or `light.xml`, depending on the desktop's
  dark/light setting. On Linux it's read when lightview starts, so after changing the desktop's
  setting, restart lightview. On Windows it follows changes as they happen. When the desktop gives
  no preference, `dark.xml` is used.
- **The base theme:** built into lightview and identical to the original Light theme. Any key a
  file leaves out takes the base theme's value. A file that's missing or can't be read (bad XML, a
  wrong value, bigger than 64 KiB) is replaced by the whole base theme, and Settings says what's
  wrong and on which line.

The three files set every key, so they work as templates; `dark.xml` explains the format at the
top. `mode` is `dark` or `light`: it decides which half of Follow system the theme can stand in for,
and the shades of things themes don't set, such as shadows. Colours are `#rrggbb` or `#rrggbbaa`.

```xml
<theme mode="dark">  <!-- or "light" -->
  <colors panel="#1b1b1b" window="#1b1b1b" border="#3c3c3c" field="#0a0a0a" text="#8c8c8c"
          accent="#005c80" selected-text="#c0deff" hyperlink="#5aaaff" warn="#ff8f00" error="#ff0000"/>
  <widget state="hovered" fill="#404040" text="#f5f5f5" border="#969696"/>
  <!-- … one widget element for each of the five states … -->
  <canvas outline="#ffffff" handle="#42a5f5" guide="#ffff00"/>
  <shape rounding="6" stroke="1" spacing="8,3" padding="4,1"/>
</theme>
```

| Element | Attribute | Changes |
|---|---|---|
| `colors` | `panel`, `window` | Background of the menu bar and side panels; of dialogs and menus |
| | `border` | Border of dialogs and menus |
| | `field` | Background of text fields |
| | `text` | Labels and other text |
| | `accent`, `selected-text` | Selected items and text: background and text colour |
| | `hyperlink`, `warn`, `error` | Links, warnings and errors |
| `widget` | `state` | Which state: `noninteractive` (separators, frames), `inactive`, `hovered`, `active` (pressed) or `open` (an open menu or list). One element per state |
| | `fill`, `text`, `border` | That state's background, text and border |
| `canvas` | `outline` | Selection, crop and straighten outlines, and the before/after divider |
| | `handle` | The selected markup element's frame and handle |
| | `guide` | Perspective and measuring guides |
| `shape` | `rounding` | Corner radius of buttons, menus and dialogs, 0–16 |
| | `stroke` | Border width, 0.5–4; set a `border` colour for `inactive` too, or its border stays invisible |
| | `spacing`, `padding` | Space between items and inside buttons, `x,y`, each 0–24 |

Unknown elements and attributes are ignored.

## Privacy

No telemetry, no data collection, no account and no background update checks. lightview only
connects to the internet when you ask it to download something, and your images stay on your
computer.

## Supported formats

| Format | Open | Save |
|---|---|---|
| JPEG | Yes | Yes |
| PNG | Yes | Yes |
| WebP | Yes | Yes (lossless) |
| GIF | Yes | Yes |
| BMP | Yes | Yes |
| TIFF | Yes (every page) | Yes (single-page files) |
| SVG | Yes | No |
| ICO | Yes (largest size) | No |
| JPEG XL | Yes | No |
| HEIC, AVIF | Yes, with the system's codecs: libheif and its plugins on Linux, Microsoft's HEIF and HEVC or AV1 Video Extensions on Windows | No |
| APNG | Yes (animated) | No |
| QOI, TGA, PNM, DDS | Yes | No |
| HDR, EXR | Yes (tone-mapped to 8 bits) | No |
| Camera RAW: CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2 | The embedded JPEG preview, rotated as shot | No |

Animated images and multi-page TIFFs can be viewed, but not saved yet. HEIC and AVIF files open only when they are shown, without being decoded ahead, because their decoders are native code. Group Duplicates in Folder is the one exception: when you start it, it decodes every image in the folder, HEIC and AVIF included.
