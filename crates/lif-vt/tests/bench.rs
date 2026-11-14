//! Throughput and refresh cost. Run in release:
//! cargo test --release -p lif-vt --test bench -- --ignored --nocapture

use std::hint::black_box;
use std::time::Instant;

use lif_types::{FrameRow, PaneFrame, Size};
use lif_vt::{GhosttyVt, Scroll, VtEngine, VtOptions};

fn data() -> (Vec<u8>, Vec<u8>) {
    let mut ascii = Vec::new();
    let mut sgr = Vec::new();
    for i in 0..200_000u32 {
        ascii.extend_from_slice(
            format!("{i:08} the quick brown fox jumps over the lazy dog 0123456789 abcdefghijklmnopqrstuvwxyz\r\n")
                .as_bytes(),
        );
        sgr.extend_from_slice(
            format!(
                "\x1b[38;2;{};{};{}m{i:08}\x1b[0m \x1b[1;4;3{}mstyled\x1b[0m text \u{2713} \u{4e2d}\u{6587}\r\n",
                i % 256,
                (i / 3) % 256,
                (i / 7) % 256,
                i % 8
            )
            .as_bytes(),
        );
    }
    (ascii, sgr)
}

fn feed_mbps(data: &[u8]) -> f64 {
    let mut best = 0f64;
    for _ in 0..3 {
        let mut v = GhosttyVt::new(&VtOptions { size: Size::new(120, 40), ..Default::default() })
            .unwrap();
        let t = Instant::now();
        for chunk in data.chunks(64 * 1024) {
            v.feed(chunk);
        }
        let mbps = data.len() as f64 / 1e6 / t.elapsed().as_secs_f64();
        best = best.max(mbps);
    }
    best
}

fn raw_mbps(data: &[u8]) -> f64 {
    let mut best = 0f64;
    for _ in 0..3 {
        let mut t0 = libghostty_vt::Terminal::new(120, 40).unwrap();
        t0.set_scrollback_max_bytes(Some(10 << 20)).unwrap();
        let t = Instant::now();
        for chunk in data.chunks(64 * 1024) {
            t0.vt_write(chunk);
        }
        best = best.max(data.len() as f64 / 1e6 / t.elapsed().as_secs_f64());
    }
    best
}

fn per_iter_us(n: u32, mut f: impl FnMut()) -> f64 {
    for _ in 0..n / 10 {
        f();
    }
    let t = Instant::now();
    for _ in 0..n {
        f();
    }
    t.elapsed().as_secs_f64() * 1e6 / f64::from(n)
}

/// A copy-mode search reads every row in turn (`VtEngine::row_scan`).
#[test]
#[ignore]
fn bench_scan() {
    for (cols, mib) in [(80u16, 10usize), (80, 300), (200, 100)] {
        let mut v = GhosttyVt::new(&VtOptions { size: Size::new(cols, 24), scrollback_bytes: mib << 20, ..Default::default() })
            .unwrap();
        let mut s = Vec::new();
        for i in 0..400_000u32 {
            s.extend_from_slice(format!("{i}\r\n").as_bytes());
        }
        for chunk in s.chunks(64 * 1024) {
            v.feed(chunk);
        }
        let h = v.scrollback_rows();
        let mut r = FrameRow::default();
        let top = v.viewport_top();
        let t = Instant::now();
        for y in (0..h).rev() {
            v.row_scan(y, &mut r);
            black_box(&r);
        }
        let back = t.elapsed();
        v.set_viewport_top(top);
        let t = Instant::now();
        for y in (0..h).step_by(97) {
            v.row(y, &mut r);
            black_box(&r);
        }
        let random = t.elapsed().as_secs_f64() * 1e6 / (h / 97) as f64;
        println!(
            "scan {cols} cols, {mib} MiB ({h} rows): row_scan backward {:.0} ms ({:.2} us/row); row() scattered {random:.1} us/row",
            back.as_secs_f64() * 1e3,
            back.as_secs_f64() * 1e6 / h as f64
        );
    }
}

