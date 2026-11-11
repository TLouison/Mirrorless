//! Phase 0 spike 05 (Linux half): full-resolution JPEG decode, zune-jpeg vs libjpeg-turbo 3.
//!
//! Inputs are the full-res JPEGs embedded in our CR3/NEF samples (extract with
//! `exiftool -b -JpgFromRaw`). Both decoders output BGRA, which is what gpui uploads.
//! libjpeg-turbo is also measured with DCT scaling (1/2, 1/4, 1/8), the "fit to screen" path.
//! The Mac run adds Apple ImageIO.
//!
//! Usage: cargo run --release -- <file.jpg>...

use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};

use turbojpeg::{Decompressor, Image, PixelFormat, ScalingFactor};
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;

const RUNS: usize = 5;

fn median(mut f: impl FnMut()) -> Duration {
    let mut times: Vec<_> = (0..RUNS)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    times.sort();
    times[RUNS / 2]
}

fn zune(bytes: &[u8]) -> usize {
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::BGRA);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(bytes), opts);
    d.decode().unwrap().len()
}

fn turbo(d: &mut Decompressor, bytes: &[u8], scale: ScalingFactor) -> (usize, usize) {
    d.set_scaling_factor(scale).unwrap();
    let h = d.read_header(bytes).unwrap().scaled(scale);
    let mut img = Image { pixels: vec![0u8; 4 * h.width * h.height], width: h.width, pitch: 4 * h.width, height: h.height, format: PixelFormat::BGRA };
    d.decompress(bytes, img.as_deref_mut()).unwrap();
    (h.width, h.height)
}

fn ms(d: Duration) -> String {
    format!("{:6.1}", d.as_secs_f64() * 1e3)
}

fn main() {
    let mut d = Decompressor::new().unwrap();
    println!("Median of {RUNS} runs, ms; BGRA output; warm cache.\n");
    println!("| file | size | MP | zune full | turbo full | turbo 1/2 | turbo 1/4 | turbo 1/8 |");
    println!("|---|---|---|---|---|---|---|---|");
    for a in std::env::args().skip(1) {
        let bytes = std::fs::read(&a).unwrap();
        let (w, h) = turbo(&mut d, &bytes, ScalingFactor::ONE);
        let tz = median(|| { zune(&bytes); });
        let cols: Vec<String> = [ScalingFactor::ONE, ScalingFactor::ONE_HALF, ScalingFactor::ONE_QUARTER, ScalingFactor::ONE_EIGHTH]
            .into_iter()
            .map(|s| {
                let t = median(|| { turbo(&mut d, &bytes, s); });
                let (sw, sh) = turbo(&mut d, &bytes, s);
                if s == ScalingFactor::ONE { ms(t) } else { format!("{} ({sw}×{sh})", ms(t)) }
            })
            .collect();
        println!(
            "| {} | {:.1} MB | {:.0} | {} | {} |",
            Path::new(&a).file_stem().unwrap().to_string_lossy(),
            bytes.len() as f64 / 1e6,
            (w * h) as f64 / 1e6,
            ms(tz),
            cols.join(" | ")
        );
    }
}
