//! Phase 0 spike 06 (Linux half): detect a camera card being mounted / unmounted.
//!
//! Approach: the kernel flags /proc/self/mountinfo with POLLPRI whenever the mount table
//! changes. On each change, diff the table; a new mount point that contains `DCIM/` is a
//! card. This sees every mount (udisks, fstab, manual), not just /media/$USER, and it can't
//! race the way watching /media/$USER for new directories can: udisks creates the mount
//! point directory *before* mounting, so an inotify watcher sees an empty folder.
//!
//! Also logs, per new mount, the sysfs `removable` flag of the backing disk — to check
//! whether "removable" is usable as a filter (built-in SD slots often report 0).
//!
//! Usage: cargo run --release   (then insert/eject a card; Ctrl-C to stop)

use std::collections::BTreeMap;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Debug)]
struct Mount {
    dev: String, // major:minor
    fstype: String,
    source: String,
}

/// Mount point -> info. mountinfo escapes spaces etc. as \040-style octal.
fn mounts() -> BTreeMap<PathBuf, Mount> {
    let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap();
    text.lines()
        .filter_map(|l| {
            let (pre, post) = l.split_once(" - ")?;
            let pre: Vec<&str> = pre.split(' ').collect();
            let post: Vec<&str> = post.split(' ').collect();
            Some((PathBuf::from(unescape(pre.get(4)?)), Mount { dev: pre.get(2)?.to_string(), fstype: post.first()?.to_string(), source: post.get(1)?.to_string() }))
        })
        .collect()
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 4], 8) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// sysfs `removable` for the whole disk backing a partition (e.g. sdb1 -> sdb).
fn removable(dev: &str) -> String {
    let sys = Path::new("/sys/dev/block").join(dev);
    let Ok(real) = std::fs::canonicalize(&sys) else { return "n/a".into() };
    for p in [real.join("removable"), real.parent().unwrap().join("removable")] {
        if let Ok(v) = std::fs::read_to_string(&p) {
            return format!("{} ({})", v.trim(), p.parent().unwrap().file_name().unwrap().to_string_lossy());
        }
    }
    "n/a".into()
}

/// Card check + a quick enumeration of DCIM/** to see how long the metadata-only walk takes.
fn inspect(root: &Path, t0: Instant) {
    let dcim = root.join("DCIM");
    if !dcim.is_dir() {
        println!("{:>8.1} ms   not a card (no DCIM/)", ms(t0));
        return;
    }
    let t = Instant::now();
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut stack = vec![dcim];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let m = e.metadata().unwrap();
            if m.is_dir() { stack.push(e.path()) } else { files += 1; bytes += m.len() }
        }
    }
    println!(
        "{:>8.1} ms   CARD: {files} files, {:.1} GB under DCIM/ (walk took {:.1} ms)",
        ms(t0), bytes as f64 / 1e9, t.elapsed().as_secs_f64() * 1e3
    );
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn main() {
    let f = File::open("/proc/self/mountinfo").unwrap();
    let mut known = mounts();
    println!("watching mount table ({} mounts). Existing mounts with DCIM/:", known.len());
    for (p, _) in known.iter().filter(|(p, _)| p.join("DCIM").is_dir()) {
        println!("  {}", p.display());
    }
    loop {
        let mut pfd = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLPRI, revents: 0 };
        let rc = unsafe { libc::poll(&mut pfd, 1, -1) };
        assert!(rc >= 0, "poll failed");
        let t0 = Instant::now();
        let now = mounts();
        for (p, m) in now.iter().filter(|(p, _)| !known.contains_key(*p)) {
            println!(
                "\n{:>8.1} ms + MOUNT {}  [{} {} dev {} removable={}]",
                ms(t0), p.display(), m.fstype, m.source, m.dev, removable(&m.dev)
            );
            inspect(p, t0);
        }
        for (p, m) in known.iter().filter(|(p, _)| !now.contains_key(*p)) {
            println!("\n{:>8.1} ms - UNMOUNT {}  [{} {}]", ms(t0), p.display(), m.fstype, m.source);
        }
        known = now;
    }
}
