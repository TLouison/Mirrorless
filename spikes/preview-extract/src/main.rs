//! Phase 0 spike: how fast can we get pixels out of CR3 / NEF / HEIF, and which
//! library (rawler vs LibRaw) should back the CR3/NEF `RawProvider`s?
//!
//! Usage: cargo run --release -- <samples-dir>

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rawler::decoders::RawDecodeParams;
use rawler::rawsource::RawSource;

const RUNS: usize = 5;

#[repr(C)]
struct MmThumb {
    data: *const u8,
    len: usize,
    width: c_int,
    height: c_int,
    is_jpeg: c_int,
    thumb_count: c_int,
    make: [c_char; 64],
    model: [c_char; 64],
    lens: [c_char; 128],
    serial: [c_char; 64],
    timestamp: i64,
    iso: f32,
    _img: *mut c_void,
    _lr: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct MmThumbItem {
    width: c_int,
    height: c_int,
    format: c_int,
    offset: u64,
    len: u64,
}

/// LibRaw_internal_thumbnail_formats::LIBRAW_INTERNAL_THUMBNAIL_JPEG
const LIBRAW_INTERNAL_THUMBNAIL_JPEG: c_int = 4;

unsafe extern "C" {
    fn mm_libraw_list(path: *const c_char, items: *mut MmThumbItem, max: c_int) -> c_int;
    fn mm_libraw_thumb(path: *const c_char, out: *mut MmThumb) -> c_int;
    fn mm_libraw_free(t: *mut MmThumb);
}

/// Owned LibRaw thumbnail; frees on drop.
struct LibrawThumb(MmThumb);

impl LibrawThumb {
    fn open(path: &Path) -> Result<Self, i32> {
        let c = CString::new(path.to_str().unwrap()).unwrap();
        let mut t: MmThumb = unsafe { std::mem::zeroed() };
        let rc = unsafe { mm_libraw_thumb(c.as_ptr(), &mut t) };
        let t = LibrawThumb(t);
        if rc != 0 { Err(rc) } else { Ok(t) }
    }
    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.0.data, self.0.len) }
    }
    fn s(field: &[c_char]) -> String {
        unsafe { CStr::from_ptr(field.as_ptr()) }.to_string_lossy().into_owned()
    }
}

impl Drop for LibrawThumb {
    fn drop(&mut self) {
        unsafe { mm_libraw_free(&mut self.0) }
    }
}

fn median<T>(mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut times = Vec::with_capacity(RUNS);
    let mut last = None;
    for _ in 0..RUNS {
        let t = Instant::now();
        let v = f();
        times.push(t.elapsed());
        last = Some(v);
    }
    times.sort();
    (times[RUNS / 2], last.unwrap())
}

fn ms(d: Duration) -> String {
    format!("{:7.1} ms", d.as_secs_f64() * 1000.0)
}

fn decode_jpeg(bytes: &[u8]) -> (usize, usize) {
    let mut d = zune_jpeg::JpegDecoder::new(Cursor::new(bytes));
    d.decode().expect("jpeg decode");
    let info = d.info().unwrap();
    (info.width as usize, info.height as usize)
}

fn list_previews(path: &Path) -> Vec<MmThumbItem> {
    let c = CString::new(path.to_str().unwrap()).unwrap();
    let mut items = [MmThumbItem::default(); 8];
    let n = unsafe { mm_libraw_list(c.as_ptr(), items.as_mut_ptr(), items.len() as c_int) };
    items[..n.max(0) as usize].to_vec()
}

/// What the app would do at view time: pread the stored byte range, no RAW parsing.
fn read_range(path: &Path, item: &MmThumbItem) -> Vec<u8> {
    use std::os::unix::fs::FileExt;
    let f = std::fs::File::open(path).unwrap();
    let mut buf = vec![0u8; item.len as usize];
    f.read_exact_at(&mut buf, item.offset).unwrap();
    buf
}

/// Reduced-size IDCT decode: asks for the smallest DCT scale (1/1, 1/2, 1/4, 1/8) >= target.
fn decode_scaled(bytes: &[u8], target_w: u16, target_h: u16) -> (u16, u16) {
    let mut d = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    d.read_info().unwrap();
    let (w, h) = d.scale(target_w, target_h).unwrap();
    d.decode().unwrap();
    (w, h)
}

