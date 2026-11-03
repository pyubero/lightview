# lightview — Roadmap v4

*Status: proposal, 2026-11-02. Milestone 32's AI items were built on 2026-11-03 (marked DONE below); the rest is not decided or built. Milestone 32 was postponed here from `ROADMAP-v3.md`, whose aim, principles and non-goals still apply.*

## Milestones

### 32. More local AI (L)
This is where lightview leads, so it deserves to stay ahead. Each model goes through the existing catalog, licence and pin flow.
- DONE: **AMD and Intel GPUs.** The WebGPU plugin (`onnxruntime-ep-webgpu` from PyPI, 6 MB) is a second, experimental download in AI Models…; on the dev machine it matched the CPU's output, ran 2.5× faster on a GTX 1050 and was slower on an Intel UHD 630. Only NVIDIA worked before. Upscayl's reach on any GPU shows what's missing. Try ONNX Runtime's WebGPU provider (Vulkan on Linux, D3D12 on Windows), a separate plugin. Linux support needs a test, and Microsoft is steering new work away from DirectML, so it's not the first choice.
- DONE: **Blur or replace the background:** reuses Remove Background's mask, so no new model (Windows Photos).
- DONE (box prompt, user's choice): **Select an object:** MobileSAM (reported as Apache-2.0, about 3 s per image on an i5 CPU; check upstream), then use it with Crop, Remove Object or Delete. Preview's Smart Lasso and ACDSee's AI Object Selection are the comparison. Reopens v2's "no smart lasso".
- DONT IMPLEMENT: **Restore faces:** GFPGAN is Apache-2.0 *except* code from NVIDIA's StyleGAN2 (non-commercial licence) and DFDNet (CC BY-NC-SA 4.0), so it's out like CodeFormer, whose S-Lab licence is non-commercial (user's choice, 2026-11-03).
- DONT IMPLEMENT: **Auto-find the corners** in Correct Perspective, for documents and screens.
- DONT IMPLEMENT: **2× upscaling,** the "later" item from 18b: shrink first, then upscale, for a smaller result.
- DONT IMPLEMENT: **Colourise old photos:** DDColor. The code is Apache-2.0; the weights' licence needs checking.
- DONT IMPLEMENT: **Describe image:** fill the Accessibility description with a local caption model. Needs a small model with a clean licence.
- **Done when** each model passes the same checks as v2's: a real test photo, CPU fallback, and clean cancelling.

## Earlier decisions this would reopen
None of these should change without the user's approval.

| Decision | Reopened by |
|---|---|
| No smart lasso (v2) | Milestone 32, Select Object: reopened with the user's approval, 2026-11-03 |

## Open questions
1. **Licences to verify before adding:** DDColor weights. Settled: MobileSAM and SAM's decoder are Apache-2.0 (used); GFPGAN's bundled parts are non-commercial (not used).
