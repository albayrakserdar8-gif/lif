//! Frame time and bytes for typical updates on a 200x50 client.
//!
//! cargo test -p lif-render --release --test bench -- --ignored --nocapture

use lif_render::{Renderer, Scene, ScenePane, StatusBar, StatusPosition};
use lif_types::{Attrs, Cell, Color, OuterCaps, PaneFrame, Rect, Size, Span, Style, StyledLine};
use std::hint::black_box;
use std::time::{Duration, Instant};

const COLS: u16 = 200;
const ROWS: u16 = 50;

fn caps() -> OuterCaps {
    OuterCaps {
        truecolor: true,
        sync_output: true,
        styled_underline: true,
        underline_color: true,
        hyperlinks: true,
        overline: true,
        ..OuterCaps::default()
    }
}

/// A line of words that differs from the lines around it, like real
/// output does.
fn line(n: usize, width: usize) -> String {
    const WORDS: [&str; 16] = [
        "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "pack", "my", "box",
        "with", "five", "dozen", "liquor", "jugs",
    ];
    let mut s = format!("{n:06} ");
    let mut x = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    while s.len() < width {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.push_str(WORDS[(x % 16) as usize]);
        s.push(if x.is_multiple_of(7) { ',' } else { ' ' });
    }
    s.truncate(width);
    s
}

fn fill_row(f: &mut PaneFrame, y: usize, n: usize, sgr_heavy: bool) {
    let w = f.size.cols as usize;
    let text = line(n, w);
    for (x, ch) in text.chars().enumerate() {
        let style = if sgr_heavy {
            let k = (x * 7 + n * 13) as u8;
            Style {
                fg: Color::Rgb(k, k.wrapping_mul(3), k.wrapping_add(90)),
                bg: if x % 3 == 0 {
                    Color::Indexed(k % 16)
                } else {
                    Color::Default
                },
                attrs: if x % 5 == 0 { Attrs::BOLD } else { Attrs::NONE },
                ..Style::default()
            }
        } else {
            Style::default()
        };
        f.rows[y].cells[x] = Cell {
            ch,
            style,
            ..Cell::BLANK
        };
    }
    f.dirty[y] = true;
}

fn scroll(f: &mut PaneFrame, k: usize, next: &mut usize) {
    let rows = f.rows.len();
    f.rows.rotate_left(k);
    for y in rows - k..rows {
        *next += 1;
        fill_row(f, y, *next, false);
    }
    f.mark_all_dirty();
}

struct Result {
    name: &'static str,
    per_frame: Duration,
    bytes: usize,
}

fn report(r: &Result) {
    eprintln!(
        "{:<52} {:>10.2} us/frame {:>8} bytes/frame",
        r.name,
        r.per_frame.as_secs_f64() * 1e6,
        r.bytes
    );
}

