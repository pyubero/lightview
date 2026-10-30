# lightview

A fast, not-so-minimal image viewer for Linux, written in Rust. It opens quickly, moves through 
a folder without waiting, and keeps your originals safe: edits are only ever written to a file 
when you choose to save.

## Features

- **Instant browsing.** Images are decoded on background threads and the neighbouring images are
  decoded ahead, so the window never freezes and moving to the next image feels smoother.
- **Viewing.** Fit to window, actual size, zoom at the pointer, pan, full screen, EXIF orientation,
  and animated GIF and WebP playback with frame stepping.
- **Editing.** Crop, resize, rotate, flip, colour adjustments and auto levels, with full undo and
  redo.
- **Selection and clipboard.** Rectangle, ellipse and lasso selections, plus cut, copy and paste of
  images.
- **Markup.** Freehand sketching, shapes, arrows, text, highlights, a magnifier and a measuring tool.
- **Metadata.** An inspector sidebar for image information, and EXIF editing for JPEGs, which always
  saves to a copy.
- **Optional AI tools.** Background removal, denoising, and upscaling or low-light enhancement. They
  run locally on your machine, and nothing is installed until you ask for it.

### Supported formats

| Format | Open | Save |
|---|---|---|
| JPEG | Yes | Yes |
| PNG | Yes | Yes |
| WebP | Yes | Yes (lossless) |
| GIF | Yes | Yes |
| BMP | Yes | Yes |
| TIFF | No | Yes |

Animated images can be viewed, but not saved yet.


## Installation

lightview runs on Linux, on both Wayland and X11. It has been developed and tested on Kubuntu with
KDE Plasma. Windows isn't supported yet.

### Ubuntu, Kubuntu, Mint, Pop!_OS, Debian 12 or newer (64-bit Intel/AMD)

1. Download the `.deb` file from the [Releases](../../releases) page.
2. Double-click it and choose Install.
3. Open lightview from your applications menu. The first time, it asks whether to become your
   default image viewer. You can change your mind later in Edit → Settings.

To uninstall, remove lightview in your software centre, or run `sudo apt remove lightview`.

### From source (any other distribution)

You need a [Rust toolchain](https://rustup.rs) that supports the 2024 edition. From a checkout of
this repository:

```sh
./install.sh
```

This builds a release binary, installs it to `~/.local/bin`, adds a desktop entry, and **makes
lightview your default viewer** for JPEG, PNG, GIF, WebP and BMP. To undo it, remove
`~/.local/bin/lightview` and `~/.local/share/applications/lightview.desktop`, then choose another
default app in your system settings.

To build without installing:

```sh
cargo build --release
```

### Optional system libraries

- **libjpeg-turbo** (`libturbojpeg.so.0`) makes JPEGs noticeably faster. It's loaded at run time, so
  no development package is needed, and lightview uses its pure-Rust decoder if it's missing.

The AI downloads use the system's `curl`, `sha256sum`, `tar` and `unzip`.

## Usage

```sh
lightview                # open an empty window
lightview photo.jpg      # open an image
```

[`MENUS.md`](MENUS.md) lists every menu item and shortcut.

Settings are saved in `~/.local/share/lightview/app.ron`.

## AI tools

The AI tools are in the Image menu. They need ONNX Runtime, installed with Tools → Download AI
Runtime…, and at least one model for the task. Models live in `~/.local/share/lightview/models`.

Support for NVIDIA GPUs is optional: Tools → AI Models… offers a download of about 1.5 GB. If the GPU
can't be used, lightview falls back to the CPU and shows why.

There are two kinds of model.

### Models you download from the app

Tools → AI Models… offers these models. Each download is pinned by SHA-256 and starts only after you
accept its licence.

| Task | Models |
|---|---|
| Remove Background | Silueta, U²-Net, IS-Net, BiRefNet |
| Denoise | SCUNet, SwinIR |
| Enhance Image | BSRGAN 4× |

### Models you convert yourself

> [!IMPORTANT]
> Real-ESRGAN and DarkIR **can't be downloaded from lightview**, and no ONNX files of them are
> published. Every user has to create these files themselves and copy them into the models folder.

1. Follow the instructions in
   [lightview-models](https://github.com/pyubero/lightview-models) to convert the models you want.
   The files are written to its `dist/` folder.
2. Copy them into `~/.local/share/lightview/models`, or use Tools → AI Models… → Add model file….
3. They then appear under Enhance Image in Tools → AI Models….

Keep the file names below, because lightview recognises these models by name:

| File | Model |
|---|---|
| `RealESRGAN_x4plus_fp32.onnx` | Real-ESRGAN 4× |
| `RealESRGAN_x4plus_fp16.onnx` | Real-ESRGAN 4×, half precision |
| `realesr-general-x4v3_fp32.onnx` | Real-ESRGAN general 4×, faster and softer |
| `realesr-general-x4v3_fp16.onnx` | Real-ESRGAN general 4×, half precision |
| `DarkIR_384.onnx` | DarkIR, for dark and night photos, same size |

A renamed file is treated as an unknown model: it's listed under every tool, has to be picked by
hand, and doesn't get its tile size.

lightview doesn't verify these files or show their licences. The licences are in lightview-models,
in each model's folder. DarkIR's weights are CC-BY-4.0, which requires crediting the authors if you
share the converted file.

### Your own models

Any other `.onnx` file in the models folder is listed too. Tools → AI Models… lets you adjust the
input size and normalisation of background removal models.

## Development

```sh
cargo test                 # run the tests
cargo run --release -- photo.jpg
lightview --timings photo.jpg   # print window, preview, full image and navigation times
lightview --bench photo.jpg     # the same, quitting after the first full image
```

| Document | Contents |
|---|---|
| [`PROJECT_BRIEF.md`](PROJECT_BRIEF.md) | Goals, scope, performance targets and early milestones |
| [`ROADMAP-v2.md`](ROADMAP-v2.md) | The roadmap for editing and AI features |
| [`MENUS.md`](MENUS.md) | Every menu item and shortcut |

## Licence

lightview is free software: you can redistribute it and/or modify it under the terms of the
[GNU General Public License](LICENSE), version 3 or later. It comes with no warranty.

The fonts built into lightview are Apache-2.0 and MIT licensed; see [`fonts/NOTICE.txt`](fonts/NOTICE.txt).
The `.deb` includes their licences and those of the Rust crates, in
`/usr/share/doc/lightview/copyright`.

Third-party licences for the downloadable AI components are in [`licenses/`](licenses).
