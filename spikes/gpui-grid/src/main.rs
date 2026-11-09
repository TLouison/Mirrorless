//! Phase 0 spike: can gpui show a 10k-thumbnail grid from raw pixel buffers with
//! bounded GPU memory, keyboard navigation, and smooth scrolling?
//!
//! Usage:
//!   gpui-grid <preview-jpeg-dir>              interactive
//!   MM_BENCH=1 gpui-grid <preview-jpeg-dir>   auto-scroll to the bottom, print stats, quit

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, FocusHandle, ImageSource, KeyBinding, Keystroke, ObjectFit, RenderImage,
    ScrollStrategy, UniformListScrollHandle, Window, WindowBounds, WindowOptions, actions, div,
    img, point, prelude::*, px, rgb, size, uniform_list,
};
use gpui_platform::application;
use image::{Frame, RgbaImage};
use smallvec::smallvec;

const COUNT: usize = 10_000;
/// Texture size: 2x the on-screen cell so thumbnails stay sharp on Retina.
const TEX_W: u32 = 384;
const TEX_H: u32 = 256;
const CELL_W: f32 = 200.0;
const CELL_H: f32 = 140.0;
/// Max thumbnails held as GPU textures; older ones are dropped from the atlas.
const CACHE_CAP: usize = 500;
const BENCH_SPEED_PX_PER_FRAME: f32 = 60.0;

actions!(grid, [Left, Right, Up, Down, Quit]);

#[derive(Default)]
struct Stats {
    created: usize,
    dropped: usize,
    frame_ms: Vec<f64>,
    last_frame: Option<Instant>,
    load_ms: Vec<f64>,
}

struct Grid {
    /// Decoded sample thumbnails in BGRA, which each cell tints to get a unique texture.
    bases: Arc<Vec<RgbaImage>>,
    cache: HashMap<usize, Arc<RenderImage>>,
    lru: VecDeque<usize>,
    pending: HashSet<usize>,
    visible: Range<usize>,
    selected: usize,
    cols: usize,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    stats: Stats,
    bench: Option<Instant>,
    keys_checked: bool,
}

/// Simulates a thumbnail job: unique BGRA buffer per index (tinted copy of a sample).
fn make_thumb(bases: &[RgbaImage], ix: usize) -> Arc<RenderImage> {
    let mut buf = bases[ix % bases.len()].clone();
    let tint = ((ix * 37) % 96) as u8;
    for (i, px) in buf.pixels_mut().enumerate() {
        // A colored band whose position encodes the index, so every texture differs.
        if (i as u32 / TEX_W) < 12 && (i as u32 % TEX_W) < (ix % 100) as u32 * 3 + 20 {
            px.0 = [40, 200, 255, 255];
        } else {
            px.0[2] = px.0[2].saturating_add(tint / 3);
        }
    }
    Arc::new(RenderImage::new(smallvec![Frame::new(buf)]))
}

fn load_bases(dir: &PathBuf) -> Vec<RgbaImage> {
    let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    paths
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "jpg"))
        .map(|p| {
            let rgba = image::open(p).unwrap().to_rgba8();
            let mut thumb = image::imageops::thumbnail(&rgba, TEX_W, TEX_H);
            for px in thumb.pixels_mut() {
                px.0.swap(0, 2); // RGBA -> BGRA, which RenderImage expects
            }
            thumb
        })
        .collect()
}

impl Grid {
    fn rows(&self) -> usize {
        COUNT.div_ceil(self.cols)
    }

    fn touch(&mut self, ix: usize) {
        if let Some(pos) = self.lru.iter().position(|&i| i == ix) {
            self.lru.remove(pos);
        }
        self.lru.push_back(ix);
    }

    fn insert(&mut self, ix: usize, image: Arc<RenderImage>, cx: &mut App) {
        self.pending.remove(&ix);
        self.cache.insert(ix, image);
        self.touch(ix);
        self.stats.created += 1;
        while self.cache.len() > CACHE_CAP {
            // Evict the least recently shown thumbnail that isn't on screen.
            let Some(pos) = self.lru.iter().position(|i| !self.visible.contains(i)) else { break };
            let victim = self.lru.remove(pos).unwrap();
            if let Some(old) = self.cache.remove(&victim) {
                cx.drop_image(old, None);
                self.stats.dropped += 1;
            }
        }
    }

