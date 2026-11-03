# Rework: keeping lightview maintainable

A review of the code after milestone 34 (2026-11-02), looking for what will make the next year of changes cheaper. Each item is a proposal with its cost and the reason for it.

**Status (2026-11-03):** items 1–5 are done, in the commits from "Move app.rs's tests into src/app/tests.rs" to "Choose draw's starting pixels in one pure function". Item 6 is still optional.

## Verdict

The architecture holds up. Modules are split by concern, the hard parts (`view`, `edit`, `curves`, `load`, `exif_edit`) are mostly pure functions with tests (about 280 in all), decoding runs off the UI thread, and `Doc` is a plain value with snapshot undo. **No rewrite is needed.**

The friction is in a few specific places, nearly all in `src/app*`: the same facts written down twice, and state that has to be reset by hand. Those are worth fixing. After that come some mechanical file splits. Most functions are already short. The longest are `menu_bar` (193 lines), `load::load` (158), `App::handle` (156), `markup::flatten` (142), `run_actions` (120) and `canvas::draw` (113), so function length isn't the main problem.

### Sizes today (code + tests)

| File | Lines | of which code (above `#[cfg(test)]`) |
|---|---:|---:|
| `src/app.rs` | 2105 | 970 |
| `src/ai.rs` | 1767 | 1224 |
| `src/load.rs` | 1380 | 729 |
| `src/edit.rs` | 1336 | 663 |
| `src/exif_edit.rs` | 944 | 520 |
| `src/markup.rs` | 911 | 719 |
| everything else | < 700 each | |

`App` (`src/app.rs:68`) is one flat struct with about 95 fields.

## Proposals, most value per effort first

### 1. One source of truth for menu actions (high value, small)

At the moment a menu action is written down in several places:

- its label appears in `menu_bar` (`src/app/menu.rs:228`) **and** in `bindable_menus` (`src/app/menu.rs:122`);
- its enable rule ("needs an image", "needs a selection", "no AI job running") lives only in `menu_bar`;
- shortcuts skip the menu, so `run_actions` (`src/app/input.rs:258`) repeats some of those rules as match guards (`Rename if self.restoring.is_none()`, `Crop if self.doc().is_some()`, …) and leaves others out.

So the enable rules can drift apart: a shortcut can run an action that its menu item shows greyed out. Today, harmless no-ops cover for this (`edit` without a doc does nothing).

**Proposal:** add one `fn enabled(&self, action: &Action) -> bool`, and have both the menu and `run_actions` call it (skip any action it disables). Build `bindable_menus` from the same `(label, action)` lists the menu draws, so labels are written once. After this, a new action needs: the enum variant, one row, one dispatch arm and the locale lines.

Guard rail: never rename an `Action` variant. Its `Debug` text is the saved shortcut key.

### 2. Keep each tool's state inside the `Tool` enum (high value, small–medium)

`crop_rect`, `perspective` + `perspective_drag`, `straighten`, `measure` + `measure_drag` and `drag_start` are `App` fields that only mean something while one tool is active. Every switch of tool clears them by hand, with about 20 tuple assignments like `(self.tool, self.crop_rect, self.selected) = (Tool::Measure, None, None)`.

**Proposal:** move that state into the variants: `Tool::Crop(Option<Rect>)`, `Tool::Perspective { corners, drag }`, `Tool::Measure { ends, drag }`, `Tool::Straighten(f32)`. Leaving a tool then drops its state automatically, and code can't read stale state from another tool.

### 3. Group the per-image state into one struct that resets itself (high value, medium)

`reset_shown` (`src/app.rs:576`) clears about 30 fields one by one, and any new per-image field has to be remembered there. One has already been missed: `curve_panel` is never reset. It's harmless today because `adjusting` is reset and the panel hides without it, but it shows the risk.

**Proposal:** move the fields that belong to the image on screen into a `struct Shown` that derives `Default`: `doc`/`undo`/`redo`/`saved`, selection, markup editing (`style`, `selected`, `grab`, `editing_text`, `change_base`, `last_restyle`), `tool`, animation, the display caches, `compare`, `adjusting`/`curve_panel`. Then a reset is `self.shown = Shown::default()`, plus cancelling `restoring`, which has a side effect.

This **revisits the 2026-10-30 decision** that "`App` stays one flat struct, no sub-structs". That rule was about privacy: only child modules see `App`'s private fields. A private sub-struct inside `app.rs` keeps that property. Grouping now removes a class of bugs, so it's worth reopening the decision.