fn raw_file(path: &Path) {
    let name = path.file_name().unwrap().to_string_lossy();
    println!("\n## {name}  ({:.1} MB)", std::fs::metadata(path).unwrap().len() as f64 / 1e6);
    let params = RawDecodeParams::default();

    // --- all embedded previews, located by LibRaw, read by byte range ---
    let (t_list, items) = median(|| list_previews(path));
    println!("libraw   locate   {}  | {} previews:", ms(t_list), items.len());
    for it in &items {
        let jpeg = it.format == LIBRAW_INTERNAL_THUMBNAIL_JPEG;
        print!("           {:>5}x{:<5} {:>8.2} MB  @{:<10} jpeg={jpeg}", it.width, it.height, it.len as f64 / 1e6, it.offset);
        if jpeg && it.len > 0 {
            let (t_read, bytes) = median(|| read_range(path, it));
            let ok = bytes.starts_with(&[0xFF, 0xD8]);
            let (t_full, _) = median(|| decode_jpeg(&bytes));
            print!("  read {} soi={ok}  zune-full {}", ms(t_read), ms(t_full));
            if it.width >= 2000 {
                let (t_fit, (w, h)) = median(|| decode_scaled(&bytes, 2560, 1600));
                let (t_thumb, (tw, th)) = median(|| decode_scaled(&bytes, 384, 256));
                print!("  | scaled {w}x{h} {}  | scaled {tw}x{th} {}", ms(t_fit), ms(t_thumb));
            }
        }
        println!();
    }

    // --- rawler ---
    let (t_meta, meta) = median(|| {
        let src = RawSource::new(path).unwrap();
        let dec = rawler::get_decoder(&src).unwrap();
        dec.raw_metadata(&src, &params)
    });
    match meta {
        Ok(m) => println!(
            "rawler   metadata {}  | {} {} | {:?}.{:?} | serial {:?} | img# {:?} | lens {:?}",
            ms(t_meta),
            m.make,
            m.model,
            m.exif.date_time_original,
            m.exif.sub_sec_time_original,
            m.exif.serial_number,
            m.exif.image_number,
            m.exif.lens_model,
        ),
        Err(e) => println!("rawler   metadata FAILED: {e}"),
    }
    for (label, which) in [("thumbnail", 0), ("preview  ", 1), ("full     ", 2)] {
        let (t, img) = median(|| {
            let src = RawSource::new(path).unwrap();
            let dec = rawler::get_decoder(&src).unwrap();
            match which {
                0 => dec.thumbnail_image(&src, &params),
                1 => dec.preview_image(&src, &params),
                _ => dec.full_image(&src, &params),
            }
        });
        match img {
            Ok(Some(i)) => println!("rawler   {label} {}  | {}x{} (decoded by rawler)", ms(t), i.width(), i.height()),
            Ok(None) => println!("rawler   {label}  -- none"),
            Err(e) => println!("rawler   {label}  FAILED: {e}"),
        }
    }

    // --- LibRaw ---
    let (t_extract, thumb) = median(|| LibrawThumb::open(path));
    match thumb {
        Ok(t) => {
            let ts = t.0.timestamp;
            println!(
                "libraw   extract  {}  | {}x{} jpeg={} {:.2} MB, {} thumbs in file | {} {} | ts {ts} | serial {:?} | lens {:?} | ISO {}",
                ms(t_extract),
                t.0.width,
                t.0.height,
                t.0.is_jpeg != 0,
                t.0.len as f64 / 1e6,
                t.0.thumb_count,
                LibrawThumb::s(&t.0.make),
                LibrawThumb::s(&t.0.model),
                LibrawThumb::s(&t.0.serial),
                LibrawThumb::s(&t.0.lens),
                t.0.iso,
            );
            if t.0.is_jpeg != 0 {
                let bytes = t.bytes().to_vec();
                let (t_dec, (w, h)) = median(|| decode_jpeg(&bytes));
                println!("zune     decode   {}  | {w}x{h} full decode of LibRaw's JPEG", ms(t_dec));
            }
        }
        Err(rc) => println!("libraw   extract  FAILED rc={rc}"),
    }
}