    fn request(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.cache.contains_key(&ix) || !self.pending.insert(ix) {
            return;
        }
        let bases = self.bases.clone();
        let t0 = Instant::now();
        cx.spawn(async move |this, cx| {
            let image = cx.background_spawn(async move { make_thumb(&bases, ix) }).await;
            this.update(cx, |this, cx| {
                this.stats.load_ms.push(t0.elapsed().as_secs_f64() * 1e3);
                this.insert(ix, image, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_row(&mut self, row: usize, cx: &mut Context<Self>) -> gpui::Div {
        let start = row * self.cols;
        let end = (start + self.cols).min(COUNT);
        let mut cells = Vec::with_capacity(self.cols);
        for ix in start..end {
            let content = match self.cache.get(&ix).cloned() {
                Some(image) => {
                    self.touch(ix);
                    img(ImageSource::Render(image)).size_full().object_fit(ObjectFit::Cover).into_any_element()
                }
                None => {
                    self.request(ix, cx);
                    div().size_full().bg(rgb(0x2a2a2a)).into_any_element()
                }
            };
            cells.push(
                div()
                    .w(px(CELL_W))
                    .h(px(CELL_H))
                    .p_1()
                    .border_2()
                    .border_color(if ix == self.selected { rgb(0xffcc00) } else { rgb(0x1a1a1a) })
                    .child(content),
            );
        }
        div().flex().flex_row().h(px(CELL_H)).children(cells)
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.selected = (self.selected as isize + delta).clamp(0, COUNT as isize - 1) as usize;
        self.scroll.scroll_to_item(self.selected / self.cols, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn bench_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(started) = self.bench else { return };
        if !self.keys_checked {
            // Real keystrokes through the keymap -> focus -> action path. Must run outside
            // render, once the dispatch tree from a completed frame exists.
            if self.stats.frame_ms.len() < 3 {
                window.request_animation_frame();
                return;
            }
            self.keys_checked = true;
            let this = cx.entity();
            let expected = 1 + self.cols;
            window.on_next_frame(move |window, cx| {
                for key in ["right", "right", "down", "left"] {
                    window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
                }
                let selected = this.read(cx).selected;
                println!(
                    "keyboard: selected={selected} after right,right,down,left (expected {expected}) -> {}",
                    if selected == expected { "OK" } else { "FAIL" }
                );
            });
        }
        let handle = self.scroll.0.borrow().base_handle.clone();
        let max = handle.max_offset().y;
        let y = handle.offset().y - px(BENCH_SPEED_PX_PER_FRAME);
        if max > px(0.) && -y >= max {
            self.report(started.elapsed());
            cx.quit();
            return;
        }
        handle.set_offset(point(px(0.), y));
        window.request_animation_frame();
    }

    fn report(&self, total: Duration) {
        let pct = |v: &[f64], p: f64| {
            let mut v = v.to_vec();
            v.sort_by(f64::total_cmp);
            v.get(((v.len() as f64 - 1.0) * p) as usize).copied().unwrap_or(0.0)
        };
        let f = &self.stats.frame_ms[self.stats.frame_ms.len().min(5)..]; // skip warm-up frames
        let rss = std::fs::read_to_string("/proc/self/status")
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("VmHWM") || l.starts_with("VmRSS"))
            .collect::<Vec<_>>()
            .join("  ");
        println!("bench: scrolled {} rows x {} cols in {:.1}s, {} frames", self.rows(), self.cols, total.as_secs_f64(), f.len());
        println!(
            "frame ms: mean {:.1}  p50 {:.1}  p95 {:.1}  p99 {:.1}  max {:.1}",
            f.iter().sum::<f64>() / f.len() as f64,
            pct(f, 0.5),
            pct(f, 0.95),
            pct(f, 0.99),
            pct(f, 1.0)
        );
        let l = &self.stats.load_ms;
        println!("thumb request->ready ms: p50 {:.1}  p95 {:.1}  max {:.1}", pct(l, 0.5), pct(l, 0.95), pct(l, 1.0));
        println!(
            "textures: created {}  dropped {}  resident {} (cap {CACHE_CAP}) ≈ {:.0} MB of {}x{} BGRA",
            self.stats.created,
            self.stats.dropped,
            self.cache.len(),
            (self.cache.len() * (TEX_W * TEX_H * 4) as usize) as f64 / 1e6,
            TEX_W,
            TEX_H
        );
        println!("memory: {rss}");
    }
}

impl Render for Grid {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if let Some(last) = self.stats.last_frame.replace(now) {
            self.stats.frame_ms.push((now - last).as_secs_f64() * 1e3);
        }
        self.cols = ((window.viewport_size().width / px(CELL_W)).floor() as usize).max(1);
        self.bench_step(window, cx);

        let header = format!(
            "{COUNT} photos · {} cols · selected #{} · textures {} resident / {} created / {} dropped",
            self.cols,
            self.selected,
            self.cache.len(),
            self.stats.created,
            self.stats.dropped
        );
        div()
            .track_focus(&self.focus)
            .key_context("Grid")
            .on_action(cx.listener(|this, _: &Left, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &Right, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &Up, _, cx| this.move_selection(-(this.cols as isize), cx)))
            .on_action(cx.listener(|this, _: &Down, _, cx| this.move_selection(this.cols as isize, cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1a1a1a))
            .font_family("IBM Plex Sans")
            .text_color(rgb(0xdddddd))
            .child(div().p_2().child(header))
            .child(
                uniform_list(
                    "grid",
                    self.rows(),
                    cx.processor(|this, range: Range<usize>, _window, cx| {
                        this.visible = range.start * this.cols..(range.end * this.cols).min(COUNT);
                        range.map(|row| this.render_row(row, cx)).collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1(),
            )
    }
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: gpui-grid <preview-jpeg-dir>"));
    let t = Instant::now();
    let bases = Arc::new(load_bases(&dir));
    assert!(!bases.is_empty(), "no .jpg files in {}", dir.display());
    println!("loaded {} sample thumbnails in {:.0} ms", bases.len(), t.elapsed().as_secs_f64() * 1e3);
    let bench = std::env::var("MM_BENCH").is_ok();

    application().run(move |cx: &mut App| {
        // Bundle the UI font instead of relying on whatever the OS has installed.
        cx.text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
                "/home/todd/src/zed-d2a4fb6/assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"
            ))])
            .unwrap();
        cx.bind_keys([
            KeyBinding::new("left", Left, Some("Grid")),
            KeyBinding::new("right", Right, Some("Grid")),
            KeyBinding::new("up", Up, Some("Grid")),
            KeyBinding::new("down", Down, Some("Grid")),
            KeyBinding::new("ctrl-q", Quit, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    Grid {
                        bases,
                        cache: HashMap::new(),
                        lru: VecDeque::new(),
                        pending: HashSet::new(),
                        visible: 0..0,
                        selected: 0,
                        cols: 1,
                        scroll: UniformListScrollHandle::new(),
                        focus,
                        stats: Stats::default(),
                        bench: bench.then(Instant::now),
                        keys_checked: !bench,
                    }
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
