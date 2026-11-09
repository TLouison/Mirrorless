# Spike 02 — gpui thumbnail grid (Linux, functional half)

**Question:** Can gpui show a 10,000-item thumbnail grid built from our own pixel buffers, with bounded GPU memory and keyboard navigation?

**Answer (Linux): yes.** Every functional question passed. Smoothness and GPU-memory numbers must be measured on the Mac (Metal, Retina); the Linux run used CPU software rendering.

Code: `spikes/gpui-grid/` · screenshot tool: `spikes/tools/xshot.py`

## Setup

- gpui from Zed `main` at `d2a4fb6ded7c470be25d1541281da3882495b65b` (path dependency on a shallow clone for the spike).
- Ubuntu 26.04 on an i5-6200U, **Xvfb** virtual display, **lavapipe** (CPU-based Vulkan) through gpui's wgpu renderer, 1280×800 window.
- 10,000 cells, 6 columns. Each cell gets a **unique** 384×256 BGRA texture: a tinted copy of one of 8 real 1620 px embedded previews from spike 01, built on gpui's background executor to simulate a thumbnail job.
- Benchmark mode scrolls 60 px per frame from top to bottom (1,667 rows) and then quits.

## Results

| Check | Result |
|---|---|
| Raw pixel buffers → `RenderImage` → `img()` | Works. BGRA `image::Frame`, no file or encoder round-trip. |
| Virtualized grid | `uniform_list` of rows, rendering only the visible rows; column count follows window width. |
| Freeing GPU textures | `cx.drop_image()` frees atlas entries. 10,000 created, 9,500 dropped, 500 resident (the cap). |
| Memory over a full scroll | Levels off: 637 MB at 10 s → 658 MB at 180 s, while 9,500 textures cycle through. No leak. |
| Keyboard | Real keystrokes (`right right down left`) through keymap → focus → action moved selection to the expected cell. |
| Thumbnail job → on screen | p50 33 ms, p95 66 ms (background build + main-thread insert). |
| Frame time | p50 48 ms, p95 64 ms. **Not meaningful:** lavapipe rasterizes every pixel on 2 old CPU cores. |

## Gotchas found

1. **Zed's `[patch.crates-io]` isn't inherited.** gpui needs Zed's forks of `async-task`, `async-process`, `calloop` and `khronos-egl`; our workspace must copy those patch lines whenever we bump the Zed revision.
2. **Platform code moved into `gpui_platform`.** Apps call `gpui_platform::application()`. The crates.io `gpui 0.2.2` release is a year behind this API.
3. **Bundle a UI font.** gpui panicked on a machine without any of its fallback fonts. Mirrorless should ship its own UI font and register it at startup.
4. **Don't dispatch keystrokes during `render`.** The dispatch tree doesn't exist yet. Use `window.on_next_frame`.
5. **Rust 2024 `impl Trait` capture rules:** helper render functions taking `&mut self` should return concrete types (`Div`) or use `+ use<>`.
6. **Texture memory is held twice** — our `Arc<RenderImage>` keeps CPU bytes while the atlas holds the GPU copy. 500 thumbnails at 384×256 ≈ 197 MB each side. On Apple Silicon's unified memory both count. Phase 1 should size the cache from the visible area (plus a margin) rather than a fixed 500, and evaluate dropping CPU bytes after upload.
7. **Cold build:** ~12 minutes on this 4-thread laptop; incremental rebuilds of the spike take seconds.

## Still to measure on the Mac

- Scroll smoothness at 60/120 Hz on Metal with a Retina window (2× texture size).
- GPU/unified memory with the cache at its cap.
- Trackpad momentum scrolling feel.
