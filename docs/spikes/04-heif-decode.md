# Spike 04 — HEIF/HEIC decode (Linux)

**Question:** Spike 01 could open HEIC files but not decode them, because no HEVC decoder was installed. With a decoder plugin, does the HEIF provider work on Linux, and is it fast enough to share the CR3/NEF preview strategy?

**Answer:** It works, but **HEIF has no cheap preview path.** Unlike CR3/NEF, there is no large embedded JPEG: the only small image is an optional ~320 px thumbnail, and the iPhone file has none. Every larger view is a full HEVC decode, which takes **0.8–1.6 s for a 12 MP image** on this laptop with libde265. That breaks the loupe's < 50 ms first-pixel bar. HEIF therefore needs its own preview plan, decided after measuring Apple's hardware decoder on the Mac.

Code: `spikes/heif-decode/` · raw output: `spikes/heif-decode-results.txt`

## Setup

- libheif 1.21.2 with `libheif-plugin-libde265` (1.21.2) and libde265 1.0.16, unpacked locally and loaded through `LIBHEIF_PLUGIN_PATH`.
- Files:
  - `libheif_example.heic`: 1280×854, one 320×212 thumbnail.
  - `exif.heic`: an **iPhone 12 Pro** photo, 3024×4032, a 48-tile grid, no thumbnail.
- No camera HEIF (Canon `.HIF`, 10-bit HDR PQ) was available publicly. That is a Mac-session item, shot on the author's own body if it supports HEIF.
- Median of 5 runs; output RGBA, 8-bit.

## Results

| File | Open | Thumbnail decode | Primary, 1 thread | 2 threads | 4 threads |
|---|---|---|---|---|---|
| libheif example, 1.1 MP | 0.5 ms | 9 ms (320×212) | 130 ms | 130 ms | 130 ms |
| iPhone 12 Pro, 12 MP grid | 2.8 ms | — none | 1,614 ms | 913 ms | 771 ms |

- Without the plugin, libheif lists only the `aom` (AV1) decoder and fails with *"Decoder plugin generated an error"*. That matches spike 01.
- **Setting `LIBHEIF_PLUGIN_PATH` replaces the default plugin directory instead of adding to it** (`aom` disappeared from the list). The app should rely on system packages on Linux, not set this variable.
- Grid images decode their tiles in parallel (`set_max_decoding_threads`). A single-image HEIF can't be split this way.
- HEVC has no equivalent of JPEG's cheap reduced-size decode, so a full decode is the only option for anything bigger than the thumbnail.
- libheif's FFmpeg decoder plugin might be faster, but it pulls in ~35 shared libraries. It wasn't tried, because Linux isn't the dogfood platform.

## Decisions and roadmap changes

1. **The HEIF provider is real on Linux** with `libheif-plugin-libde265` as a runtime dependency. It's development-only, because the Mac uses ImageIO (next item).
2. **HEIF is the first format that needs Tier 2** (the rendered preview cache). The provider serves:
   - Tier 0 from the embedded thumbnail when present, otherwise from a decode at import.
   - Tier 1 returns `TierUnavailable`.
   - The loupe needs a cached ~2560 px render made at import time.
   The cost: about 300 KB per HEIF photo on disk, plus decode time during import.
3. **The Mac decides how much of point 2 is needed.** ImageIO uses Apple's hardware HEVC decoder. If a 12 MP decode is ≲ 50 ms there, skip the Tier 2 cache on macOS and decode on demand like a JPEG. Add to the Mac session: ImageIO decode time for the iPhone file and for a real camera `.HIF`, and a check that 10-bit PQ files tone-map correctly.
4. **Import grid on HEIF-heavy cards:** with no thumbnail, every file is a full decode before it can appear. Show a placeholder tile immediately and fill it from the job queue at `Visible` priority. Never block the grid on decode.