### 4. One place that chooses what pixels to show, and how caches are keyed (medium value, medium)

`canvas::draw` (`src/app/canvas.rs:132`) chooses among six possible sources: the preview, the decoded frame, `Doc.base`, the flattened markup, the baked shrink and the sharp SVG render. Each has its own cache (`tex_key`, `resized`, `baked`, `sharp`, `flat`, `navigator`) and its own key. Whether the caches stay correct depends on `generation += 1` in eight places, and CLAUDE.md already lists this as a gotcha.

**Proposal:** pull the choice of "which pixels, under which `Doc`" out into one pure function with unit tests for each case. Where a key can be built from `(generation, Doc)` in one helper, do that instead of bumping `generation` at each call site. Keep the caches themselves as they are; only the decision logic moves.

### 5. Split the big mixed modules (medium value, mechanical, low risk)

Do one commit per bullet, moving code only, with no behaviour change:

- **`ai.rs` → `src/ai/`**
  - `runtime.rs`: ONNX Runtime and GPU install, `fetch`, `sha256_file`, `run`
  - `catalog.rs`: `Known`, `License`, `Preprocess`, presets, `choose`, `installed`
  - `infer.rs`: sessions, `tiled`, `restore`, `inpaint`, `mask`, `cutout`
  - `ocr.rs`: `read_text` and its helpers (`text_boxes`, `ctc_decode`, `join_lines`)
  - `ui.rs`: `models_ui`, `license_ui`, `gpu_ui`, `preprocess_ui`
- **`load.rs`**: keep the decode pool and `load`. Move the TIFF/EXIF byte helpers (`tiff_readers`, `thumbnail_range`, `next_ifd_ptr`, `ifd_chain_ends`, `drop_exif_thumbnail`, `set_exif_dpi`), which `save` and `exif_edit` use too, to `tiff.rs`. Move the file operations (`rename`, trash and restore, `list_folder`, `natural_cmp`, `first_image`, `watch`, `stamp`) to `files.rs`.
- **`edit.rs`**: `read_qr` isn't an edit, so move it next to the OCR code. Move the pixel patches (`pixelate`, `remove_red_eye`, `auto_levels`) to `retouch.rs`. `Doc`, `render` and the warp stay.
- **Tests**: move `app.rs`'s 1130 test lines into `src/app/tests.rs`, which halves the file people actually read. The i18n test only scans source above `#[cfg(test)]`, so nothing changes for it.

When moving code: a `tr` string in a moved file still has to sit above that file's first `#[cfg(test)]`.

### 6. Dialog flags (low value, optional)

About 13 fields (`show_*`, `*_prompt`, `Option<…Dialog>`) each decide whether a dialog is open, and `dialogs()` calls every dialog function every frame. Settings, Shortcuts and AI Models can be open on top of each other, so leave those as they are. The confirmations that can never be open together (discard edits, delete, changed on disk, flatten before perspective) could become one `Option<Confirm>` enum. Do this only while working nearby anyway.

## Keep as it is

- **Domain modules owning their `*_ui` functions** (`edit::adjust_ui`, `curves::curves_ui`, `ai::models_ui`, `exif_edit::sections_ui`). Each panel sits next to the data it edits, and the pattern is the same across the whole tree.
- **No trait for formats or decoders.** `load` dispatches by extension and every format has its own quirk (limits, colour, orientation, pages). A trait would add a layer with one implementer per format.
- **`Doc` as a cloneable value with snapshot undo.** It's simple and tested. A command-style undo would be more code for no gain at these image sizes.
- **No workspace or crate split.** It's one binary with one maintainer, and compile time isn't a complaint. Separate crates would widen the `pub` surface and churn `Cargo.toml`.
- **`Msg` channel + `Notify`.** `App::handle` is long because it has many cases, not because it's tangled.

## Order and checks

1. Item 5 first: the tests move, then `ai/`, then the `load` and `edit` splits. It's mechanical and makes later diffs smaller to review.
2. Then 1, 2, 3 and 4, in that order. Each one is small enough for one commit.
3. Item 6 only opportunistically.

For every step: `cargo test` green, `cargo check --target x86_64-pc-windows-gnu` clean, no user-visible change, and `--bench` window/full times unchanged when compared with interleaved runs (see CLAUDE.md). Add no new dependencies, and don't touch the locale files unless a string's text changes.