#[test]
#[ignore]
fn bench_frames() {
    let status: StyledLine = vec![
        Span::new("[lif] 0:zsh* 1:vim", Style::default()),
        Span::plain("  12:00"),
    ];
    let full = Rect::new(0, 0, COLS, ROWS - 1);
    let new_frame = |r: Rect, heavy: bool| {
        let mut f = PaneFrame::new(r.size());
        for y in 0..r.rows as usize {
            fill_row(&mut f, y, y, heavy);
        }
        f
    };
    let mut results = Vec::new();

    // Drives one full-width pane through `frames` frames.
    let run1 = |name: &'static str,
                frames: usize,
                opt: bool,
                f: &mut PaneFrame,
                mutate: &mut dyn FnMut(&mut PaneFrame, usize)| {
        let mut r = Renderer::new(caps());
        r.set_scroll_optimization(opt);
        let mut out = Vec::with_capacity(1 << 20);
        let mut total = Duration::ZERO;
        let mut bytes = 0;
        for i in 0..=frames {
            if i > 0 {
                mutate(f, i);
            }
            let panes = [ScenePane {
                id: 1,
                rect: full,
                frame: f,
                active: true,
                selection: None,
                matches: &[],
                indicator: None,
                base: Default::default(),
            }];
            let mut scene = Scene::new(Size::new(COLS, ROWS), &panes);
            scene.status = Some(StatusBar {
                line: &status,
                fill: Style {
                    bg: Color::Indexed(2),
                    ..Style::default()
                },
                position: StatusPosition::Bottom,
            });
            out.clear();
            let t = Instant::now();
            r.render(&scene, &mut out);
            let dt = t.elapsed();
            black_box(&out);
            if i > 0 {
                total += dt;
                bytes += out.len();
            }
            f.clear_dirty();
        }
        Result {
            name,
            per_frame: total / frames as u32,
            bytes: bytes / frames,
        }
    };

    let mut f = new_frame(full, false);
    results.push(run1(
        "nothing changed",
        100_000,
        true,
        &mut f,
        &mut |_, _| {},
    ));
    results.push(run1(
        "one cell changed",
        100_000,
        true,
        &mut f,
        &mut |f, i| {
            f.rows[20].cells[50].ch = if i % 2 == 0 { 'x' } else { 'y' };
            f.dirty[20] = true;
        },
    ));
    results.push(run1(
        "one row changed (200 cells)",
        50_000,
        true,
        &mut f,
        &mut |f, i| fill_row(f, 20, 1000 + i, false),
    ));
    let mut next = 100_000;
    results.push(run1(
        "full-width pane scrolled 1 line, scroll opt",
        20_000,
        true,
        &mut f,
        &mut |f, _| scroll(f, 1, &mut next),
    ));
    results.push(run1(
        "full-width pane scrolled 1 line, no scroll opt",
        20_000,
        false,
        &mut f,
        &mut |f, _| scroll(f, 1, &mut next),
    ));
    results.push(run1(
        "full-width pane scrolled 10 lines, scroll opt",
        10_000,
        true,
        &mut f,
        &mut |f, _| scroll(f, 10, &mut next),
    ));
    results.push(run1(
        "full-width pane scrolled 10 lines, no scroll opt",
        10_000,
        false,
        &mut f,
        &mut |f, _| scroll(f, 10, &mut next),
    ));
    results.push(run1(
        "every cell changed, plain ASCII",
        5_000,
        true,
        &mut f,
        &mut |f, i| {
            for y in 0..f.rows.len() {
                fill_row(f, y, i * 7 + y * 31 + 500, false);
            }
        },
    ));
    let mut g = new_frame(full, true);
    results.push(run1(
        "every cell changed, SGR-heavy",
        2_000,
        true,
        &mut g,
        &mut |f, i| {
            for y in 0..f.rows.len() {
                fill_row(f, y, i * 7 + y * 31 + 500, true);
            }
        },
    ));

    // Full repaint of a fresh client: 200x50 = 10,000 cells.
    {
        let f = new_frame(Rect::new(0, 0, COLS, ROWS), false);
        let panes = [ScenePane {
            id: 1,
            rect: Rect::new(0, 0, COLS, ROWS),
            frame: &f,
            active: true,
            selection: None,
                matches: &[],
            indicator: None,
            base: Default::default(),
        }];
        let scene = Scene::new(Size::new(COLS, ROWS), &panes);
        let mut r = Renderer::new(caps());
        let mut out = Vec::with_capacity(1 << 20);
        let n = 5_000;
        let mut total = Duration::ZERO;
        for _ in 0..n {
            out.clear();
            let t = Instant::now();
            r.render_full(&scene, &mut out);
            total += t.elapsed();
            black_box(&out);
        }
        results.push(Result {
            name: "full repaint, 10,000 plain cells",
            per_frame: total / n,
            bytes: out.len(),
        });
        let blank = PaneFrame::new(Size::new(COLS, ROWS));
        let panes = [ScenePane {
            id: 1,
            rect: Rect::new(0, 0, COLS, ROWS),
            frame: &blank,
            active: true,
            selection: None,
                matches: &[],
            indicator: None,
            base: Default::default(),
        }];
        let scene = Scene::new(Size::new(COLS, ROWS), &panes);
        let mut total = Duration::ZERO;
        for _ in 0..n {
            out.clear();
            let t = Instant::now();
            r.render_full(&scene, &mut out);
            total += t.elapsed();
            black_box(&out);
        }
        results.push(Result {
            name: "full repaint, blank screen (compose + compare)",
            per_frame: total / n,
            bytes: out.len(),
        });
        let g = new_frame(Rect::new(0, 0, COLS, ROWS), true);
        let panes = [ScenePane {
            id: 1,
            rect: Rect::new(0, 0, COLS, ROWS),
            frame: &g,
            active: true,
            selection: None,
                matches: &[],
            indicator: None,
            base: Default::default(),
        }];
        let scene = Scene::new(Size::new(COLS, ROWS), &panes);
        let mut total = Duration::ZERO;
        for _ in 0..n {
            out.clear();
            let t = Instant::now();
            r.render_full(&scene, &mut out);
            total += t.elapsed();
            black_box(&out);
        }
        results.push(Result {
            name: "full repaint, 10,000 SGR-heavy cells",
            per_frame: total / n,
            bytes: out.len(),
        });
    }

    // Four panes (2x2), the top-left one scrolling by a line per frame.
    for (opt, name) in [
        (true, "4 panes, one scrolling 1 line, lr margins"),
        (false, "4 panes, one scrolling 1 line, repaint"),
    ] {
        let (w0, h0) = (COLS / 2, (ROWS - 1) / 2);
        let rects = [
            Rect::new(0, 0, w0, h0),
            Rect::new(w0 + 1, 0, COLS - w0 - 1, h0),
            Rect::new(0, h0 + 1, w0, ROWS - 1 - h0 - 1),
            Rect::new(w0 + 1, h0 + 1, COLS - w0 - 1, ROWS - 1 - h0 - 1),
        ];
        let mut frames: Vec<PaneFrame> = rects.iter().map(|r| new_frame(*r, false)).collect();
        let mut r = Renderer::new(OuterCaps { lr_margins: opt, ..caps() });
        let mut out = Vec::with_capacity(1 << 20);
        let (mut total, mut bytes, n) = (Duration::ZERO, 0, 20_000);
        let mut next = 0;
        for i in 0..=n {
            if i > 0 {
                scroll(&mut frames[0], 1, &mut next);
            }
            let panes: Vec<ScenePane> = rects
                .iter()
                .zip(&frames)
                .enumerate()
                .map(|(k, (r, f))| ScenePane {
                    id: k as u64,
                    rect: *r,
                    frame: f,
                    active: k == 0,
                    selection: None,
                matches: &[],
                    indicator: None,
                    base: Default::default(),
                })
                .collect();
            let mut scene = Scene::new(Size::new(COLS, ROWS), &panes);
            scene.status = Some(StatusBar {
                line: &status,
                fill: Style::default(),
                position: StatusPosition::Bottom,
            });
            out.clear();
            let t = Instant::now();
            r.render(&scene, &mut out);
            let dt = t.elapsed();
            black_box(&out);
            if i > 0 {
                total += dt;
                bytes += out.len();
            }
            for f in &mut frames {
                f.clear_dirty();
            }
        }
        results.push(Result {
            name,
            per_frame: total / n as u32,
            bytes: bytes / n,
        });
    }

    eprintln!();
    for r in &results {
        report(r);
    }
}
