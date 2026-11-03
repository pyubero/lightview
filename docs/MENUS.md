# lightview menus

The menu bar as built in `src/app/menu.rs` (`menu_bar`). The shortcuts below are the defaults: Edit →
Settings → Change Keyboard Shortcuts… changes or removes them, and gives one to any item listed here except
Open Recent, Cut, Copy and Paste. The bar is hidden in full screen. Each action
appears in exactly one menu. Items are greyed out until an image is fully loaded, unless "Enabled
when" says otherwise. `▸` marks a submenu.

## File
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Open… | Ctrl+O | always | System file dialog; lists every format lightview opens (see [`FEATURES.md`](FEATURES.md#supported-formats)). |
| Open Recent ▸ | | always | The last 10 files opened explicitly (hover shows the full path); "No recent files" when empty. |
| Close | Ctrl+W | always | Closes the window (asks first if there are unsaved edits). |
| — | | | |
| Save Copy | Ctrl+S | a still image is loaded, with unsaved edits | Writes `name-edited.ext` next to the original with no dialog (`-edited-2` and so on; SVG and ICO become PNG). Never replaces a file. |
| Save As… | Ctrl+Shift+S | a still image is loaded | Saves a copy with all edits; the format follows the extension (JPEG at quality 90 with EXIF kept, PNG, lossless WebP, BMP, TIFF, GIF). |
| — | | | |
| Copy Path | | an image is open | Copies the file's full path as text. |
| Copy File | | an image is open | Puts the file itself on the clipboard, for pasting into a chat or a file manager. It stays there while lightview runs. |
| Show in File Manager | | an image is open | Opens the file manager with the file highlighted, or just its folder if the desktop can't highlight. |
| Rename… | F2 | an image is open, no AI job running | Asks for the new file name, with the extension left unselected. Never replaces a file; edits stay. |
| Delete Image… | Delete | an image is open | Asks first (Enter accepts), then moves the file to the Trash. Ctrl+Z restores it. |

## Edit
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Undo | Ctrl+Z | there's something to undo | Edits on the current image first, then the last image deleted this session. |
| Redo | Ctrl+Shift+Z | there's something to redo | |
| — | | | |
| Cut | Ctrl+X | there's a selection | Copy, then Delete Selection. |
| Copy | Ctrl+C | | Copies the selection, or the whole edited image, to the clipboard. |
| Paste | Ctrl+V | | Pastes a clipboard image as a movable element. |
| Delete Selection | Delete | there's a selection | Erases it: transparent in PNG/WebP/GIF/TIFF or images with alpha, else the background colour. |
| — | | | |
| Select All | Ctrl+A | | Selects the whole image. |
| Deselect | Ctrl+Shift+A | there's a selection | |
| — | | | |
| Settings… | Ctrl+, | always | GPU image scaling, interpolation, background colour, pixel grid (lines between pixels from 8× up), colour management (on: images with an ICC profile are converted to sRGB as they're decoded; off: shown as stored, and Save As keeps their profile), default window size, sort order, and a Change Keyboard Shortcuts… button, which opens the list of menu items by menu (Change records the next key combination, refused if another item or a fixed key has it; Remove; Reset All Shortcuts). |

## View
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Previous Image | ← | not on the first image of the folder | |
| Next Image | → | not on the last image of the folder | |
| First Image | Home | not on the first image of the folder | |
| Last Image | End | not on the last image of the folder | |
| — | | | |
| Zoom In | Ctrl+= (or Ctrl++) | | Also the mouse wheel / pinch. |
| Zoom Out | Ctrl+− | | |
| Actual Size | Ctrl+1 | | 1:1 pixels. |
| Fit to Window | Ctrl+0 | | |
| — | | | |
| Enter Full Screen | F11 | always | Esc leaves full screen. |
| — | | | |
| Show / Hide Image Information | Ctrl+I | | The inspector sidebar: General, EXIF (editable for JPEG, PNG and TIFF, saving a copy; WebP and RAW read only), Location, Accessibility (image description). |
| — | | | |
| Pages ▸ | | shown only for a multi-page TIFF | |
| &nbsp;&nbsp;Next Page | PageDown | | |
| &nbsp;&nbsp;Previous Page | PageUp | | |
| Animation ▸ | | the image is animated (not a multi-page TIFF) | |
| &nbsp;&nbsp;Play Animation | | it's paused | |
| &nbsp;&nbsp;Pause Animation | | it's playing | |
| &nbsp;&nbsp;Next Frame | PageDown | | Pauses and steps forward. |
| &nbsp;&nbsp;Previous Frame | PageUp | | Pauses and steps back. |

## Image
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Crop | | | With a selection: crops to it (erasing outside an ellipse or lasso). Without: the crop tool (drag a rectangle, Enter applies, Esc cancels). Within 3% of 1:1, 4:3, 3:2 or 16:9 (either way round) it snaps to that ratio, and shows it. |
| Resize… | | | Width, height, keep aspect ratio, percentage, resolution (DPI). |
| Correct Perspective | | a still image is loaded, Adjust Color/Curves closed | Drag four corners onto something rectangular; Enter straightens it and crops to it, Esc cancels. Markup is flattened into the image, after asking. |
| Straighten… | | as Correct Perspective | A panel with an angle (−180° to 180°): the image turns live under a grid, with its new bounds outlined. Done or Enter applies it through Correct Perspective: the canvas grows to fit the turned image and the new corners get the background colour (transparent with alpha), so cropping is left to you. Esc cancels. |
| — | | | |
| Rotate Left | Ctrl+L | | |
| Rotate Right | Ctrl+R | | |
| Flip Horizontally | | | |
| Flip Vertically | | | |
| — | | | |
| Adjust Color… | | | Panel with Exposure, Contrast, Saturation, Tint and Auto Levels; Done makes it one undo step. |
| Adjust Curves… | | | Panel with a curve per channel in RGB, HSL or HSV, over a histogram, and a line describing the channel; Done makes it one undo step. |
| Auto Levels | | | Applies at once. |
| Reset Color Adjustments | | | Applies at once. |
| Greyscale | | Adjust Color/Curves closed | Ticked while on. Sets Saturation to −1 (and back to 0), so Adjust Color shows it. |
| Invert Colors | | Adjust Color/Curves closed | Ticked while on. Inverts after every other colour adjustment. |
| — | | | |
| Remove Red Eye | | a selection, no AI job running | Removes the red from very red pixels in the selection. One undo step. |
| Pixelate Selection | | a selection, no AI job running | Pixelates the selection on a randomly placed grid, with random noise in each block. One undo step. Its hint says a solid shape is safer for text. |
| — | | | |
| *AI Image Processing* | | | (a heading, not an item) |
| Remove Background | | AI runtime and a background model installed, no AI job running | |
| Remove Background and Crop | | same | Also crops to the subject. |
| Blur Background… | | same | Finds the subject, then shows it over a blurred copy with a Strength slider (0–10 % of the long side, 0 being no blur; the preview updates when the slider is let go). Done keeps it as one undo step; Cancel or Esc puts the image back. |
| Replace Background | | same | Puts the subject over the background colour from Settings. One undo step. |
| Denoise | | AI runtime and a Denoise model installed, no AI job running | Preview of the visible area first, then Apply to Whole Image / Selection. |
| Enhance Image | | AI runtime and an Enhance model installed, no AI job running | Same flow; with an upscaler the image becomes 4× bigger. Shows a one-time notice per model. |
| Remove Object | | AI runtime and a Remove Object model installed, a selection, no AI job running | Fills in the selection, with no preview; Ctrl+Z undoes it. |
| Recognize Text | | AI runtime and a Recognize Text model installed, not already reading | Reads the text in the selection's bounds, or the whole image, and shows it in a window with Copy. Upright text only; a large image is searched at up to 1920 px, so select small text to read it at full size. |
| — | | | |
| AI Models… | | always | Choose, download (after accepting licences) or add models per task; GPU support (NVIDIA via CUDA, or any Vulkan / Direct3D 12 card via WebGPU). Real-ESRGAN and DarkIR are never downloaded: users convert them and copy them in (see `README.md`). |
| Download AI Runtime… | | always; only shown while the runtime is missing | Downloads ONNX Runtime (after accepting its licence). |

## Tools
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Show / Hide Drawing Toolbar | | | Remembered between runs. |
| — | | | |
| Rectangular Selection | | | Selection tool (shown ticked while active). |
| Elliptical Selection | | | |
| Lasso | | | |
| Select Object (AI) | | AI runtime and the MobileSAM model installed, no AI job running | Drag a box around one object; after a second or two the box turns into the object's outline. Esc or a new box drops a pending result. Holes inside the object stay selected. |
| Measure | | | Drag a line; distance, ΔX, ΔY in image pixels (ticked while active). |
| Copy Color | | | Click a pixel to copy its colour as `#RRGGBB` (`#RRGGBBAA` if it isn't opaque), as it's seen with the edits (ticked while active). |
| Read QR Code | | not already reading | Reads the QR codes in the selection's bounds, or the whole image, and shows what they say with Copy. Links are never opened. |
| — | | | |
| Sketch | | | Freehand. |
| Shapes ▸ | | | Rectangle, Rounded Rectangle, Ellipse, Line, Arrow. |
| Text | | | Click to place, type; double-click (Select tool) to edit. |
| Highlight | | | Translucent rectangle. |
| Magnifier | | | A live lens follows the pointer; click to place it. |

## Drawing toolbar (Tools → Show Drawing Toolbar)
Select · Sketch · Shapes ⏷ (Rectangle, Rounded Rectangle, Ellipse, Line, Arrow) · Text · Highlight ·
Magnifier · Measure · Description · *style controls for the selected element or tool* (colour,
thickness, fill, font size, zoom) · Undo · Redo · Done.

## Keys without a menu item
| Key | Does |
|---|---|
| Enter | Applies the crop (Crop tool), the correction (Correct Perspective) or the angle (Straighten). |
| Esc | Cancels the drawing in progress, else leaves the current tool, else leaves full screen. |
| Delete / Backspace | Deletes the selected element (Select tool), else the selection; with neither, Delete (not Backspace) offers to delete the image. |
| Mouse back / forward buttons | Previous / next image. |
| Mouse wheel / pinch | Zoom at the pointer. |
| Right-drag | Pans, with any tool. |
| Drag | Uses the current tool; with no tool, pans. |

## Without a menu
| What | Does |
|---|---|
| Drag a file or folder onto the window | Opens the file, or the folder's first image (asks first if there are unsaved edits). X11 and Windows only: not supported on Wayland yet. |
| Navigator (bottom-right, while zoomed in) | The whole image with what's on screen marked; click or drag on it to move there. |
| The open file changes on disk | Reloads it at the same zoom. With unsaved edits, a dialog offers Reload or Keep Edits. |
