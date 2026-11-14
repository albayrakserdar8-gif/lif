//! Micro-benchmarks. Run with
//! \`cargo test -p lif-copy --release --test bench -- --ignored --nocapture\`.

mod common;
use common::*;
use lif_copy::{Command, Config, CopyMode, DEFAULT_SEARCH_STEP_ROWS, Enter, MemoryView, ModeKeys, Outcome};
use lif_types::Pos;
use std::time::Instant;

fn big_view() -> MemoryView {
    let lines: Vec<String> = (0..10_000)
        .map(|i| format!("{i:05} the quick brown fox jumps over the lazy dog; [ok] path/to/file.rs:{i}"))
        .collect();
    let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    MemoryView::from_lines(80, 24, &refs)
}

fn per_op(name: &str, n: u32, f: impl FnMut(u32)) {
    let mut f = f;
    let t = Instant::now();
    for i in 0..n {
        f(i);
    }
    let ns = t.elapsed().as_nanos() as f64 / n as f64;
    println!("{name:<40} {ns:>10.0} ns/op");
}

#[test]
#[ignore]
fn bench() {
    let mut v = big_view();
    let mut m = CopyMode::new(&mut v, Config::new(ModeKeys::Vi), Pos { x: 0, y: 23 }, Enter::default());
    m.run_command(&mut v, Command::HalfpageUp, None, 1);
    per_op("cursor-up/down", 1_000_000, |i| {
        let c = if i % 2 == 0 { Command::CursorUp } else { Command::CursorDown };
        m.run_command(&mut v, c, None, 1);
    });
    per_op("cursor-left/right", 1_000_000, |i| {
        let c = if i % 2 == 0 { Command::CursorRight } else { Command::CursorLeft };
        m.run_command(&mut v, c, None, 1);
    });
    per_op("next-word", 1_000_000, |i| {
        let c = if i % 1000 == 999 { Command::HistoryBottom } else { Command::NextWord };
        m.run_command(&mut v, c, None, 1);
    });
    per_op("page-up / page-down", 1_000_000, |i| {
        let c = if (i / 100) % 2 == 0 { Command::PageUp } else { Command::PageDown };
        m.run_command(&mut v, c, None, 1);
    });
    per_op("handle_key j", 1_000_000, |i| {
        let k = if i % 2 == 0 { key("j") } else { key("k") };
        m.handle_key(&k, &mut v);
    });
    m.run_command(&mut v, Command::BeginSelection, None, 1);
    m.run_command(&mut v, Command::HalfpageUp, None, 1);
    m.run_command(&mut v, Command::SearchBackward, Some("fox"), 1);
    per_op("overlays (selection + matches)", 100_000, |_| {
        let o = m.overlays(&v);
        assert!(!o.matches.is_empty());
    });

    // Searching a million rows that do not match.
    for (name, pat, regex) in [("literal", "needle", false), ("regex", "ne+dle|haystack[0-9]", true)] {
        let mut s = Synth::new(1_000_000, None);
        let mut m = CopyMode::new(&mut s, Config::new(ModeKeys::Vi), Pos { x: 0, y: 23 }, Enter::default());
        let cmd = if regex { Command::SearchBackward } else { Command::SearchBackwardText };
        let t = Instant::now();
        let mut worst = 0u128;
        let mut steps = 1;
        let t0 = Instant::now();
        let mut o = m.run_command(&mut s, cmd, Some(pat), 1);
        worst = worst.max(t0.elapsed().as_micros());
        while o == Outcome::Searching {
            let t1 = Instant::now();
            o = m.search_step(&mut s, DEFAULT_SEARCH_STEP_ROWS);
            worst = worst.max(t1.elapsed().as_micros());
            steps += 1;
        }
        println!(
            "search 1M rows ({name}): {:>6} ms total, {steps} steps of {DEFAULT_SEARCH_STEP_ROWS} rows, worst step {worst} us",
            t.elapsed().as_millis()
        );
    }
}
