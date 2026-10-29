# lightview menus

The menu bar as built in `src/app.rs` (`menu_bar`). The bar is hidden in full screen. "Needs an
image" means the item is greyed out until an image is fully loaded. `▸` marks a submenu.

## File
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Open… | Ctrl+O | always | System file dialog; lists jpg, jpeg, png, gif, webp, bmp. |
| Open Recent ▸ | | always | The last 10 files opened explicitly (hover shows the full path); "No recent files" when empty. |
| Close | Ctrl+W | always | Closes the window (asks first if there are unsaved edits). |
| — | | | |
| Save As… | Ctrl+Shift+S | a still image is loaded | Saves a copy with all edits; the format follows the extension. |
| Export ▸ | | a still image is loaded | Save As in one chosen format: |
| &nbsp;&nbsp;JPEG… | | | quality 90, EXIF kept |
| &nbsp;&nbsp;PNG… | | | |
| &nbsp;&nbsp;WebP… | | | lossless |
| &nbsp;&nbsp;BMP… | | | |
| &nbsp;&nbsp;TIFF… | | | |
| &nbsp;&nbsp;GIF… | | | |

## Edit
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Undo | Ctrl+Z | there's something to undo | |
| Redo | Ctrl+Shift+Z | there's something to redo | |
| — | | | |
| Select All | Ctrl+A | needs an image | Selects the whole image. |
| Deselect | Ctrl+Shift+A | there's a selection | |
| Copy | Ctrl+C | needs an image | Copies the selection, or the whole edited image, to the clipboard. |
| Paste | Ctrl+V | needs an image | Pastes a clipboard image as a movable element. |
| Cut | Ctrl+X | there's a selection | Copy, then Delete Selection. |
| Delete Selection | Delete | there's a selection | Erases it: transparent in PNG/WebP/GIF/TIFF or images with alpha, else the background colour. |
| Copy Selected Region | | there's a selection | Same as Copy. |
| Create Image from Selection | | there's a selection | Crops to the selection (erasing outside an ellipse or lasso). |
| — | | | |
| Crop | | needs an image | Crop tool: drag a rectangle, Enter applies, Esc cancels. |
| Resize… | | needs an image | Width, height, keep aspect ratio, percentage, resolution (DPI). |
| — | | | |
| Adjust Color… | | needs an image | Panel with Exposure, Contrast, Saturation, Tint and Auto Levels; Done makes it one undo step. |
| Reset Color Adjustments | | needs an image | |
| — | | | |
| Rotate Left | Ctrl+L | needs an image | |
| Rotate Right | Ctrl+R | needs an image | |
| Flip Horizontally | | needs an image | |
| Flip Vertically | | needs an image | |

## View
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Previous Image | ← | not on the first image of the folder | |
| Next Image | → | not on the last image of the folder | |
| — | | | |
| Zoom In | Ctrl+= (or Ctrl++) | needs an image | Also the mouse wheel / pinch. |
| Zoom Out | Ctrl+− | needs an image | |
| Actual Size | Ctrl+1 | needs an image | 1:1 pixels. |
| Fit to Window | Ctrl+0 | needs an image | |
| — | | | |
| Enter Full Screen | F11 | always | Esc leaves full screen. |
| — | | | |
| Show / Hide Image Information | Ctrl+I | needs an image | The inspector sidebar: General, EXIF, Location, Accessibility. |
| — | | | |
| Animation ▸ | | the image is animated | |
| &nbsp;&nbsp;Play Animation | | it's paused | |
| &nbsp;&nbsp;Pause Animation | | it's playing | |
| &nbsp;&nbsp;Next Frame | | | Pauses and steps forward. |
| &nbsp;&nbsp;Previous Frame | | | Pauses and steps back. |

## Tools
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Edit EXIF… | | a JPEG is loaded | Opens the inspector sidebar to edit EXIF (saves a copy). |
| Rectangular Selection | | needs an image | Selection tool (shown ticked while active). |
| Elliptical Selection | | needs an image | |
| Lasso | | needs an image | |
| — | | | |
| *AI Image Processing* | | | (a heading, not an item) |
| Remove Background | | image, AI runtime and a background model installed, no AI job running | |
| Remove Background and Crop | | same | Also crops to the subject. |
| Denoise | | image, AI runtime and a Denoise model installed, no AI job running | Preview of the visible area first, then Apply to Whole Image / Selection. |
| Enhance Image | | image, AI runtime and an Enhance model installed, no AI job running | Same flow; with an upscaler the image becomes 4× bigger. Shows a one-time notice per model. |
| AI Models… | | always | Choose, download (after accepting licences) or add models per task; GPU support. |
| Download AI Runtime… | | only shown while the runtime is missing | Downloads ONNX Runtime (after accepting its licence). |
| — | | | |
| Measure | | needs an image | Measure tool: drag a line; distance, ΔX, ΔY in image pixels (ticked while active). |
| Adjust ▸ | | needs an image | |
| &nbsp;&nbsp;Adjust Color… | | | All five open the same Adjust Color panel. |
| &nbsp;&nbsp;Exposure… | | | |
| &nbsp;&nbsp;Contrast… | | | |
| &nbsp;&nbsp;Saturation… | | | |
| &nbsp;&nbsp;Tint… | | | |
| &nbsp;&nbsp;Auto Levels | | | Applies at once. |
| &nbsp;&nbsp;Reset Adjustments | | | Applies at once. |

## Draw
| Item | Shortcut | Enabled when | Does |
|---|---|---|---|
| Show / Hide Drawing Toolbar | | needs an image | Remembered between runs. |
| — | | | |
| Sketch | | needs an image | Freehand. |
| Shapes ▸ | | needs an image | |
| &nbsp;&nbsp;Rectangle | | | |
| &nbsp;&nbsp;Rounded Rectangle | | | |
| &nbsp;&nbsp;Ellipse | | | |
| &nbsp;&nbsp;Line | | | |
| &nbsp;&nbsp;Arrow | | | |
| Text | | needs an image | Click to place, type; double-click (Select tool) to edit. |
| Text Formatting… | | needs an image | Text tool with its style controls in the toolbar. |
| Border Color… | | needs an image | Shows the toolbar's style controls. |
| Fill Color… | | needs an image | Shows the toolbar's style controls. |
| Highlight | | needs an image | Translucent rectangle. |
| Magnifier | | needs an image | Click to place a lens. |
| — | | | |
| Image Description… | | needs an image | Opens the inspector's Accessibility section. |

## Settings
A button on the menu bar (not a menu). Opens the Settings dialog: GPU image scaling, interpolation,
background colour, default window size, sort order.

## Drawing toolbar (Draw → Show Drawing Toolbar)
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
