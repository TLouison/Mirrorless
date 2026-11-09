# Mirrorless — MVP Roadmap

> The library app that Affinity Photo never shipped. Fast import, fast culling, one-keystroke hand-off to your editor of choice. No subscription.

## 1. Product framing

**Who:** Hobbyist photographers who left Lightroom over the subscription and now edit in Affinity Photo (later Photoshop, darktable, …) but miss having a library.

**What Mirrorless is:** importer + library + culler + editor launcher.
**What Mirrorless is not (for MVP):** a RAW developer. Pixels are edited in the external editor; Mirrorless never modifies originals.

**MVP is done when the author switches to it for real shoots:**
1. Insert a real ~1,000-shot card (CR3 / NEF / RAW+JPEG / HEIF) → detected → instant preview grid → duplicates skipped → copied + verified into `YYYY/MM/DD/` with no Finder involvement.
2. Cull that shoot (pick/reject, 1–5 stars, 2-up compare, rejects → Trash) at embedded-preview speed.
3. Send a pick to Affinity Photo, save, and see the edited file appear stacked with its RAW.

## 2. Decisions locked in

| Area | Decision |
|---|---|
| Dogfood platform | **macOS**. Develop on Linux; platform-specific code behind traits. CI on both. |
| Stack | **Rust + gpui** (pinned to a specific Zed git rev). |
| Library model | **Folder-first.** Files live in a plain tree the user owns. SQLite is an index/cache, not the source of truth. |
| Metadata durability | Ratings/flags/labels written to **XMP sidecars** so DB loss or leaving the app is non-catastrophic. |
| Formats | Canon CR3, Nikon NEF, JPEG, HEIF/HEIC; RAW+JPEG pairs grouped as one photo. |
| Speed bar | **Embedded-preview speed** (Photo Mechanic class): first pixels of the next image in < 50 ms; full resolution ready before the next keypress at normal culling pace (see [spike 01](spikes/01-preview-extraction.md)). No in-app demosaic in MVP. Unsupported formats and unavailable preview tiers return a **typed error the UI displays**; the gap for adding them later is defined now (§3.2). |
| Culling keys | Pick / Reject, 1–5 stars, 2-up compare with synced zoom/pan. |
| Rejects | "Delete rejects" moves the whole group (RAW, JPEG, sidecar) to **macOS Trash**. |
| Import trigger | Card detected → **prompt window with preview grid** of new (non-duplicate) shots → Enter to import. |
| Destination layout | `<Library root>/YYYY/MM/DD/<original filename>` from EXIF capture time. |
| Dedup | **Metadata fingerprint, confirmed by content hash** (see §4.3). |
| Second-copy backup | Desired, **post-MVP**. Design the copy engine for N destinations from day one. |
| Editor hand-off | Send the **original RAW**; Affinity's Develop persona converts it. Watch for the saved result and stack it. |
| Organization in MVP | Folders, filters (rating / flag / camera / lens / file type), collections (albums). |
| Lightroom migration | **Strategic goal, post-MVP.** Schema must map LR concepts cleanly now. |
| Distribution | Personal tool first. No signing/notarization/licensing work in MVP. |

## 3. Architecture

```
mirrorless/
  crates/
    mm-core       domain model, catalog (SQLite via rusqlite), queries, undo log
    mm-media      metadata + embedded-preview extraction, decode, thumbnail cache
    mm-xmp        XMP sidecar read/write (Adobe-compatible)
    mm-import     volume scanning, dedup, copy/verify engine
    mm-platform   traits: VolumeWatcher, Trash, EditorLauncher, FsWatcher; macOS + Linux impls
    mm-app        gpui UI (grid, loupe, compare, import window, sidebar)
```

Rules: `mm-app` is the only crate depending on gpui. Everything else is headless and testable on Linux with fixture files.

### 3.1 Data model (sketch)

