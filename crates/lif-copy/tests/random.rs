//! Random command, key, mouse and content sequences keep copy mode's
//! invariants and never panic.

mod common;
use common::*;
use lif_copy::{Command, Config, CopyMode, Enter, MemoryView, ModeKeys, Outcome, PaneView, SelectionShape};
use lif_types::{Cell, CellWidth, Mods, MouseAction, MouseButton, MouseEvent, Pos};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<'a, T>(&mut self, s: &'a [T]) -> &'a T {
        &s[self.below(s.len())]
    }
}

const PIECES: &[&str] = &[
    "a", "b", "word", " ", "  ", "-", ".", "(", ")", "[", "]", "{", "}", "漢", "字", "テ", "👍🏽", "👨\u{200d}👩\u{200d}👧", "e\u{301}",
    "\u{308}", "x", "_", "/", "$ ", "0", "9", "Z", "\t",
];

fn line(r: &mut Rng) -> String {
    let n = match r.below(10) {
        0 => 0,
        1 => 40 + r.below(80),
        _ => r.below(12),
    };
    let mut s = String::new();
    for _ in 0..n {
        let p = *r.pick(PIECES);
        s.push_str(if p == "\t" { "    " } else { p });
    }
    s
}

fn make_view(r: &mut Rng) -> MemoryView {
    let cols = 1 + r.below(30) as u16;
    let rows = 1 + r.below(10) as u16;
    let n = r.below(60);
    let lines: Vec<String> = (0..n).map(|_| line(r)).collect();
    let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    MemoryView::with_prompts(cols, rows, &refs, "$ ")
}

fn check(m: &mut CopyMode, v: &MemoryView, what: &str) {
    let size = v.size();
    let (cols, rows) = (size.cols.max(1) as usize, size.rows.max(1) as usize);
    let total = v.history_rows() + rows;
    let c = m.cursor();
    assert!(c.row < total, "{what}: cursor row {c:?} / {total}");
    assert!(c.col < cols, "{what}: cursor col {c:?} / {cols}");
    if let Some(row) = v.rows.get(c.row)
        && let Some(cell) = row.cells.get(c.col)
    {
        assert_ne!(cell.width, CellWidth::WideTail, "{what}: cursor on a tail at {c:?}");
    }
    assert!(m.top() <= v.history_rows(), "{what}: top");
    assert!(c.row >= m.top() && c.row < m.top() + rows, "{what}: cursor not visible {c:?} top {}", m.top());
    assert_eq!(v.viewport_top, m.top(), "{what}: viewport not applied");
    if let Some((a, b, _)) = m.selection() {
        for p in [a, b] {
            assert!(p.row < total && p.col < cols, "{what}: selection end {p:?}");
        }
    }
    let _ = m.selection_text(v);
    let o = m.overlays(v);
    assert!((o.cursor.x as usize) < cols && (o.cursor.y as usize) < rows, "{what}: overlay cursor");
    if let Some(s) = o.selection {
        for p in [s.start, s.end] {
            assert!((p.x as usize) < cols && (p.y as usize) < rows, "{what}: overlay selection {s:?}");
        }
        if s.shape == SelectionShape::Stream {
            assert!((s.start.y, s.start.x) <= (s.end.y, s.end.x), "{what}: {s:?}");
        }
    }
    for sp in o.matches {
        assert!((sp.y as usize) < rows && sp.x0 <= sp.x1 && (sp.x1 as usize) < cols, "{what}: span {sp:?}");
    }
}

fn mutate(r: &mut Rng, v: &mut MemoryView) {
    match r.below(4) {
        0 => {
            let l = line(r);
            v.push_line(&l);
        }
        1 => {
            let n = r.below(5);
            v.trim(n);
        }
        2 => {
            let cols = 1 + r.below(30) as u16;
            for row in &mut v.rows {
                row.cells.resize(cols as usize, Cell::BLANK);
                if let Some(last) = row.cells.last_mut()
                    && last.width == CellWidth::Wide
                {
                    *last = Cell::BLANK;
                }
            }
            v.size.cols = cols;
        }
        _ => {
            v.size.rows = 1 + r.below(10) as u16;
        }
    }
}

