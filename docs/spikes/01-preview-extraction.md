# Spike 01 — Preview extraction (Linux)

**Question:** How fast can we get pixels out of CR3 / NEF / HEIF, and should the CR3/NEF `RawProvider`s be built on `rawler` or LibRaw?

**Answer:** Use **LibRaw** (`libraw_r`, thread-safe build) through a small C shim. Every tested body embeds three to four JPEG previews, and LibRaw reports each one's byte offset and length in under 1 ms without decoding anything. The full-resolution preview is too slow to decode inside the 50 ms budget, so the mid-size 1620×1080 preview becomes the "instant" layer and full resolution is prefetched in the background.

Code: `spikes/preview-extract/` · raw output: `spikes/preview-extract-results.txt`

## Setup

- Machine: Intel i5-6200U (2016, 2 cores / 4 threads), Ubuntu 26.04. Apple Silicon should be several times faster per core.
- Files were in the page cache (warm), so these are CPU-bound numbers. Card read speed is a separate Mac spike.
- Samples: 8 public files from raw.pixls.us — Canon R5 (RAW + CRAW), R6, R10; Nikon D850, Z6, Z8 (lossless and High Efficiency). One HEIC from the libheif repo.
- Median of 5 runs. JPEG decoding by `zune-jpeg` (pure Rust).

## Results

### Every body embeds a full-resolution JPEG, plus smaller ones

| Body | Previews in file (LibRaw `thumbs_list`) |
|---|---|
| Canon R5 / R6 / R10 (CR3) | 160×120 · 1620×1080 · full res (LibRaw reports 0×0; size is read from the JPEG header) |
| Nikon D850 / Z6 / Z8 (NEF, incl. HE) | 160×120 (not JPEG) · 640×424 · 1620×1080 · full res |

Locating all previews with LibRaw takes **0.6–0.9 ms per file**. Reading one by byte range (`pread`) at the stored offset returns a valid JPEG every time, which confirms the `preview_info` design: store the offset once, never parse the RAW again to view it.

### Decode cost by preview size

| Preview | Size on disk | zune-jpeg full decode |
|---|---|---|
| 160×120 (CR3) | ~10 KB | 0.3 ms |
| 640×424 (NEF) | ~150 KB | 3.3 ms |
| 1620×1080 | 0.2–1.1 MB | 9–22 ms |
| 24 MP full res (R6, Z6, R10) | 1.3–2.4 MB | 137–147 ms |
| 45 MP full res (R5, D850, Z8) | 3–6 MB | 277–348 ms |

Reduced-size decoding through `jpeg-decoder` barely helped (45 MP at 1/8 scale: 160–170 ms), because most of the time goes into reading the compressed data, not the final transform.

### Thumbnail throughput for a 1,000-file card

| Strategy | 1 thread | 2 threads | 4 threads |
|---|---|---|---|
| A: decode the full-res preview | 250 s | 133 s | 106 s |
| **B: locate, read and decode the 1620 px preview** | 15 s | 7.9 s | **7.3 s** |

### rawler vs LibRaw

| | rawler 0.8 | LibRaw 0.21 |
|---|---|---|
| Lists all previews with offsets | No — returns only a decoded image | **Yes**, < 1 ms |
| Preview access cost | 140–330 ms (always decodes the largest) | 1–4 ms to get the JPEG bytes |
| Capture time | Yes, **with subseconds** | Seconds only |
| Body serial | Missing for the D850 | All 8 |
| Lens | All 8 | Missing for the D850 |
| Image number / shutter count | Not returned | Not returned (not queried) |
| HE NEF (Z8) | Metadata + preview OK | Previews OK |

### HEIF

libheif opened the file in 0.5 ms (1280×854, one embedded thumbnail), but **decoding failed: no HEVC decoder plugin is installed**. On Linux this needs `libheif-plugin-libde265`. On the Mac, Apple's ImageIO (hardware HEVC) is the likely HEIF provider anyway.

## Decisions and roadmap changes

1. **LibRaw backs the CR3/NEF providers.** Use `libraw_r` behind a small C shim so Rust never depends on LibRaw's struct layouts. It's LGPL-2.1/CDDL; dynamic linking is fine for a personal tool and keeps a future commercial option open.
2. **Duplicate-check fingerprint needs subsecond capture time,** which LibRaw doesn't provide. In Phase 1, read `SubSecTimeOriginal` (and the shutter count where the maker notes have it) with a small EXIF reader. This is cheap, since the EXIF block is in the first few KB. *Superseded by [spike 03](03-metadata-fingerprint.md): LibRaw's EXIF callback provides subseconds, and shutter count is dropped.*
3. **Progressive preview upgrade is needed for CR3/NEF too, not just future formats.** The loupe shows the 1620×1080 preview first (~15 ms), then swaps in full resolution (140–350 ms here). Full res is prefetched for the neighbours in the job system so it's usually ready before you arrive.
4. **Restate the speed bar:** *first pixels in < 50 ms; full resolution ready before the next keypress during normal culling pace.*
5. **Import grid fills from the smallest previews first** (160×120 / 640×424, ~10–150 KB each), then upgrades. That keeps card reads tiny, which matters more than CPU once the card is the bottleneck.
6. **Thumbnails come from the 1620 px preview,** never the full-res one (7 s vs 106 s per 1,000 files on this laptop).
7. **Mac session additions:** compare zune-jpeg against libjpeg-turbo (needs `nasm`, unavailable here) and Apple ImageIO for full-res decode, and confirm HEIC through ImageIO.
