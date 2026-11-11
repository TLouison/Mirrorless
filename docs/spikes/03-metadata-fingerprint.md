# Spike 03 — Metadata for the import fingerprint (Linux)

**Question:** Where do the duplicate-check fields come from (roadmap §4.3)? Spike 01 found that LibRaw doesn't return subsecond capture time, so we expected to need a second EXIF parser for RAWs. Is that true, and which reader handles JPEG and HEIF?

**Answer:** No second parser is needed for RAWs. LibRaw's EXIF callback (`set_exifparser_handler`) gives us `SubSecTimeOriginal`, `OffsetTimeOriginal` and the body serial during the **same open** that lists the embedded previews, at no extra cost. JPEG and HEIF use **kamadak-exif**. **Drop shutter count from the fingerprint:** Canon stores it only in CR3 timed metadata, and the fingerprint doesn't need it.

Code: `spikes/metadata/` · raw output: `spikes/metadata-results.txt`

## Setup

- Same machine and RAW samples as spike 01, plus four files with EXIF: an iPhone 12 Pro HEIC and a vivo phone JPEG from the nom-exif test data, and Canon 40D / Nikon D70 JPEGs from `ianare/exif-samples`.
- The expected values come from ExifTool 13.61.
- "Bytes read" is the process's `rchar` delta: bytes requested from the kernel, including stdio buffering. On an SD card this is what crosses the reader.
- Median of 7 runs, with files in the page cache.

## Results

### What ExifTool says is available

| Field | CR3 (R5/R6/R10) | NEF (D850/Z6/Z8) |
|---|---|---|
| `SubSecTimeOriginal` | EXIF IFD, all | EXIF IFD, all |
| `OffsetTimeOriginal` | EXIF IFD, all | EXIF IFD, all (even the 2017 D850) |
| Body serial | EXIF `BodySerialNumber`, all | EXIF on Z6/Z8; **D850: maker notes only** |
| Shutter count | **CR3 timed-metadata track (CTMD)** — not EXIF or maker notes | Nikon maker notes (`0x00a7`) |

### Readers compared

| | LibRaw + EXIF callback | nom-exif 3.8 | kamadak-exif 0.6 |
|---|---|---|---|
| CR3 | ✅ all fields, ~1 ms, 470–570 KB read | ✅ all fields, 0.06 ms, **30–36 KB read** | ❌ unsupported format |
| NEF | ✅ all fields incl. D850 serial, ~0.8 ms, 270–350 KB read | ❌ **fails on all four** (`malformed ifd entry … Incomplete`) | ⚠️ correct but **reads the whole file** (23–58 MB, 13–33 ms) |
| JPEG | n/a | ✅ 8–280 KB | ✅ **8–17 KB** |
| HEIC (iPhone) | n/a | ✅ 8 KB | ✅ 24 KB |
| Subseconds | ✅ | ✅ (joined into a DateTime) | ✅ |

- The callback receives EXIF IFD tags with their plain tag number, IFDn tags as `tag | (n+1) << 20`, and **no maker-note tags**. LibRaw parses the maker notes itself, which is how it gets the D850 serial; it doesn't pass them to the callback.
- `other.shot_order` was 0 for every file, so LibRaw doesn't supply an image counter for these bodies.
- `libheif_example.heic` from spike 01 contains no EXIF at all. Both readers return a clean error, which the HEIF provider must treat as "no metadata", not as a failure.

## Decisions and roadmap changes

1. **RAW metadata comes from the LibRaw open we already do.** The C shim registers an EXIF callback and copies `DateTimeOriginal`, `SubSecTimeOriginal`, `OffsetTimeOriginal` and `BodySerialNumber`. The body serial falls back to `shootinginfo.BodySerial` (maker notes). One parse per file gives previews and metadata together. The shim becomes C++ (it needs `LibRaw_abstract_datastream::read`), and it allocates the `LibRaw` object on the heap because it is too large for worker-thread stacks.
2. **JPEG and HEIF use kamadak-exif.** It's correct on all four files and reads the fewest bytes. Never use it for NEF, because it reads the whole file.
3. **Fingerprint = `DateTimeOriginal` + subseconds + body serial + file size.** Shutter count is dropped:
   - Canon keeps it in the CR3 CTMD track, which would need a third parser.
   - A fingerprint collision costs only one extra hash, because a fingerprint hit is always confirmed by BLAKE3 (§4.3). It can never cause a wrong "duplicate" verdict.
   - Two different frames with the same serial, the same 1/100 s and the same compressed byte size are effectively impossible.
   - RAW+JPEG twins have different sizes, so they get different fingerprints, which is correct: each file is deduplicated separately.
   - Phones have no serial, but they record milliseconds; time + size is enough there.
4. **Store `OffsetTimeOriginal`** with the capture time. It's present on every modern body and makes "camera local time" (confirmed default §8.5) unambiguous later.
5. **Watch on the Mac session:** LibRaw reads ~0.5 MB per CR3 to open it, about 15× more than nom-exif. That's roughly 0.5 GB per 1,000-file card just for the scan. If the card-throughput spike shows the first grid paint is I/O-bound, use nom-exif for the CR3 *metadata* pass and open LibRaw lazily. Keep this decision behind the `RawProvider` trait.
