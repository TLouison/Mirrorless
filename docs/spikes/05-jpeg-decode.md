# Spike 05 — JPEG decoder (Linux half)

**Question:** Which decoder turns the embedded full-resolution JPEGs into pixels for the loupe: zune-jpeg (pure Rust, used in spike 01) or libjpeg-turbo? And can reduced-size decoding make "fit to screen" cheaper than a full decode? Apple ImageIO is the third candidate and is measured in the Mac session.

**Answer (Linux):** Use **libjpeg-turbo 3** (`turbojpeg` crate, built from vendored source with SIMD). It is **~27 % faster** than zune-jpeg at full resolution, and its scaled decode really skips work. Decoding a 45 MP preview at 1/2 scale (4128 px, enough for fit-to-screen on any current Mac display) takes **140–180 ms instead of 285–326 ms**. In spike 01, `jpeg-decoder`'s scaled mode barely helped.

Code: `spikes/jpeg-decode/` · raw output: `spikes/jpeg-decode-results.txt`

## Setup

- Same i5-6200U laptop. Inputs are the full-res JPEGs embedded in the 8 spike-01 RAWs (`exiftool -b -JpgFromRaw`), the R5's 1620 px preview, and a 12 MP phone JPEG.
- Both decoders output **BGRA**, the format gpui uploads. This makes zune-jpeg about 100 ms slower than its RGB numbers in spike 01.
- libjpeg-turbo 3.x is built by `turbojpeg-sys` with `cmake` + `nasm` (`require-simd`). The cold build adds ~40 s.
- Median of 5 runs, single-threaded, files in memory.

## Results (ms)

| Image | MP | zune full | turbo full | turbo ½ | turbo ¼ | turbo ⅛ |
|---|---|---|---|---|---|---|
| Nikon D850 | 45 | 433 | 318 | 170 | 128 | 90 |
| Nikon Z8 (lossless / HE) | 45 | 441–444 | 322–326 | 177–182 | 129–134 | 96–97 |
| Canon R5 (RAW / CRAW) | 45 | 394 | 285 | 142 | 92 | 69 |
| Canon R10 | 24 | 208 | 150 | 65 | 49 | 37 |
| Nikon Z6 | 24 | 198 | 146 | 53 | 44 | 32 |
| Canon R6 | 20 | 197 | 141 | 64 | 55 | 41 |
| Phone JPEG | 13 | 154 | 112 | 65 | 59 | 46 |
| R5 1620 px preview | 1.7 | 14.3 | 8.9 | 6.6 | 5.8 | 4.3 |

- Gains are smaller at ¼ and ⅛, because Huffman decoding of the whole file is a fixed cost that scaling can't skip.
- **None of the embedded JPEGs have restart markers** (no `DRI` segment), so one image can't be decoded in parallel slices. Parallelism comes from decoding neighbours at the same time (§3.4), not from splitting one image.

## Decisions and roadmap changes

1. **libjpeg-turbo replaces zune-jpeg** in `mm-media`, unless ImageIO wins clearly on the Mac. Build requirements: `cmake` + `nasm` (Linux: apt; macOS CI: `brew install nasm`).
2. **The loupe gets a "fit" layer between the 1620 px preview and 100 %:** 1620 px preview (~9 ms) → scaled decode to the display size (½ or ¼, 50–180 ms) → full resolution only when zooming. The prefetcher works at fit size for neighbours and decodes full resolution only for the current image.
3. **Thumbnails** use the 1620 px preview at ¼ scale (405 px, 5.8 ms) instead of a full decode plus resize. That's 35 % faster, and only a small resize (405 → 384 px) is left.
4. **Mac session:** run this exact binary on Apple Silicon, and add an ImageIO column (`CGImageSourceCreateThumbnailAtIndex` for fit, `CGImageSourceCreateImageAtIndex` for full).
