//! Throughput and refresh cost. Run in release:
//! cargo test --release -p lif-vt --test bench -- --ignored --nocapture

use std::hint::black_box;
use std::time::Instant;

use lif_types::{PaneFrame, Size};
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
