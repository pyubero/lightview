## The project
*(Fill or update after asking the user's permission. Contains the project's background, motivation, goal(s) and approach. Keep entries short, one line each.)*

- lightview: a minimal image viewer for Linux, written fully in Rust, meant to be the user's default image app (Windows later).
- Motivation: an image viewer that opens instantly and feels snappy, with no bloat.
- Goals: first image on screen in under 100 ms (12 MP JPEG, cold start); instant next/prev; v1 is view only.
- Approach: egui/eframe (OpenGL) + the `image` crate, decoding on a background thread, neighbouring images decoded ahead; full spec in `PROJECT_BRIEF.md`.

## Principles (do not remove or weaken these)
1. **Think Before Coding** — surface assumptions, ambiguity, and simpler alternatives before writing anything; stop and ask if something's unclear.
2. **Simplicity First** — write the minimum code the problem needs, no speculative features or unrequested flexibility.
3. **Surgical Changes** — touch only what the task requires; don't refactor, "improve," or clean up unrelated code.
4. **Goal-Driven Execution** — turn tasks into verifiable success criteria (usually tests) and loop until they pass, rather than stopping at "looks right."

## Project Notes
*(Project-specific facts Claude has learned go here — commands, gotchas, conventions. Keep entries short, one line each.)*

- Scope and targets live in `PROJECT_BRIEF.md`; read it first.
- Dev machine: Kubuntu, KDE Plasma on Wayland, i7-8750H with turbo OFF (`intel_pstate/no_turbo=1`), so timings are pessimistic.
- Build/test: `cargo test`, `cargo build --release`. Benchmark: `target/release/lightview --bench <img>` prints decode/resize/first-frame ms and quits.
- `cargo init` creates a git repo on its own; use `--vcs none` unless git is wanted.
- Resize opaque images as RGB (U8x3), not RGBA: 9 ms vs 66 ms for 12 MP → 480×360.
- M1 result (2026-10-28): eframe alone takes 115–200 ms to the first frame; a 12 MP JPEG takes ~360–410 ms to decode in a fresh process (~190 ms warm) → P1 is ~420 ms, so the 100 ms target is missed.

## Architecture Decisions
*(Log of important architecture/approach discussions with the programmer. One line per decision: what was decided and why, not the full discussion.)*

Format: `YYYY-MM-DD — decision — reason`

- 2026-10-28 — UI with egui/eframe — needs menus/dialogs and a future Windows port.
- 2026-10-28 — decoding with the `image` crate, only png/jpeg/gif/webp/bmp — fully Rust, no C dependencies.
- 2026-10-28 — no AVIF/HEIC/SVG/RAW, gallery, delete/move or packaging in v1 — keep the app simple.
- 2026-10-28 — UI always renders with eframe's OpenGL backend (glow); a "GPU image scaling" setting (off by default) picks GPU vs CPU (`fast_image_resize`) scaling — egui has no CPU-only renderer.
- 2026-10-28 — v1 is view only: rotate/flip never change the file; Save As/Export later, Draw/EXIF editing a later phase — keep v1 small.
- 2026-10-28 — fixed 480×360 window, never resized on navigation or remembered; fit-to-window (aspect kept, small images upscaled) on black; no wrap; sort order setting (name default) — user's choice, kept as behaviours not settings.

## Self-Update Rule
Before finishing your turn:
- If you learned something project-specific that would've helped you at the start of a task (a build command, a gotcha, a convention), add a one-line entry to **Project Notes**.
- If you and the programmer discussed and settled on an architecture or approach (chose a pattern, rejected an alternative, set a constraint), add a one-line entry to **Architecture Decisions**. Log the decision, not the conversation.

Don't ask permission to do this — just do it. Never edit the Principles section. Keep each section under ~30 lines; if a section grows past that, prune the stalest or most redundant entries rather than appending forever.