- `root` — a library root folder (one in MVP, many later).
- `folder` — path relative to root.
- `file` — one file on disk: path, size, mtime, BLAKE3 hash, role (`raw` | `jpeg` | `heif` | `derived` | `sidecar`).
- `photo` — the logical unit the user rates. Groups a RAW, its JPEG twin, and derived edits (stack). Holds capture time, camera make/model/serial, lens, dimensions, orientation, fingerprint.
- `rating` — stars, flag (pick / reject / none), color label (stored now, UI later).
- `collection` + `collection_photo`.
- `edit_session` — photo, editor, launched_at; used to attribute new files during round-trip.
- `preview_info` — per file: embedded preview dimensions, byte offset/length, and which provider handled it (§3.2). Lets the app decide per file whether the embedded JPEG is "good enough" without re-parsing.
- Reserved for AI (§6.1), not built in MVP: `burst` (photo → burst group), `analysis` (photo → model, version, scores JSON), `face` (photo, bbox, embedding, person_id), `person`. Migrations add them later; listed here so the photo/file split doesn't fight them.

These map directly onto Lightroom Classic's `Adobe_images`, `AgLibraryFile`, `AgLibraryCollection` tables, which keeps the eventual LR importer tractable.

### 3.2 Preview strategy — the core of "lightning fast"

Embedded JPEG previews vary a lot by manufacturer. Typical sizes, approximate and listed for context only:

| Format | Typical embedded preview | Good enough for 100 % zoom? |
|---|---|---|
| Canon CR3 / CR2 | Full resolution | Yes |
| Nikon NEF (incl. Z HE/HE*) | Full resolution | Yes |
| Pentax PEF / Leica DNG | Usually full resolution | Usually |
| Panasonic RW2 | ~1920 px | Fit-to-screen only |
| Olympus / OM ORF | ~3200 px | Fit-to-screen on most displays |
| Sony ARW | ~1616×1080 | No — soft even full-screen on a Retina display |
| Fuji RAF | ~1920 px (X-Trans) | No |
| Generic / phone DNG | Anything from none to full | Varies |

So the embedded JPEG can't be the whole strategy forever. **MVP builds only what CR3/NEF/JPEG/HEIF need**, but defines the seams where the rest plugs in.

#### Preview tiers

```
request(photo, target_px) -> Result<Preview, PreviewError>
  Tier 0  thumbnail cache (~384 px)                        MVP
  Tier 1  embedded JPEG, read by byte range from the RAW    MVP
  Tier 2  rendered preview cache (~2560 px)                 not built: returns PreviewError::TierUnavailable
  Tier 3  on-demand full RAW render (demosaic)              not built: returns PreviewError::TierUnavailable
```

#### Raw providers (the extension seam)

- `mm-media` defines a `RawProvider` trait: `metadata()`, `embedded_preview()`, and later `render()`. Each format is handled by a provider looked up by file type.
- MVP ships providers for CR3, NEF, JPEG and HEIF only. **How providers get registered and customized** (static registry, per-format config, full dependency injection) is deliberately left open; the trait boundary is the only commitment.
- No provider for a file type → `PreviewError::UnsupportedFormat { format }`. Provider exists, but the request needs a tier it can't serve → `PreviewError::TierUnavailable { tier, reason }`.

#### How errors show up in the UI

- **Unsupported format** (e.g. a Sony ARW on a card): the file still appears in the grid and the import window, with a placeholder tile saying *"ARW previews not supported yet"*. It can still be imported, rated, deduplicated by hash and sent to an editor. Only the preview is missing.
- **Tier unavailable** (e.g. 100 % zoom on a file whose embedded JPEG is small): show the best tier available plus an inline notice, *"Full-resolution preview not available for this format"*.
- Errors are values, not panics or log lines: every preview request path must handle them, so a future Tier 2/3 provider replaces an error with an image without touching the UI.

#### Performance rules

- The loupe shows the best result available immediately and swaps in a better one if it arrives later. In MVP that means thumbnail → 1620×1080 embedded preview (~15 ms) → full-resolution embedded JPEG (140–350 ms on a 2016 laptop for 24–45 MP). CR3 and NEF both carry all three, so this upgrade path is exercised from day one.
- **Thumbnails** (~384 px long edge, ~20 KB): generated once at import/index from the 1620 px embedded preview — never the full-res one (7 s vs 106 s per 1,000 files in spike 01) — into an on-disk content-addressed cache. 250k photos ≈ 5 GB.
- The import grid paints first from the smallest embedded preview (160×120 CR3 / 640×424 NEF, ~10–150 KB read per file), then upgrades.
- **No large-preview cache for CR3/NEF.** The embedded JPEG is already on disk inside the RAW; caching it would double storage (~100 GB at 250k photos).
- In loupe: decode full resolution for the current image and ±N neighbors ahead of time into an LRU of GPU textures; cancel stale work when the user jumps (via the job system, §3.4).
- JPEG decode: `zune-jpeg` (or libjpeg-turbo via `turbojpeg`) with DCT downscaling for fit-to-screen; full decode only on zoom.
- Metadata/preview extraction for CR3/NEF: **LibRaw** (`libraw_r`, thread-safe) behind a small C shim; it lists every embedded preview's offset and length in < 1 ms. LibRaw lacks subsecond capture time, so a small EXIF reader supplies `SubSecTimeOriginal` for the fingerprint. JPEG decode: `zune-jpeg` for now; compare libjpeg-turbo and Apple ImageIO on the Mac. HEIF via `libheif-rs`; on macOS, ImageIO is an acceptable fallback inside the HEIF provider.

