//! Spike: prove libghostty-vt links statically from Rust, measure parse
//! throughput, dirty-row tracking and memory.
use libghostty_vt::render::{CellIterator, RowIterator};
use libghostty_vt::{RenderState, Terminal};
use std::time::Instant;

fn main() {
    let mut term = Terminal::new(120, 40).unwrap();
    term.set_scrollback_max_bytes(Some(10 * 1024 * 1024)).unwrap();
    let mut state = RenderState::new().unwrap();
    let mut rows = RowIterator::new().unwrap();
    let mut cells = CellIterator::new().unwrap();

    // 1. Basic round trip.
    term.vt_write(b"hello \x1b[1;38;2;255;100;0mworld\x1b[0m \xe4\xb8\xad\r\nline2");
    {
        let snap = state.update(&term).unwrap();
        println!("dirty={:?} cursor={:?}", snap.dirty().unwrap(), snap.cursor_viewport().unwrap());
        let mut it = rows.update(&snap).unwrap();
        let mut s = String::new();
        while let Some((y, row)) = it.next_dirty() {
            s.clear();
            let mut ci = cells.update(row).unwrap();
            while let Some(c) = ci.next() {
                c.graphemes_utf8(&mut s).unwrap();
            }
            let t = s.trim_end();
            if !t.is_empty() {
                println!("row {y}: {t:?}");
            }
        }
        drop(it);
        snap.clean().unwrap();
    }

    // 2. One-line change => how many dirty rows?
    term.vt_write(b"\x1b[5;1Hx");
    {
        let snap = state.update(&term).unwrap();
        let mut it = rows.update(&snap).unwrap();
        let mut n = 0;
        while it.next_dirty().is_some() {
            n += 1;
        }
        drop(it);
        println!("dirty rows after 1-cell write: {n} ({:?})", snap.dirty().unwrap());
        snap.clean().unwrap();
    }

    // 3. Throughput: plain ASCII lines and SGR-heavy lines.
    let mut ascii = Vec::new();
    let mut sgr = Vec::new();
    for i in 0..200_000u32 {
        ascii.extend_from_slice(format!("{i:08} the quick brown fox jumps over the lazy dog 0123456789 abcdefghijklmnopqrstuvwxyz\r\n").as_bytes());
        sgr.extend_from_slice(format!("\x1b[38;2;{};{};{}m{i:08}\x1b[0m \x1b[1;4;3{}mstyled\x1b[0m text \u{2713} \u{4e2d}\u{6587}\r\n", i % 256, (i / 3) % 256, (i / 7) % 256, i % 8).as_bytes());
    }
    for (name, data) in [("ascii", &ascii), ("sgr+utf8", &sgr)] {
        let t = Instant::now();
        // Feed in 64 KiB chunks like a PTY reader would.
        for chunk in data.chunks(64 * 1024) {
            term.vt_write(chunk);
        }
        let dt = t.elapsed();
        println!(
            "{name}: {:.1} MB in {:.1} ms = {:.0} MB/s",
            data.len() as f64 / 1e6,
            dt.as_secs_f64() * 1e3,
            data.len() as f64 / 1e6 / dt.as_secs_f64()
        );
    }
    println!(
        "total_rows={} scrollback_rows={}",
        term.total_rows().unwrap(),
        term.scrollback_rows().unwrap()
    );
    println!("memory_usage={:?}", term.memory_usage().unwrap());
}