/// Copy mode reads scrollback rows one at a time (`VtEngine::row`).
#[test]
#[ignore]
fn bench_rows() {
    for (cols, mib) in [(80u16, 10usize), (200, 100)] {
        let mut v = GhosttyVt::new(&VtOptions { size: Size::new(cols, 50), scrollback_bytes: mib << 20, ..Default::default() })
            .unwrap();
        let mut s = Vec::new();
        for i in 0..2_000_000u32 {
            s.extend_from_slice(
                format!("\x1b[3{}m{i:08}\x1b[0m the quick brown fox jumps over the lazy dog \u{4e2d}\u{6587}\r\n", i % 8)
                    .as_bytes(),
            );
        }
        for chunk in s.chunks(64 * 1024) {
            v.feed(chunk);
        }
        let h = v.scrollback_rows();
        let mut r = FrameRow::default();
        for (name, compact) in [("plain", false), ("compressed", true)] {
            for (where_, y) in [("oldest", 3usize), ("middle", h / 2), ("newest", h - 1)] {
                if compact {
                    for _ in 0..100_000 {
                        v.compact();
                    }
                }
                // The first read of a compressed page restores it.
                let t = Instant::now();
                v.row(y, &mut r);
                let first = t.elapsed().as_secs_f64() * 1e6;
                let us = per_iter_us(2_000, || {
                    v.row(black_box(y), &mut r);
                    black_box(&r);
                });
                // Copy mode keeps the viewport on the rows it shows.
                v.set_viewport_top(y.saturating_sub(5));
                let shown = per_iter_us(2_000, || {
                    v.row(black_box(y), &mut r);
                    black_box(&r);
                });
                v.set_viewport_top(h);
                println!(
                    "row() {cols} cols, {mib} MiB scrollback ({h} rows, {} pages compressed), {name:10} {where_:6}: first {first:7.1} us, then {us:6.2} us/row, {shown:6.2} us/row when in the viewport",
                    v.memory().compressed_pages
                );
            }
        }
    }
}

#[test]
#[ignore]
fn bench() {
    let (ascii, sgr) = data();
    println!(
        "feed ascii:    {:7.0} MB/s (raw libghostty vt_write {:7.0} MB/s), {:.1} MB",
        feed_mbps(&ascii),
        raw_mbps(&ascii),
        ascii.len() as f64 / 1e6
    );
    println!(
        "feed sgr+utf8: {:7.0} MB/s (raw libghostty vt_write {:7.0} MB/s), {:.1} MB",
        feed_mbps(&sgr),
        raw_mbps(&sgr),
        sgr.len() as f64 / 1e6
    );

    let size = Size::new(200, 50);
    for (name, styled) in [("plain", false), ("sgr", true)] {
        let mut v = GhosttyVt::new(&VtOptions { size, ..Default::default() }).unwrap();
        // 100 lines: 50 of scrollback plus a full screen.
        for y in 0..100u32 {
            let mut line = String::new();
            for x in 0..40u32 {
                if styled {
                    line.push_str(&format!("\x1b[38;5;{}m", (x * 7 + y) % 256));
                }
                line.push_str("ab\u{4e2d}d");
            }
            v.feed(format!("{line}\x1b[0m\r\n").as_bytes());
        }
        let mut f = PaneFrame::new(size);
        v.refresh(&mut f);

        let idle = per_iter_us(10_000, || {
            black_box(v.refresh(&mut f));
        });
        let mut i = 0u32;
        let one = per_iter_us(10_000, || {
            i += 1;
            v.feed(if i.is_multiple_of(2) { b"\x1b[25;7Hx" } else { b"\x1b[25;7Hy" });
            black_box(v.refresh(&mut f));
        });
        f.clear_dirty();
        v.feed(b"\x1b[25;7Hz");
        v.refresh(&mut f);
        assert_eq!(f.dirty.iter().filter(|d| **d).count(), 1);
        // Moving the viewport by one line rewrites every row.
        let mut j = 0u32;
        let all = per_iter_us(2_000, || {
            j += 1;
            v.scroll(Scroll::Lines(if j.is_multiple_of(2) { 1 } else { -1 }));
            black_box(v.refresh(&mut f));
        });
        f.clear_dirty();
        v.scroll(Scroll::Lines(-1));
        v.refresh(&mut f);
        assert!(f.dirty.iter().all(|d| *d));
        println!(
            "refresh 200x50 {name:5}: idle {idle:6.2} us, 1 dirty row {one:6.2} us, all rows {all:7.1} us"
        );
    }
}