fn heif_file(path: &Path) {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};
    let name = path.file_name().unwrap().to_string_lossy();
    println!("\n## {name}  ({:.1} MB)", std::fs::metadata(path).unwrap().len() as f64 / 1e6);
    let lib = LibHeif::new();
    let (t_open, (w, h, n)) = median(|| {
        let ctx = HeifContext::read_from_file(path.to_str().unwrap()).unwrap();
        let hd = ctx.primary_image_handle().unwrap();
        (hd.width(), hd.height(), hd.number_of_thumbnails())
    });
    println!("libheif  open     {}  | {w}x{h}, {n} embedded thumbnails", ms(t_open));
    let (t_dec, res) = median(|| {
        let ctx = HeifContext::read_from_file(path.to_str().unwrap()).unwrap();
        let hd = ctx.primary_image_handle().unwrap();
        lib.decode(&hd, ColorSpace::Rgb(RgbChroma::Rgb), None).map(|_| ())
    });
    match res {
        Ok(()) => println!("libheif  decode   {}  | full primary image", ms(t_dec)),
        Err(e) => println!("libheif  decode   FAILED: {}", e.message),
    }
}

/// Simulate thumbnail jobs across pools of different sizes. Files are in the page
/// cache, so this is the CPU-bound ceiling; card read speed is measured on the Mac.
fn throughput(raws: &[PathBuf]) {
    use rayon::prelude::*;
    const REPS: usize = 6;
    let jobs: Vec<&PathBuf> = raws.iter().cycle().take(raws.len() * REPS).collect();
    type Job = fn(&Path);
    let variants: [(&str, Job); 2] = [
        ("A: largest preview, full decode", |p| {
            let th = LibrawThumb::open(p).unwrap();
            decode_jpeg(th.bytes());
        }),
        ("B: locate + read ~1620px preview + scaled decode to thumbnail", |p| {
            let items = list_previews(p);
            let it = items
                .iter()
                .filter(|i| i.format == LIBRAW_INTERNAL_THUMBNAIL_JPEG && i.width >= 1000)
                .min_by_key(|i| i.width)
                .expect("mid-size preview");
            decode_scaled(&read_range(p, it), 384, 256);
        }),
    ];
    for (label, job) in variants {
        println!("\n## Throughput {label}, {} jobs", jobs.len());
        for threads in [1, 2, 4] {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            let t = Instant::now();
            pool.install(|| jobs.par_iter().for_each(|p| job(p)));
            let per_s = jobs.len() as f64 / t.elapsed().as_secs_f64();
            println!("threads {threads}: {per_s:7.1} files/s  -> 1000-file card in {:6.1} s", 1000.0 / per_s);
        }
    }
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: preview-extract <samples-dir>");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let ext = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let raws: Vec<PathBuf> = files.iter().filter(|p| matches!(ext(p).as_str(), "cr3" | "nef")).cloned().collect();
    if let Some(out) = std::env::args().nth(2) {
        // `preview-extract <samples> <out-dir>`: dump each RAW's ~1620px embedded JPEG for the gpui grid spike.
        std::fs::create_dir_all(&out).unwrap();
        for p in &raws {
            let it = *list_previews(p)
                .iter()
                .filter(|i| i.format == LIBRAW_INTERNAL_THUMBNAIL_JPEG && i.width >= 1000)
                .min_by_key(|i| i.width)
                .unwrap();
            let dest = Path::new(&out).join(p.file_stem().unwrap()).with_extension("jpg");
            std::fs::write(&dest, read_range(p, &it)).unwrap();
            println!("wrote {}", dest.display());
        }
        return;
    }
    println!("Median of {RUNS} runs per measurement; files are in the page cache (warm).");
    for p in &raws {
        raw_file(p);
    }
    for p in files.iter().filter(|p| matches!(ext(p).as_str(), "heic" | "heif" | "hif")) {
        heif_file(p);
    }
    throughput(&raws);
}
