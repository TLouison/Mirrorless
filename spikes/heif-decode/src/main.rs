//! Phase 0 spike 04: can the HEIF provider decode real HEIC on Linux, and how fast?
//!
//! Needs an HEVC decoder plugin (libheif-plugin-libde265). Run with
//! LIBHEIF_PLUGIN_PATH pointing at it if it isn't installed system-wide.
//!
//! Usage: cargo run --release -- <file.heic>...

use std::path::Path;
use std::time::{Duration, Instant};

use libheif_rs::{ColorSpace, HeifContext, ImageHandle, LibHeif, RgbChroma};

const RUNS: usize = 5;

fn median<T>(mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..RUNS {
        let t = Instant::now();
        last = Some(f());
        times.push(t.elapsed());
    }
    times.sort();
    (times[RUNS / 2], last.unwrap())
}

fn ms(d: Duration) -> String {
    format!("{:7.1} ms", d.as_secs_f64() * 1e3)
}

fn decode(lib: &LibHeif, h: &ImageHandle) -> Result<(u32, u32), String> {
    lib.decode(h, ColorSpace::Rgb(RgbChroma::Rgba), None)
        .map(|img| (img.width(), img.height()))
        .map_err(|e| e.message)
}

fn file(lib: &LibHeif, path: &Path) {
    let p = path.to_str().unwrap();
    println!("\n## {}  ({:.1} MB)", path.file_name().unwrap().to_string_lossy(), std::fs::metadata(path).unwrap().len() as f64 / 1e6);
    let (t_open, ctx) = median(|| HeifContext::read_from_file(p).unwrap());
    let h = ctx.primary_image_handle().unwrap();
    println!(
        "open        {}  | {}x{} luma {} bit, {} thumbnail(s), {} top-level images",
        ms(t_open), h.width(), h.height(), h.luma_bits_per_pixel(), h.number_of_thumbnails(), ctx.image_ids().len()
    );
    let mut ids = vec![0; h.number_of_thumbnails()];
    h.thumbnail_ids(&mut ids);
    for id in ids {
        let th = h.thumbnail(id).unwrap();
        let (t, r) = median(|| decode(lib, &th));
        println!("thumbnail   {}  | {r:?}", ms(t));
    }
    for threads in [1, 2, 4] {
        let (t, r) = median(|| {
            let mut ctx = HeifContext::read_from_file(p).unwrap();
            ctx.set_max_decoding_threads(threads);
            let h = ctx.primary_image_handle().unwrap();
            decode(lib, &h)
        });
        println!("primary     {}  | {threads} tile thread(s) -> {r:?}", ms(t));
    }
}

fn main() {
    let lib = LibHeif::new();
    let decoders = lib.decoder_descriptors(8, None);
    println!("libheif {:?}; decoders: {:?}", lib.version(), decoders.iter().map(|d| d.id().to_string()).collect::<Vec<_>>());
    println!("Median of {RUNS} runs; warm page cache; output RGBA 8-bit.");
    for a in std::env::args().skip(1) {
        file(&lib, Path::new(&a));
    }
}
