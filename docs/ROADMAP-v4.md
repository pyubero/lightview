# lightview — Roadmap v4

*Status: proposal, 2026-11-02. Nothing here is decided or built. Milestone 32 was postponed here from `ROADMAP-v3.md`, whose aim, principles and non-goals still apply.*

## Milestones

### 32. More local AI (L)
This is where lightview leads, so it deserves to stay ahead. Each model goes through the existing catalog, licence and pin flow.
- **AMD and Intel GPUs.** Only NVIDIA works today. Upscayl's reach on any GPU shows what's missing. Try ONNX Runtime's WebGPU provider (Vulkan on Linux, D3D12 on Windows), a separate plugin. Linux support needs a test, and Microsoft is steering new work away from DirectML, so it's not the first choice.
- **Blur or replace the background:** reuses Remove Background's mask, so no new model (Windows Photos).
- **Select an object:** MobileSAM (reported as Apache-2.0, about 3 s per image on an i5 CPU; check upstream), then use it with Crop, Remove Object or Delete. Preview's Smart Lasso and ACDSee's AI Object Selection are the comparison. Reopens v2's "no smart lasso".
- DONT IMPLEMENT: **Auto-find the corners** in Correct Perspective, for documents and screens.
- DONT IMPLEMENT: **2× upscaling,** the "later" item from 18b: shrink first, then upscale, for a smaller result.
- **Colourise old photos:** DDColor. The code is Apache-2.0; the weights' licence needs checking.
- **Restore faces:** GFPGAN is Apache-2.0 with bundled third-party licences, which need a careful read. CodeFormer's S-Lab licence is non-commercial, so it's out.
- **Describe image:** fill the Accessibility description with a local caption model. Needs a small model with a clean licence.
- **Done when** each model passes the same checks as v2's: a real test photo, CPU fallback, and clean cancelling.

## Earlier decisions this would reopen
None of these should change without the user's approval.

| Decision | Reopened by |
|---|---|
| No smart lasso (v2) | Milestone 32, click to select |

## Open questions
1. **Licences to verify before adding:** MobileSAM, DDColor weights, GFPGAN's bundled parts.