fn run_one(seed: u64, steps: usize) {
    let mut r = Rng(seed);
    let mut v = make_view(&mut r);
    let keys = if r.below(2) == 0 { ModeKeys::Vi } else { ModeKeys::Emacs };
    let mut cfg = Config::new(keys);
    cfg.search_step_rows = 1 + r.below(50);
    cfg.wrap_search = r.below(3) != 0;
    let table: Vec<String> = cfg.table.iter().map(|(k, _)| k.to_string()).collect();
    let cur = Pos { x: r.below(40) as u16, y: r.below(12) as u16 };
    let enter = Enter { page_up: r.below(4) == 0, exit_at_bottom: r.below(2) == 0, hide_position: false };
    let mut m = CopyMode::new(&mut v, cfg, cur, enter);
    check(&mut m, &v, "new");
    let mut now = 0u64;
    for step in 0..steps {
        let what;
        let out = match r.below(12) {
            0..=3 => {
                let cmd = *r.pick(Command::ALL);
                let arg = match cmd {
                    Command::GotoLine => Some(r.below(100).to_string()),
                    c if c.is_jump() => Some(PIECES[r.below(PIECES.len())].chars().next().unwrap().to_string()),
                    c if c.takes_argument() => Some(line(&mut r).chars().take(1 + r.below(4)).collect()),
                    _ => None,
                };
                let n = if r.below(10) == 0 { 1 + r.below(200) as u32 } else { 1 + r.below(3) as u32 };
                what = format!("{seed}/{step}: {} {n} {arg:?}", cmd.name());
                m.run_command(&mut v, cmd, arg.as_deref(), n)
            }
            4..=6 => {
                let k = if r.below(5) == 0 {
                    ch(PIECES[r.below(PIECES.len())].chars().next().unwrap())
                } else {
                    key(r.pick(&table))
                };
                what = format!("{seed}/{step}: key {:?}", k.key);
                m.handle_key(&k, &mut v)
            }
            7 | 8 => {
                now += r.below(400) as u64;
                let action = *r.pick(&[MouseAction::Press, MouseAction::Motion, MouseAction::Release]);
                let button = *r.pick(&[
                    Some(MouseButton::Left),
                    Some(MouseButton::Left),
                    Some(MouseButton::WheelUp),
                    Some(MouseButton::WheelDown),
                    None,
                ]);
                let e = MouseEvent {
                    action,
                    button,
                    mods: Mods::NONE,
                    cell: Pos { x: r.below(35) as u16, y: r.below(12) as u16 },
                    pixel: None,
                };
                what = format!("{seed}/{step}: mouse {e:?}");
                let o = m.handle_mouse(&e, now, &mut v);
                if r.below(3) == 0 {
                    m.autoscroll_tick(&mut v);
                }
                o
            }
            9 => {
                what = format!("{seed}/{step}: prompt");
                match r.below(4) {
                    0 => {
                        let t: String = line(&mut r).chars().take(r.below(4)).collect();
                        m.prompt_changed(&mut v, &t)
                    }
                    1 => m.prompt_again(&mut v, r.below(2) == 0),
                    2 => {
                        let t: String = line(&mut r).chars().take(r.below(4)).collect();
                        m.prompt_accept(&mut v, &t)
                    }
                    _ => m.prompt_cancel(&mut v),
                }
            }
            10 => {
                what = format!("{seed}/{step}: search step");
                m.search_step(&mut v, 1 + r.below(30))
            }
            _ => {
                what = format!("{seed}/{step}: mutate");
                mutate(&mut r, &mut v);
                m.refresh(&mut v);
                Outcome::Redraw
            }
        };
        if let Outcome::Copy { data, .. } = &out {
            assert!(std::str::from_utf8(data).is_ok());
        }
        check(&mut m, &v, &what);
    }
    m.leave(&mut v);
    assert_eq!(v.viewport_top, v.history_rows());
}

#[test]
fn random_sequences_keep_invariants() {
    // LIF_COPY_FUZZ_SEEDS=n runs a longer campaign.
    let seeds: u64 = std::env::var("LIF_COPY_FUZZ_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    for seed in 1..=seeds {
        run_one(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1, 400);
    }
}
