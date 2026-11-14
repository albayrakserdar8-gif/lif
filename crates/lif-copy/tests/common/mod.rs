#![allow(dead_code)]

use lif_copy::{Command, Config, CopyMode, Enter, MemoryView, ModeKeys, Outcome, PaneView};
use lif_types::{Key, KeyEvent, Mods, Pos};

/// A pane laid out like tmux's after \`printf '%s' "$(cat file)"\` in a
/// fresh \`cols\` x \`rows\` pane: the lines, the cursor right after the last
/// character, and copy mode entered there.
pub fn pane(cols: u16, rows: u16, text: &str, keys: ModeKeys) -> (MemoryView, CopyMode) {
    let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    let mut v = MemoryView::from_lines(cols, rows, &lines);
    let last = lines.len() - 1;
    // Cursor after the last character of the last line.
    let mut last_row = 0;
    let mut n = 0;
    for (i, r) in v.rows.iter().enumerate() {
        if n == last {
            last_row = i;
            if !r.wrapped {
                break;
            }
        } else if !r.wrapped {
            n += 1;
        }
    }
    let x = v.rows[last_row].cells.iter().rposition(|c| c.ch != ' ' || c.extra != 0).map_or(0, |p| p + 1);
    let y = last_row - v.history_rows();
    let m = CopyMode::new(&mut v, Config::new(keys), Pos { x: x.min(cols as usize - 1) as u16, y: y as u16 }, Enter::default());
    (v, m)
}

/// Run \`send-keys -X [-N n] name [arg]\` written as "name arg" or "N name arg".
pub fn x(m: &mut CopyMode, v: &mut MemoryView, s: &str) -> Outcome {
    let mut parts = s.splitn(2, ' ');
    let mut first = parts.next().unwrap_or("");
    let mut rest = parts.next();
    let mut n = 1;
    if let Ok(k) = first.parse::<u32>() {
        n = k;
        let mut p = rest.unwrap_or("").splitn(2, ' ');
        first = p.next().unwrap_or("");
        rest = p.next();
    }
    let cmd = Command::from_name(first).unwrap_or_else(|| panic!("unknown command {first}"));
    let mut out = m.run_command(v, cmd, rest, n);
    while out == Outcome::Searching {
        out = m.search_step(v, 1_000_000);
    }
    out
}

/// (cursor column, cursor row on screen, scroll offset) as tmux's
/// \`#{copy_cursor_x},#{copy_cursor_y},#{scroll_position}\`.
pub fn st(m: &CopyMode) -> (usize, usize, usize) {
    let c = m.cursor();
    (c.col, c.row - m.top(), m.position().0)
}

/// Run each step and compare the state after it.
pub fn script(m: &mut CopyMode, v: &mut MemoryView, steps: &[(&str, (usize, usize, usize))]) {
    for (i, (s, want)) in steps.iter().enumerate() {
        x(m, v, s);
        assert_eq!(st(m), *want, "step {i}: {s}");
        assert_eq!(v.viewport_top, m.top(), "viewport follows after {s}");
    }
}

/// The copied text of an outcome.
pub fn copied(o: &Outcome) -> String {
    match o {
        Outcome::Copy { data, .. } => String::from_utf8(data.clone()).unwrap(),
        other => panic!("expected a copy, got {other:?}"),
    }
}

/// Copy the selection without clearing it.
pub fn yank(m: &mut CopyMode, v: &mut MemoryView) -> String {
    copied(&x(m, v, "copy-selection-no-clear"))
}

pub fn key(name: &str) -> KeyEvent {
    lif_core::KeyCode::parse(name).unwrap_or_else(|| panic!("bad key {name}")).to_event()
}

pub fn ch(c: char) -> KeyEvent {
    let mut e = KeyEvent::new(Key::Char(c), Mods::NONE);
    e.text.push(c);
    e
}

pub fn keys(m: &mut CopyMode, v: &mut MemoryView, names: &[&str]) -> Outcome {
    let mut last = Outcome::None;
    for n in names {
        last = m.handle_key(&key(n), v);
        while last == Outcome::Searching {
            last = m.search_step(v, 1_000_000);
        }
    }
    last
}

/// A huge synthetic pane whose rows are generated on demand: "log <n>
/// lorem ipsum ..." on every row, "needle in a haystack" on \`needle\`.
pub struct Synth {
    pub cols: u16,
    pub screen: u16,
    pub total: usize,
    pub needle: Option<usize>,
    pub top: usize,
    pub reads: std::cell::Cell<u64>,
}

impl Synth {
    pub fn new(total: usize, needle: Option<usize>) -> Synth {
        Synth { cols: 80, screen: 24, total, needle, top: total - 24, reads: std::cell::Cell::new(0) }
    }
}

impl PaneView for Synth {
    fn size(&self) -> lif_types::Size {
        lif_types::Size::new(self.cols, self.screen)
    }

    fn history_rows(&self) -> usize {
        self.total - self.screen as usize
    }

    fn row(&self, row: usize, out: &mut lif_types::FrameRow) {
        self.reads.set(self.reads.get() + 1);
        out.cells.clear();
        out.cells.resize(self.cols as usize, lif_types::Cell::BLANK);
        out.clusters.clear();
        out.wrapped = false;
        let mut x = 0;
        let mut put = |c: char| {
            if x < out.cells.len() {
                out.cells[x].ch = c;
                x += 1;
            }
        };
        if self.needle == Some(row) {
            "needle in a haystack".chars().for_each(&mut put);
            return;
        }
        "log ".chars().for_each(&mut put);
        let mut digits = [0u8; 20];
        let mut n = row;
        let mut k = 0;
        loop {
            digits[k] = (n % 10) as u8;
            k += 1;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        for d in digits[..k].iter().rev() {
            put((b'0' + d) as char);
        }
        " lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod".chars().for_each(&mut put);
    }

    fn set_viewport_top(&mut self, top: usize) {
        self.top = top;
    }
}