### 3.3 XMP sidecars

- Naming: Adobe convention `IMG_0001.xmp` (shared by a RAW+JPEG pair), which Lightroom, Bridge, Capture One, and Affinity understand.
- `xmp:Rating` 0–5; **reject = `xmp:Rating -1`** (Adobe convention). Pick has no standard field → custom `mirrorless:Pick` namespace.
- Writes are debounced and atomic (write temp + rename). On index, existing XMP is read in, so a re-index rebuilds ratings after DB loss.
- Never write into RAW or JPEG files.

### 3.4 Job system (multithreaded)

All background work — card scanning, copying, hashing, thumbnails, loupe prefetch, XMP writes, indexing, and later AI analysis — goes through one scheduler in `mm-core`. It runs many jobs at once, but limits concurrency per *resource* rather than globally, because the bottleneck is usually a specific disk, not the CPU.

**Each job declares:**
- **Priority:** `Interactive` (the image you're looking at) > `Visible` (on-screen thumbnails, loupe neighbors) > `Import` > `Background` (indexing, hashing old files) > `Analysis` (AI, later).
- **Resource class:** `Io(device)`, `Cpu`, or `Accelerator` (GPU/Neural Engine, later). A job that needs both I/O and CPU runs its read on the I/O pool and hands the bytes to the CPU pool.
- **Key:** for de-duplication (two requests for the same thumbnail become one job) and cancellation.

**Worker pools:**

| Pool | Concurrency | Why |
|---|---|---|
| CPU (decode, resize, hash) | cores − 1 | Leave a core for the UI thread so scrolling never stutters. |
| I/O per device | SD card 1–2, internal SSD 4–8, external HDD/network 1–2 (configurable) | Parallel random reads on an SD card or spinning disk are *slower* than sequential. Grouping by device lets a card copy, a thumbnail read from the SSD, and a write to an external drive all proceed at once. |
| Catalog writer | 1 thread | SQLite allows one writer. Writes are batched into transactions; reads use a separate connection pool (WAL mode). |
| Accelerator | 1 (later) | AI models saturate the GPU/ANE on their own. |

**Behaviour:**
- **Priority changes while queued:** scrolling a thumbnail into view bumps its job to `Visible`; scrolling away drops it back.
- **Cancellation:** jobs carry a cancel token checked between chunks. Jumping 200 images ahead in the loupe cancels the stale prefetches.
- **Streaming pipelines:** importing one file is read from card → hash while streaming → write to destination → verify → thumbnail. Each stage runs on its own pool, so file N+1 is being read while file N is verified and file N−1 is thumbnailed.
- **Results go to the UI over a channel** and are applied on gpui's main thread. `mm-core` doesn't depend on gpui's executor, so it stays testable headless.
- **Progress and errors are reported per job group** (an import, an index run), which drives the progress UI and the error display from §3.2.
- Implementation: plain `std::thread` pools with `crossbeam-channel` and a priority queue per pool.
- **Why no async runtime (tokio etc.):** async pays off for thousands of mostly-idle tasks such as network connections. Here the work is file I/O and CPU, with a few dozen jobs in flight at most. On macOS, file I/O is blocking even in tokio (`tokio::fs` hands each call to a blocking thread pool), and CPU work has to stay off an async runtime anyway. Tokio also provides none of what we need — priorities, re-prioritizing queued jobs, per-device limits — and gpui already runs its own executor (GCD on macOS), so adding tokio would mean bridging two runtimes. If network features arrive later (model downloads, cloud import, sync), use a small contained runtime or gpui's executor for just that work. The engine sits behind a submit-job / receive-results interface, so the decision can be revisited without touching the rest of the app.

## 4. Key subsystems in detail

### 4.1 Card detection

- macOS: watch `/Volumes` (FSEvents via `notify`) or DiskArbitration; a volume counts as a card if it contains `DCIM/`. Linux: same approach on `/media/$USER` and `/run/media/$USER`.
- The card is **strictly read-only** to Mirrorless. Never delete, never format.

### 4.2 Import flow

1. Volume appears → enumerate `DCIM/**` (fast, metadata only).
2. For each file, read EXIF + embedded thumbnail (header bytes only) → stream into the import window grid immediately.
3. Compute fingerprint; mark duplicates as already-imported (shown dimmed, deselected).
4. User deselects anything, presses Enter.
5. Copy engine: stream to `dest/.tmp-<name>`, hashing (BLAKE3) during the copy → fsync → re-read and verify hash → atomic rename → insert into catalog → write thumbnail.
6. Collision with a *different* file at the destination name (counter rollover, second body) → append `-1`, `-2`. Never overwrite.
7. Progress UI, cancellable, resumable (import job persisted in DB); a crash mid-import leaves only `.tmp-` files, which are cleaned up on next launch.

Engine API takes a list of destinations so the post-MVP backup copy is a config change, not a rewrite.

### 4.3 Duplicate detection

- **Fingerprint** = capture datetime incl. subseconds + camera body serial + file size (+ shutter count / image number where the format exposes it). Cheap: needs only header bytes.
- Fingerprint hit → hash the card file and compare to the stored BLAKE3 to confirm. Fingerprint miss → new.
- Every library file has a stored hash (it's free during import, background-computed for adopted folders).

### 4.4 Editor round-trip (Affinity)

- Editor registry: name, app path or bundle id, args template, accepted input types. Ship presets for Affinity Photo 2 and the newer unified Affinity app; detect which is installed.
- **Send to editor** (`⌘E`): `open -a <app> <original RAW>`; record an `edit_session`.
- Watch the photo's folder (and the editor's last save location if discoverable) for new files whose stem starts with the original's stem (`IMG_0001*.afphoto|tif|tiff|psd|jpg|png`) created after the session started → register as `derived`, stack under the original.
- Thumbnail for `.afphoto`: try Quick Look (`QLThumbnailGenerator`) on macOS; fall back to a generic badge. Exported TIFF/JPEG get normal thumbnails.
- If the saved file lands elsewhere, offer "Attach edited file…" on the photo as a manual escape hatch.

## 5. Phased plan

Sizes are relative (S ≈ days, M ≈ 1–2 weeks, L ≈ 2–4 weeks of evenings-and-weekends work).

### Phase 0 — Spikes to kill the biggest risks (M)
Each spike is throwaway code with a written yes/no outcome. Linux spikes run first; the Mac-only ones are batched into one session with a real card from a recent shoot.

| Spike | Runs on | Question |
|---|---|---|
| ~~**Preview extraction**~~ ✅ | Linux | Done: LibRaw chosen; see [spike 01](spikes/01-preview-extraction.md). HEIC decode still to confirm (needs `libheif-plugin-libde265` on Linux, ImageIO on Mac). |
| **JPEG decoder** | **Mac** | Full-res decode time: zune-jpeg vs libjpeg-turbo vs ImageIO on Apple Silicon. |
| **gpui image grid** — Linux ✅ | Linux (function), **Mac (performance)** | Linux done: textures from pixel buffers, eviction, bounded memory and keyboard all work; see [spike 02](spikes/02-gpui-grid.md). **Mac:** 60/120 fps scroll, Retina texture size, unified-memory use. |
| **Card read throughput** | **Mac**, real card + reader | 1 vs 2 vs 4 parallel readers, to set the default I/O concurrency (§3.4). |
| **Affinity behaviour** | **Mac** | Where does Affinity's save dialog default to after `open -a` on a RAW? What name does it suggest? Does Quick Look thumbnail `.afphoto`? *Decides how reliable auto-stacking can be.* |
| **Card detection** | **Mac**, real SD reader | `/Volumes` + `DCIM/` detection and mount/unmount events. |

A macOS CI runner confirms the code builds and tests pass, but can't answer the hardware, Affinity, or smoothness questions.

Exit: every spike answered; architecture adjusted if any fail.

### Phase 1 — Foundation: index an existing folder (M)
- [ ] Cargo workspace + crates per §3, CI on Linux + macOS (fmt, clippy, tests).
- [ ] SQLite schema + migrations; WAL mode; single catalog-writer thread.
- [ ] Job system (§3.4): priority queues, CPU and per-device I/O pools, cancellation, de-duplication, progress reporting.
- [ ] `RawProvider` trait + CR3/NEF/JPEG/HEIF providers; `PreviewError` surfaced as placeholder tiles (§3.2).
- [ ] "Add library root" → background indexer: walk, read metadata, group RAW+JPEG, read existing XMP, generate thumbnails.
- [ ] Sidebar folder tree (`YYYY/MM/DD`) + thumbnail grid with flag/star badges.
- [ ] Fixture corpus of small CR3/NEF/JPEG/HEIF files for tests.

Exit: point it at an existing photo folder on the Mac; browsing feels instant.

### Phase 2 — Loupe + culling (L)
- [ ] Loupe view with prefetch/LRU and the progressive preview request (§3.2); fit and 100 % zoom; space to toggle; inline notice when a tier is unavailable.
- [ ] Keys: `P` pick, `X` reject, `U` unflag, `0–5` stars, arrows, auto-advance toggle.
- [ ] Undo/redo for rating changes.
- [ ] XMP write-through (debounced, atomic).
- [ ] 2-up compare (`C`): synced zoom/pan, swap/promote selection, keep ratings keys working in both panes.
- [ ] Filter bar: flag, min stars, camera, lens, file type.
- [ ] "Delete rejects…" → confirm dialog → Trash (whole group + sidecar) → catalog update.
- [ ] Perf instrumentation: log time-to-first-pixel per loupe navigation; assert first pixels < 50 ms p95 and log time-to-full-resolution on the dogfood Mac.

Exit: cull a real 1,000-shot folder faster than the author did in Lightroom.

### Phase 3 — Card import (L)
- [ ] Volume watcher (§4.1) → import window auto-opens.
- [ ] Streaming preview grid + duplicate dimming (§4.3).
- [ ] Copy/verify engine (§4.2), resumable, cancellable, collision-safe.
- [ ] Destination root setting; `YYYY/MM/DD` layout.
- [ ] Post-import: jump straight to the new shoot in grid view ready to cull.
- [ ] Test: import the same card twice → second run imports zero files. Import two cards with overlapping filenames → no overwrite.

Exit: **MVP criterion #1 met.**

### Phase 4 — Editor round-trip (M)
- [ ] Editor registry + Affinity presets + preferences UI.
- [ ] `⌘E` send to editor; edit_session tracking.
- [ ] Folder watcher → derived-file detection → stacking (§4.4).
- [ ] Stack UI: badge on thumbnail, expand/collapse, loupe shows the edited version by default (toggle original).
- [ ] Manual "Attach edited file…" fallback.

Exit: **MVP criterion #2 met.**

### Phase 5 — Collections + dogfood hardening (M)
- [ ] Collections: create, add/remove (drag + shortcut), sidebar section.
- [ ] External-change handling: files moved/deleted in Finder are detected on launch and via FSEvents; missing files flagged, not silently dropped.
- [ ] Catalog auto-backup (rolling SQLite copies) + "Rebuild catalog from disk + XMP".
- [ ] Crash/log capture to a local file.
- [ ] Two weeks of real use; triage everything that made you reach for Finder.

Exit: the author uses Mirrorless for every shoot for two weeks without falling back.

## 6. Post-MVP (ordered by likely value)

1. Second-destination backup during import (engine already supports it).
2. **Burst culling assistant** (§6.1) — the AI feature closest to the core culling value.
3. Lightroom Classic catalog import (`.lrcat` is SQLite): picks, ratings, labels, keywords, collections, folders. **Develop edits will not transfer** — be explicit about that in the UI; optionally offer to export LR's rendered previews as JPEGs.
4. Broader RAW support: new `RawProvider`s and Tier 2/3 rendering (§3.2) for Sony ARW, Fuji RAF, Olympus ORF, Panasonic RW2, DNG. Decide the provider registration / dependency-injection approach at this point.
5. More editor presets: Photoshop, darktable (gets RAW + `.CR3.xmp` naming quirk), Capture One, Pixelmator Pro; per-editor "send RAW vs send TIFF".
6. **Person detection and grouping** (§6.1).
7. Keywords/tags (`dc:subject`), color labels UI, metadata panel editing.
8. Survey view (N-up), focus-peaking overlay.
9. Lightroom (cloud) import via Adobe's export format.
10. Export presets (resize + watermark for sharing).
11. Linux as a supported target (darktable as primary editor), then Windows.
12. Map view, semantic search ("dog on a beach").
13. Packaging, signing, notarization, auto-update; decide license / business model.

### 6.1 AI features

Principles: **on-device only** (no uploads; the library is private), runs in the background job queue (§3.2), results stored in the catalog with model name and version so they can be recomputed, and **AI only suggests — it never deletes or rejects without the user confirming**.

Runtime: ONNX Runtime via the `ort` crate, using the CoreML backend on macOS (Apple Neural Engine/GPU) and CPU/CUDA elsewhere. Models are downloaded on first use, not bundled.

**Burst culling assistant**
- *Burst grouping (no ML):* consecutive frames from the same body within ~1 s (tunable), plus the maker's burst/sequence EXIF tags where present. Shown as collapsible stacks in the grid. Cheap enough that it could move into MVP Phase 2 as a stretch goal.
- *Per-frame scores:* overall sharpness (variance of Laplacian), **subject sharpness** (same, inside detected face/subject boxes; this is what actually decides most bursts), eyes open/closed, motion blur, highlight/shadow clipping, optionally an aesthetic-quality model.
- *UX:* "Review burst" opens compare/survey with frames ranked; the top frame is pre-marked as a suggested pick and the rest as suggested rejects. One key accepts, and the user can override any frame. Suggestions are visually distinct from real flags until accepted.
- Score on the best available preview tier — embedded full-res JPEGs make CR3/NEF ideal; formats without a large tier report `TierUnavailable` and are scored on what exists, or skipped with a visible reason.

**Person detection and grouping**
- Face detection → face embedding → clustering into unnamed people → the user names clusters → new photos are auto-suggested.
- "People" sidebar section; filter or collection by person. Names can be written to XMP (`mwg-rs` regions + `Iptc4xmpExt:PersonInImage`) so they survive outside the app.
- **Model licensing matters:** many popular face models (e.g. InsightFace weights) are non-commercial only. Prefer permissively licensed ones (e.g. YuNet / SFace from the OpenCV model zoo) so a future commercial decision isn't blocked.

## 7. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| gpui is pre-1.0, sparsely documented, API churns with Zed | High | Pin a Zed rev; isolate all UI in `mm-app`; Phase 0 grid spike before committing. |
| Affinity saves edits somewhere unpredictable → stacking fails | Medium | Phase 0 spike; stem-matching watcher; manual attach fallback. |
| Other makers (Sony, Fuji, …) embed small previews | Certain | Not built in MVP; `RawProvider` seam + typed `PreviewError` shown in the UI (§3.2) so support is additive later. |
| Concurrency bugs in the job system (races, deadlocks, starvation) | Medium | Single catalog writer; message passing instead of shared locks; headless stress tests that cancel and reprioritize randomly. |
| AI models are slow on large libraries or licensed for non-commercial use only | Medium | Background queue with visible-first priority; CoreML backend; vet model licenses before adopting. |
| CR3/NEF parsing gaps in a pure-Rust library | Medium | Spike both `rawler` and LibRaw; keep behind `mm-media` trait. |
| DB/XMP drift (another tool edits XMP) | Medium | Compare XMP mtime on access; XMP wins if newer. |
| Developing on Linux, dogfooding on Mac hides macOS-only bugs | Medium | macOS CI runner; weekly build on the Mac from Phase 1 onward. |

## 8. Confirmed defaults

1. **Card writes** — never (no delete-after-import). Revisit with the backup feature.
2. **Pick vs stars** — both are kept.
3. **Which Affinity** — detect and support both Affinity Photo 2 and the new unified Affinity app.
4. **Library location** — may be an external drive; an unplugged root shows as offline instead of erroring.
5. **Capture-time timezone** — camera local time as recorded.
6. **Lightroom edits** — will not transfer on catalog import; this is understood and accepted.
