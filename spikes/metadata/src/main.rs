//! Phase 0 spike 03: where does the import fingerprint come from?
//!
//! Fingerprint (roadmap §4.3) = capture time incl. subseconds + body serial + file size.
//! Compares LibRaw (one open, EXIF callback) against two pure-Rust EXIF readers on speed,
//! correctness (checked against ExifTool by hand) and bytes read from disk — the last one
//! matters most, because on import the file is on an SD card.
//!
//! Usage: cargo run --release -- <file>...

use std::ffi::{CStr, CString, c_char, c_int};
use std::path::Path;
use std::time::{Duration, Instant};

const RUNS: usize = 7;

#[repr(C)]
struct MmMeta {
    make: [c_char; 64],
    model: [c_char; 64],
    serial: [c_char; 64],
    dto: [c_char; 24],
    subsec: [c_char; 8],
    offset: [c_char; 8],
    exif_serial: [c_char; 64],
    timestamp: i64,
    shot_order: u32,
    preview_count: c_int,
}

unsafe extern "C" {
    fn mm_meta_read(path: *const c_char, out: *mut MmMeta) -> c_int;
}

fn s(f: &[c_char]) -> String {
    unsafe { CStr::from_ptr(f.as_ptr()) }.to_string_lossy().trim().to_string()
}

/// Bytes this process has asked the kernel for (`rchar`), page cache or not. With a cold
/// cache on a card this is what actually crosses the reader.
fn rchar() -> u64 {
    let io = std::fs::read_to_string("/proc/self/io").unwrap();
    io.lines().find_map(|l| l.strip_prefix("rchar: ")).unwrap().parse().unwrap()
}

/// Median time over RUNS, plus bytes read and the result of the last run.
fn measure<T>(mut f: impl FnMut() -> T) -> (Duration, u64, T) {
    let mut times = Vec::new();
    let mut bytes = 0;
    let mut last = None;
    for _ in 0..RUNS {
        let b0 = rchar();
        let t = Instant::now();
        let v = f();
        times.push(t.elapsed());
        bytes = rchar() - b0;
        last = Some(v);
    }
    times.sort();
    (times[RUNS / 2], bytes, last.unwrap())
}

fn row(lib: &str, t: Duration, bytes: u64, out: String) {
    println!("  {lib:<8} {:6.2} ms {:>9} read | {out}", t.as_secs_f64() * 1e3, human(bytes));
}

fn human(b: u64) -> String {
    if b >= 1 << 20 { format!("{:.1} MB", b as f64 / 1048576.0) } else { format!("{:.0} KB", b as f64 / 1024.0) }
}

fn libraw(path: &Path) {
    let c = CString::new(path.to_str().unwrap()).unwrap();
    let (t, b, r) = measure(|| {
        let mut m: MmMeta = unsafe { std::mem::zeroed() };
        let rc = unsafe { mm_meta_read(c.as_ptr(), &mut m) };
        (rc, m)
    });
    let (rc, m) = r;
    if rc != 0 {
        return row("libraw", t, b, format!("FAILED rc={rc}"));
    }
    row(
        "libraw",
        t,
        b,
        format!(
            "{} {} | {}.{} {} | serial {:?} (exif {:?}) | shot_order {} | {} previews",
            s(&m.make), s(&m.model), s(&m.dto), s(&m.subsec), s(&m.offset),
            s(&m.serial), s(&m.exif_serial), m.shot_order, m.preview_count
        ),
    );
}

fn nom(path: &Path) {
    use nom_exif::ExifTag as T;
    let (t, b, r) = measure(|| nom_exif::read_exif(path));
    match r {
        Ok(e) => {
            let g = |tag| e.get(tag).map(|v| v.to_string()).unwrap_or_else(|| "-".into());
            row(
                "nom-exif",
                t,
                b,
                format!(
                    "{} | {}.{} {} | serial {}",
                    g(T::Model), g(T::DateTimeOriginal), g(T::SubSecTimeOriginal), g(T::OffsetTimeOriginal), g(T::CameraSerialNumber)
                ),
            );
        }
        Err(e) => row("nom-exif", t, b, format!("FAILED: {e}")),
    }
}

fn kamadak(path: &Path) {
    use exif::{In, Tag};
    let (t, b, r) = measure(|| {
        let f = std::fs::File::open(path).unwrap();
        exif::Reader::new().read_from_container(&mut std::io::BufReader::new(f))
    });
    match r {
        Ok(e) => {
            let g = |tag| e.get_field(tag, In::PRIMARY).map(|f| f.display_value().to_string()).unwrap_or_else(|| "-".into());
            row(
                "kamadak",
                t,
                b,
                format!(
                    "{} | {}.{} {} | serial {}",
                    g(Tag::Model), g(Tag::DateTimeOriginal), g(Tag::SubSecTimeOriginal), g(Tag::OffsetTimeOriginal), g(Tag::BodySerialNumber)
                ),
            );
        }
        Err(e) => row("kamadak", t, b, format!("FAILED: {e}")),
    }
}

fn main() {
    println!("Median of {RUNS} runs; warm page cache. 'read' = bytes requested from the kernel per file.");
    for a in std::env::args().skip(1) {
        let p = Path::new(&a);
        let size = std::fs::metadata(p).unwrap().len();
        println!("\n## {}  ({:.1} MB)", p.file_name().unwrap().to_string_lossy(), size as f64 / 1e6);
        let ext = p.extension().unwrap().to_string_lossy().to_lowercase();
        if matches!(ext.as_str(), "cr3" | "nef") {
            libraw(p);
        }
        nom(p);
        kamadak(p);
    }
}
