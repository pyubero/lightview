# lightview menus

The menu bar as built in `src/app.rs` (`menu_bar`). The bar is hidden in full screen. Each action
appears in exactly one menu. Items are greyed out until an image is fully loaded, unless "Enabled
when" says otherwise. `▸` marks a submenu.

## File
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Open… | Ctrl+O | always | System file dialog; lists jpg, jpeg, png, gif, webp, bmp. |
| Open Recent ▸ | | always | The last 10 files opened explicitly (hover shows the full path); "No recent files" when empty. |
| Close | Ctrl+W | always | Closes the window (asks first if there are unsaved edits). |
| — | | | |
| Save As… | Ctrl+Shift+S | a still image is loaded | Saves a copy with all edits; the format follows the extension (JPEG at quality 90 with EXIF kept, PNG, lossless WebP, BMP, TIFF, GIF). |

## Edit
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Undo | Ctrl+Z | there's something to undo | |
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
| Settings… | Ctrl+, | always | GPU image scaling, interpolation, background colour, default window size, sort order. |

## View
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Previous Image | ← | not on the first image of the folder | |
| Next Image | → | not on the last image of the folder | |
| — | | | |
| Zoom In | Ctrl+= (or Ctrl++) | | Also the mouse wheel / pinch. |
| Zoom Out | Ctrl+− | | |
| Actual Size | Ctrl+1 | | 1:1 pixels. |
| Fit to Window | Ctrl+0 | | |
| — | | | |
| Enter Full Screen | F11 | always | Esc leaves full screen. |
| — | | | |
| Show / Hide Image Information | Ctrl+I | | The inspector sidebar: General, EXIF (editable for JPEGs, saves a copy), Location, Accessibility (image description). |
| — | | | |
| Animation ▸ | | the image is animated | |
| &nbsp;&nbsp;Play Animation | | it's paused | |
| &nbsp;&nbsp;Pause Animation | | it's playing | |
| &nbsp;&nbsp;Next Frame | | | Pauses and steps forward. |
| &nbsp;&nbsp;Previous Frame | | | Pauses and steps back. |

## Image
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Crop | | | With a selection: crops to it (erasing outside an ellipse or lasso). Without: the crop tool (drag a rectangle, Enter applies, Esc cancels). |
| Resize… | | | Width, height, keep aspect ratio, percentage, resolution (DPI). |
| — | | | |
| Rotate Left | Ctrl+L | | |
| Rotate Right | Ctrl+R | | |
| Flip Horizontally | | | |
| Flip Vertically | | | |
| — | | | |
| Adjust Color… | | | Panel with Exposure, Contrast, Saturation, Tint and Auto Levels; Done makes it one undo step. |
| Auto Levels | | | Applies at once. |
| Reset Color Adjustments | | | Applies at once. |
| — | | | |
| *AI Image Processing* | | | (a heading, not an item) |
| Remove Background | | AI runtime and a background model installed, no AI job running | |
| Remove Background and Crop | | same | Also crops to the subject. |
| Denoise | | AI runtime and a Denoise model installed, no AI job running | Preview of the visible area first, then Apply to Whole Image / Selection. |
| Enhance Image | | AI runtime and an Enhance model installed, no AI job running | Same flow; with an upscaler the image becomes 4× bigger. Shows a one-time notice per model. |

## Tools
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Show / Hide Drawing Toolbar | | | Remembered between runs. |
| — | | | |
| Rectangular Selection | | | Selection tool (shown ticked while active). |
| Elliptical Selection | | | |
| Lasso | | | |
| Measure | | | Drag a line; distance, ΔX, ΔY in image pixels (ticked while active). |
| — | | | |
| Sketch | | | Freehand. |
| Shapes ▸ | | | Rectangle, Rounded Rectangle, Ellipse, Line, Arrow. |
| Text | | | Click to place, type; double-click (Select tool) to edit. |
| Highlight | | | Translucent rectangle. |
| Magnifier | | | Click to place a lens. |
| — | | | |
| AI Models… | | always | Choose, download (after accepting licences) or add models per task; GPU support. |
| Download AI Runtime… | | always; only shown while the runtime is missing | Downloads ONNX Runtime (after accepting its licence). |

## Drawing toolbar (Tools → Show Drawing Toolbar)
Select · Sketch · Shapes ⏷ (Rectangle, Rounded Rectangle, Ellipse, Line, Arrow) · Text · Highlight ·
Magnifier · Measure · Description · *style controls for the selected element or tool* (colour,
thickness, fill, font size, zoom) · Undo · Redo · Done.

## Keys without a menu item
| Key | Does |
|---|---|
| Enter | Applies the crop (Crop tool). |
| Esc | Cancels the drawing in progress, else leaves the current tool, else leaves full screen. |
| Delete / Backspace | Deletes the selected element (Select tool), else the selection. |
| Mouse wheel / pinch | Zoom at the pointer. |
| Drag | Pans (or uses the current tool). |
