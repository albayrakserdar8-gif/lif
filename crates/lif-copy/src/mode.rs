//! The copy-mode state machine.

use crate::Point;
use crate::command::Command;
use crate::grid::{Grid, Reader, RowCache, WHITESPACE};
use crate::keys::{Action, KeyTable, ModeKeys};
use crate::regex::Scratch;
use crate::search::{Job, LineText, Match, MatchSpan, Pattern, Step, viewport_matches};
use crate::view::PaneView;
use lif_core::KeyCode;
use lif_types::{Key, KeyAction, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent, Pos};
use std::fmt::Write as _;
use std::rc::Rc;

/// tmux's default `word-separators`.
pub const DEFAULT_WORD_SEPARATORS: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^\x60{|}~";

/// Rows a search examines per call by default: about 1 ms of work for a
/// literal pattern on 80-column rows (M-series, see tests/bench.rs).
pub const DEFAULT_SEARCH_STEP_ROWS: usize = 2_000;

/// Furthest a bracket match looks, in rows.
const BRACKET_ROWS: usize = 10_000;

/// What happens when a mouse drag ends (and after a double or triple
/// click): tmux's `MouseDragEnd1Pane` binding, which lif sets with the
/// `mouse-drag-end` option.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DragEnd {
    /// Copy and leave copy mode (tmux's default, `copy-pipe-and-cancel`).
    #[default]
    CopyAndCancel,
    /// Copy the selection and stay in copy mode with it still shown
    /// (`copy-pipe-no-clear`).
    CopyAndStay,
    /// Keep the selection without copying (tmux with `MouseDragEnd1Pane`
    /// unbound).
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseConfig {
    /// Also applies to the copy after a double or triple click.
    pub drag_end: DragEnd,
    /// Lines per wheel notch (tmux binds `send-keys -X -N 5 scroll-up`).
    pub wheel_lines: u32,
    /// Longest gap between the clicks of a double or triple click.
    pub click_ms: u64,
}

impl Default for MouseConfig {
    fn default() -> Self {
        MouseConfig { drag_end: DragEnd::CopyAndCancel, wheel_lines: 5, click_ms: 300 }
    }
}

/// Settings shared by every pane's copy mode; cheap to clone.
#[derive(Clone, Debug)]
pub struct Config {
    pub mode_keys: ModeKeys,
    /// The key table for `mode_keys`, possibly changed by configuration.
    pub table: Rc<KeyTable>,
    pub word_separators: String,
    /// tmux's `wrap-search`.
    pub wrap_search: bool,
    pub mouse: MouseConfig,
    /// Rows per search step for searches started from keys (see
    /// [`CopyMode::search_step`]).
    pub search_step_rows: usize,
}

impl Config {
    /// tmux's defaults for `keys`.
    pub fn new(keys: ModeKeys) -> Config {
        Config {
            mode_keys: keys,
            table: Rc::new(KeyTable::defaults(keys)),
            word_separators: DEFAULT_WORD_SEPARATORS.to_string(),
            wrap_search: true,
            mouse: MouseConfig::default(),
            search_step_rows: DEFAULT_SEARCH_STEP_ROWS,
        }
    }
}

/// Flags of the `copy-mode` command that matter here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Enter {
    /// `-u`: start one page up.
    pub page_up: bool,
    /// `-e`: leave copy mode when scrolling reaches the bottom.
    pub exit_at_bottom: bool,
    /// `-H`: hide the position indicator.
    pub hide_position: bool,
}

/// What the server should do after copy mode handled something. Every
/// variant other than `None` implies the pane must be redrawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing changed.
    None,
    Redraw,
    /// Put `data` in a new paste buffer (or append it to the top one) and
    /// on the clipboard; then leave copy mode if `exit`. `pipe`: the copy
    /// came from tmux's `copy-pipe` family (keys bound to it, and mouse
    /// selections, which tmux binds to `copy-pipe-and-cancel`), so the text
    /// also goes to the `copy-command` option's command when one is set.
    Copy { data: Vec<u8>, append: bool, exit: bool, pipe: bool },
    /// The key is bound to commands copy mode does not run itself
    /// ([`Action::Commands`](crate::Action)): look it up in the model's
    /// key table and run them.
    Binding,
    /// Leave copy mode (call [`CopyMode::leave`] first).
    Exit,
    /// Show a prompt on the status line and feed its input back through
    /// [`CopyMode::prompt_changed`] (incremental only),
    /// [`CopyMode::prompt_accept`] and [`CopyMode::prompt_cancel`].
    Prompt { label: &'static str, initial: String, incremental: bool },
    /// Show a message on the status line.
    Message(String),
    /// A search is under way: redraw, then call [`CopyMode::search_step`]
    /// again (after polling for other events without blocking) until it
    /// returns something else.
    Searching,
}

/// Shape of a selection, as lif-render's `SelectionKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionShape {
    Stream,
    Lines,
    Rect,
}

/// The selection in pane coordinates, both ends inclusive, start before end
/// (same meaning as lif-render's `Selection`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionOverlay {
    pub start: Pos,
    pub end: Pos,
    pub shape: SelectionShape,
}

/// Everything the renderer draws over the pane for copy mode.
#[derive(Clone, Copy, Debug)]
pub struct Overlays<'a> {
    pub selection: Option<SelectionOverlay>,
    /// The copy-mode cursor, in pane coordinates.
    pub cursor: Pos,
    /// `[offset/history]`, right-aligned on the first row; `None` when
    /// hidden (`-H`, toggle-position).
    pub indicator: Option<&'a str>,
    /// Search matches in the viewport.
    pub matches: &'a [MatchSpan],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelKind {
    Char,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sel {
    anchor: Point,
    kind: SelKind,
    /// Set by stop-selection: the end no longer follows the cursor.
    end: Option<Point>,
    /// Line selections: the cursor is on the end side (tmux's
    /// LINE_SEL_LEFT_RIGHT) rather than the start side.
    ends: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PromptState {
    cmd: Command,
    origin: Point,
    top: usize,
}

#[derive(Default, Debug)]
struct MouseState {
    last_press: Option<(Pos, u64)>,
    clicks: u8,
    press: Option<Point>,
    dragging: bool,
    /// -1 / 1 while a drag sits on the top / bottom edge.
    edge: i8,
}

#[derive(Default, Debug)]
struct SearchState {
    last: Option<(String, bool, bool)>,
    pat: Option<Pattern>,
    job: Option<Job>,
    current: Option<Match>,
    show: bool,
    line: LineText,
    scratch: Scratch,
    /// Whether the job runs for an incremental prompt (failure is silent).
    quiet: bool,
}

/// One pane's copy mode.
///
/// Positions are absolute rows of the [`PaneView`] (0 = oldest) and
/// columns. Every public method takes the view, re-reads its size and
/// history first (output may have arrived in between) and tells it where
/// the viewport is afterwards.
#[derive(Debug)]
pub struct CopyMode {
    cfg: Config,
    cols: usize,
    rows: usize,
    history: usize,
    total: usize,
    trimmed: u64,
    top: usize,
    applied_top: Option<usize>,
    cur: Point,
    /// tmux's lastcx / lastsx: the column vertical motions aim for, and
    /// the length of the line it was taken on (to keep end-of-line sticky).
    last_cx: usize,
    last_sx: usize,
    /// Where the last vertical move left the cursor after moving it off a
    /// wide tail, and the tail column. tmux would have left the cursor on
    /// the tail; remembering it keeps the column from drifting left.
    tail_snap: Option<(Point, usize)>,
    sel: Option<Sel>,
    rect: bool,
    count: Option<u32>,
    awaiting_jump: Option<Command>,
    last_jump: Option<(Command, char)>,
    mark: Option<Point>,
    hide_position: bool,
    exit_at_bottom: bool,
    recentre: u8,
    prompt: Option<PromptState>,
    search: SearchState,
    mouse: MouseState,
    cache: RowCache,
    indicator: String,
    spans: Vec<MatchSpan>,
}

fn mods_plain(m: Mods) -> bool {
    !m.contains(Mods::CTRL) && !m.contains(Mods::ALT) && !m.contains(Mods::META)
}

impl CopyMode {
    /// Enter copy mode on `view` with the cursor at the pane's cursor
    /// (`cursor`, screen coordinates).
    pub fn new(view: &mut dyn PaneView, cfg: Config, cursor: Pos, enter: Enter) -> CopyMode {
        let mut m = CopyMode {
            cfg,
            cols: 1,
            rows: 1,
            history: 0,
            total: 1,
            trimmed: view.trimmed_rows(),
            top: 0,
            applied_top: None,
            cur: Point::default(),
            last_cx: 0,
            last_sx: 0,
            tail_snap: None,
            sel: None,
            rect: false,
            count: None,
            awaiting_jump: None,
            last_jump: None,
            mark: None,
            hide_position: enter.hide_position,
            exit_at_bottom: enter.exit_at_bottom,
            recentre: 0,
            prompt: None,
            search: SearchState::default(),
            mouse: MouseState::default(),
            cache: RowCache::default(),
            indicator: String::new(),
            spans: Vec::new(),
        };
        m.sync(view);
        m.top = m.history;
        m.cur = Point { row: m.history + cursor.y as usize, col: cursor.x as usize };
        m.clamp();
        m.with_grid(view, |m, g| m.cur.col = g.head(m.cur.row, m.cur.col));
        if enter.page_up {
            m.run(view, Command::PageUp, None, 1);
        }
        m.apply(view);
        m
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Replace the settings (e.g. after the configuration was reloaded).
    pub fn set_config(&mut self, cfg: Config) {
        self.cfg = cfg;
    }

    /// The cursor, as an absolute position.
    pub fn cursor(&self) -> Point {
        self.cur
    }

    /// The absolute row at the top of the viewport.
    pub fn top(&self) -> usize {
        self.top
    }

    /// `(offset, history)`: rows the viewport is scrolled up from the
    /// bottom, and rows of history (tmux's `scroll_position` and
    /// `history_size`).
    pub fn position(&self) -> (usize, usize) {
        (self.history - self.top, self.history)
    }

    /// The selection's ends (anchor, other end) and whether it is a line,
    /// rectangle or character selection.
    pub fn selection(&self) -> Option<(Point, Point, SelectionShape)> {
        let s = self.sel?;
        let shape = match s.kind {
            SelKind::Line => SelectionShape::Lines,
            SelKind::Char if self.rect => SelectionShape::Rect,
            SelKind::Char => SelectionShape::Stream,
        };
        Some((s.anchor, s.end.unwrap_or(self.cur), shape))
    }

    /// The repeat count being typed, if any (for a `(repeat)` display).
    pub fn pending_count(&self) -> Option<u32> {
        self.count
    }

    /// A short label for a pending one-key prompt (jumps), for the status
    /// line: tmux shows these as prompts.
    pub fn pending_label(&self) -> Option<&'static str> {
        Some(match self.awaiting_jump? {
            Command::JumpForward => "(jump forward)",
            Command::JumpBackward => "(jump backward)",
            Command::JumpToForward => "(jump to forward)",
            _ => "(jump to backward)",
        })
    }

    /// The last search pattern (tmux's `pane_search_string`).
    pub fn search_string(&self) -> Option<&str> {
        self.search.last.as_ref().map(|l| l.0.as_str())
    }

    /// Whether a search is waiting for [`search_step`](Self::search_step).
    pub fn searching(&self) -> bool {
        self.search.job.is_some()
    }

    // ----- plumbing -----------------------------------------------------

    fn sync(&mut self, view: &dyn PaneView) {
        let size = view.size();
        let trimmed = view.trimmed_rows();
        if trimmed > self.trimmed {
            let d = (trimmed - self.trimmed).min(usize::MAX as u64) as usize;
            let shift = |p: &mut Point| p.row = p.row.saturating_sub(d);
            shift(&mut self.cur);
            if let Some(s) = &mut self.sel {
                shift(&mut s.anchor);
                if let Some(e) = &mut s.end {
                    shift(e);
                }
            }
            if let Some(m) = &mut self.mark {
                shift(m);
            }
            if let Some(p) = &mut self.mouse.press {
                shift(p);
            }
            if let Some(p) = &mut self.prompt {
                shift(&mut p.origin);
                p.top = p.top.saturating_sub(d);
            }
            self.search.current = None;
            self.search.job = None;
            self.top = self.top.saturating_sub(d);
            self.applied_top = None;
            self.trimmed = trimmed;
        }
        self.cols = (size.cols as usize).max(1);
        self.rows = (size.rows as usize).max(1);
        self.history = view.history_rows();
        self.total = self.history + self.rows;
        self.cache.invalidate();
        self.clamp();
    }

    fn clamp(&mut self) {
        let last = self.total - 1;
        let cols = self.cols;
        let fix = |p: &mut Point| {
            p.row = p.row.min(last);
            p.col = p.col.min(cols - 1);
        };
        fix(&mut self.cur);
        if let Some(s) = &mut self.sel {
            fix(&mut s.anchor);
            if let Some(e) = &mut s.end {
                fix(e);
            }
        }
        if let Some(m) = &mut self.mark {
            fix(m);
        }
        if let Some(p) = &mut self.mouse.press {
            fix(p);
        }
        if let Some(p) = &mut self.prompt {
            fix(&mut p.origin);
            p.top = p.top.min(self.history);
        }
        self.top = self.top.min(self.history);
        self.ensure_visible();
    }

    fn ensure_visible(&mut self) {
        if self.cur.row < self.top {
            self.top = self.cur.row;
        } else if self.cur.row >= self.top + self.rows {
            self.top = self.cur.row + 1 - self.rows;
        }
        self.top = self.top.min(self.history);
    }

    /// tmux's window_copy_scroll_to: show a far-away row a quarter of the
    /// screen above the bottom (or at the very top / bottom of the history).
    fn scroll_to(&mut self, row: usize) {
        if row >= self.top && row < self.top + self.rows {
            return;
        }
        let gap = self.rows / 4;
        self.top = if row < self.rows {
            0
        } else if row > self.history + self.rows - gap {
            self.history
        } else {
            row + gap - self.rows
        };
        self.top = self.top.min(self.history);
        self.ensure_visible();
    }

    /// End of every entry point: the cursor never rests on a wide tail
    /// (content may have changed under it), and the view shows our viewport.
    fn apply(&mut self, view: &mut dyn PaneView) {
        self.with_grid(&*view, |m, g| m.cur.col = g.head(m.cur.row, m.cur.col));
        if self.applied_top != Some(self.top) {
            view.set_viewport_top(self.top);
            self.applied_top = Some(self.top);
        }
    }

    fn with_grid<R>(&mut self, view: &dyn PaneView, f: impl FnOnce(&mut Self, &mut Grid) -> R) -> R {
        let mut cache = std::mem::take(&mut self.cache);
        let r = {
            let mut g = Grid::new(view, &mut cache, self.cols, self.total);
            f(self, &mut g)
        };
        self.cache = cache;
        r
    }

    fn vi(&self) -> bool {
        self.cfg.mode_keys == ModeKeys::Vi
    }

    fn rect_active(&self) -> bool {
        self.rect && self.sel.is_some_and(|s| s.kind == SelKind::Char)
    }

    /// The rightmost column the cursor may rest on in `row`.
    fn max_x(&self, g: &mut Grid, row: usize) -> usize {
        if self.rect_active() {
            return self.cols - 1;
        }
        let len = g.len(row);
        let x = if self.vi() { len.saturating_sub(1) } else { len.min(self.cols - 1) };
        g.head(row, x)
    }

    fn normalize(&mut self, g: &mut Grid) {
        self.cur.row = self.cur.row.min(self.total - 1);
        let max = self.max_x(g, self.cur.row);
        self.cur.col = g.head(self.cur.row, self.cur.col.min(max));
    }

    /// Before a vertical move: remember the column to aim for, unless the
    /// cursor sits exactly at the end of its line (tmux's lastcx/lastsx).
    fn note_col(&mut self, g: &mut Grid) {
        let ox = g.len(self.cur.row);
        let cx = match self.tail_snap.take() {
            Some((p, tail)) if p == self.cur => tail,
            _ => self.cur.col,
        };
        if cx != ox {
            self.last_cx = cx;
            self.last_sx = ox;
        }
    }

    /// After a vertical move: tmux's column rule. `aim` uses the remembered
    /// column (cursor motions); page motions keep the current one.
    fn place_col(&mut self, g: &mut Grid, aim: bool) {
        let row = self.cur.row;
        if !self.rect_active() {
            if aim {
                self.cur.col = self.last_cx;
            }
            let px = g.len(row);
            if (self.cur.col >= self.last_sx && self.cur.col != px) || self.cur.col > px {
                self.cur.col = px;
            }
        }
        if let Some(s) = self.sel
            && s.kind == SelKind::Line
            && s.end.is_none()
        {
            self.cur.col = if s.ends { g.len(row) } else { 0 };
        }
        let before = self.cur.col;
        self.normalize(g);
        if self.cur.col != before && g.is_tail(row, before) && g.head(row, before) == self.cur.col {
            self.tail_snap = Some((self.cur, before));
        }
    }

    fn reader_op(&mut self, g: &mut Grid, f: impl FnOnce(&mut Reader)) {
        let mut r = Reader::new(g, self.cur.col, self.cur.row);
        f(&mut r);
        let (x, y) = (r.x.min(self.cols - 1), r.y);
        self.cur = Point { row: y, col: g.head(y, x) };
    }

    // ----- input --------------------------------------------------------

    /// Handle a key press.
    pub fn handle_key(&mut self, ev: &KeyEvent, view: &mut dyn PaneView) -> Outcome {
        if ev.action == KeyAction::Release || matches!(ev.key, Key::Modifier(_)) {
            return Outcome::None;
        }
        self.sync(view);
        let had_job = self.search.job.take().is_some();
        let out = self.key(ev, view);
        self.apply(view);
        if had_job && out == Outcome::None { Outcome::Redraw } else { out }
    }

    fn key(&mut self, ev: &KeyEvent, view: &mut dyn PaneView) -> Outcome {
        if let Some(cmd) = self.awaiting_jump.take() {
            let mut chars = ev.text.chars();
            let ch = match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_control() => Some(c),
                _ => match ev.key {
                    Key::Char(c) if mods_plain(ev.mods) && !c.is_control() => Some(c),
                    _ => None,
                },
            };
            let n = self.count.take().unwrap_or(1);
            return match ch {
                Some(c) => {
                    let mut buf = [0u8; 4];
                    self.run(view, cmd, Some(c.encode_utf8(&mut buf)), n)
                }
                None => Outcome::Redraw,
            };
        }
        let Some(code) = KeyCode::from_event(ev) else { return Outcome::None };
        if let Some(n) = self.count
            && let Key::Char(d @ '0'..='9') = code.key
            && code.mods.is_empty()
        {
            self.count = Some((n * 10 + d as u32 - '0' as u32).min(1_000_000));
            return Outcome::Redraw;
        }
        let Some(action) = self.cfg.table.get(&code) else {
            self.count = None;
            return Outcome::None;
        };
        match action {
            Action::Count(d) => {
                self.count = Some(d as u32);
                Outcome::Redraw
            }
            Action::Command(cmd) => {
                let n = self.count.take().unwrap_or(1);
                self.run(view, cmd, None, n)
            }
            Action::Pipe(cmd) => {
                let n = self.count.take().unwrap_or(1);
                match self.run(view, cmd, None, n) {
                    Outcome::Copy { data, append, exit, .. } => Outcome::Copy { data, append, exit, pipe: true },
                    other => other,
                }
            }
            Action::Commands => {
                self.count = None;
                Outcome::Binding
            }
            Action::Ask(cmd) if cmd.is_jump() => {
                self.awaiting_jump = Some(cmd);
                Outcome::Redraw
            }
            Action::Ask(cmd) => {
                self.count = None;
                self.exit_at_bottom = false;
                self.prompt = Some(PromptState { cmd, origin: self.cur, top: self.top });
                let incremental =
                    matches!(cmd, Command::SearchForwardIncremental | Command::SearchBackwardIncremental);
                let label = match cmd {
                    Command::GotoLine => "(goto line)",
                    Command::SearchBackward
                    | Command::SearchBackwardText
                    | Command::SearchBackwardIncremental => "(search up)",
                    _ => "(search down)",
                };
                let initial = if incremental { self.search_string().unwrap_or("").to_string() } else { String::new() };
                Outcome::Prompt { label, initial, incremental }
            }
            Action::SearchWord { forward } => {
                let n = self.count.take().unwrap_or(1);
                self.exit_at_bottom = false;
                let word = self.with_grid(view, |m, g| m.cursor_word(g));
                if word.is_empty() {
                    return Outcome::None;
                }
                let cmd = if forward { Command::SearchForwardText } else { Command::SearchBackwardText };
                self.run(view, cmd, Some(&word), n)
            }
        }
    }

    /// The prompt's text changed (incremental search only): search from
    /// where the prompt was opened, as tmux's `-i` prompts do.
    pub fn prompt_changed(&mut self, view: &mut dyn PaneView, text: &str) -> Outcome {
        self.sync(view);
        let Some(p) = self.prompt else { return Outcome::None };
        let forward = match p.cmd {
            Command::SearchForwardIncremental => true,
            Command::SearchBackwardIncremental => false,
            _ => return Outcome::None,
        };
        self.cur = p.origin;
        self.top = p.top;
        self.clamp();
        let out = if text.is_empty() {
            self.search.job = None;
            self.search.current = None;
            self.search.show = false;
            Outcome::Redraw
        } else {
            self.with_grid(view, |m, g| m.start_search(g, text, false, forward, 1, p.origin, true))
        };
        self.apply(view);
        out
    }

    /// Search again from the current match within an incremental prompt
    /// (C-s / C-r pressed again).
    pub fn prompt_again(&mut self, view: &mut dyn PaneView, forward: bool) -> Outcome {
        self.sync(view);
        let Some((text, regex, _)) = self.search.last.clone() else { return Outcome::None };
        let origin = self.cur;
        let out = self.with_grid(view, |m, g| m.start_search(g, &text, regex, forward, 1, origin, true));
        self.apply(view);
        out
    }

    /// The prompt was answered.
    pub fn prompt_accept(&mut self, view: &mut dyn PaneView, text: &str) -> Outcome {
        self.sync(view);
        let Some(p) = self.prompt.take() else { return Outcome::None };
        let out = match p.cmd {
            Command::SearchForwardIncremental | Command::SearchBackwardIncremental => {
                if text.is_empty() {
                    // An empty incremental search repeats the last one.
                    let forward = p.cmd == Command::SearchForwardIncremental;
                    match self.search.last.clone() {
                        Some((t, regex, _)) => {
                            self.with_grid(view, |m, g| m.start_search(g, &t, regex, forward, 1, p.origin, false))
                        }
                        None => Outcome::Redraw,
                    }
                } else if self.search.job.is_some() {
                    self.search.quiet = false;
                    Outcome::Searching
                } else {
                    Outcome::Redraw
                }
            }
            cmd => {
                if text.is_empty() {
                    Outcome::Redraw
                } else {
                    self.run(view, cmd, Some(text), 1)
                }
            }
        };
        self.apply(view);
        out
    }

    /// The prompt was dismissed: an incremental search goes back to where
    /// it started.
    pub fn prompt_cancel(&mut self, view: &mut dyn PaneView) -> Outcome {
        self.sync(view);
        let Some(p) = self.prompt.take() else { return Outcome::None };
        if matches!(p.cmd, Command::SearchForwardIncremental | Command::SearchBackwardIncremental) {
            self.search.job = None;
            self.search.current = None;
            self.search.show = false;
            self.cur = p.origin;
            self.top = p.top;
            self.clamp();
        }
        self.apply(view);
        Outcome::Redraw
    }

    /// Continue a pending search, examining about `budget_rows` rows.
    ///
    /// The server calls this while [`searching`](Self::searching) is true,
    /// once per turn of its event loop (polling its descriptors with a zero
    /// timeout in between, so input and output keep flowing). Any key press
    /// cancels the search.
    pub fn search_step(&mut self, view: &mut dyn PaneView, budget_rows: usize) -> Outcome {
        self.sync(view);
        if self.search.job.is_none() {
            return Outcome::None;
        }
        let out = self.with_grid(view, |m, g| m.drive_search(g, budget_rows));
        self.apply(view);
        out
    }

    /// Handle a mouse event with pane-relative coordinates. `now_ms` is a
    /// monotonic clock reading, used to recognize double and triple clicks.
    pub fn handle_mouse(&mut self, ev: &MouseEvent, now_ms: u64, view: &mut dyn PaneView) -> Outcome {
        self.sync(view);
        let out = self.with_grid(view, |m, g| m.mouse(ev, now_ms, g));
        let out = match out {
            MouseOut::Done(o) => o,
            MouseOut::Run(cmd, n) => self.run(view, cmd, None, n),
        };
        self.apply(view);
        out
    }

    /// Whether a drag is resting on the top or bottom edge, so the server
    /// should call [`autoscroll_tick`](Self::autoscroll_tick) on a timer
    /// (tmux scrolls every 50 ms) until it stops.
    pub fn autoscroll_active(&self) -> bool {
        self.mouse.dragging && self.mouse.edge != 0
    }

    /// Scroll one line further while a drag rests on an edge.
    pub fn autoscroll_tick(&mut self, view: &mut dyn PaneView) -> Outcome {
        self.sync(view);
        if !self.autoscroll_active() {
            return Outcome::None;
        }
        let up = self.mouse.edge < 0;
        let moved = self.edge_scroll(up);
        self.apply(view);
        if moved { Outcome::Redraw } else { Outcome::None }
    }

    /// Run a command, as `send-keys -X [-N count] <command> [argument]`.
    pub fn run_command(&mut self, view: &mut dyn PaneView, cmd: Command, arg: Option<&str>, count: u32) -> Outcome {
        self.sync(view);
        self.search.job = None;
        let out = self.run(view, cmd, arg, count.max(1));
        self.apply(view);
        out
    }

    /// Re-read the pane after it changed size or content, keeping
    /// everything in bounds.
    pub fn refresh(&mut self, view: &mut dyn PaneView) {
        self.sync(view);
        self.with_grid(view, |m, g| m.normalize(g));
        // The engine may have moved its viewport when output arrived.
        self.applied_top = None;
        self.apply(view);
    }

    /// Put the viewport back on the live screen. Call when leaving.
    pub fn leave(&mut self, view: &mut dyn PaneView) {
        let h = view.history_rows();
        view.set_viewport_top(h);
        self.applied_top = Some(h);
    }

    // ----- overlays -----------------------------------------------------

    /// What to draw over the pane for the current viewport.
    pub fn overlays(&mut self, view: &dyn PaneView) -> Overlays<'_> {
        self.sync(view);
        self.indicator.clear();
        if !self.hide_position {
            let _ = write!(self.indicator, "[{}/{}]", self.history - self.top, self.history);
        }
        let selection = self.with_grid(view, |m, g| m.selection_overlay(g));
        self.spans.clear();
        if self.search.show
            && let Some(pat) = &self.search.pat
        {
            let mut cache = std::mem::take(&mut self.cache);
            {
                let mut g = Grid::new(view, &mut cache, self.cols, self.total);
                viewport_matches(
                    &mut g,
                    pat,
                    &mut self.search.line,
                    &mut self.search.scratch,
                    self.top,
                    self.rows,
                    self.search.current,
                    &mut self.spans,
                );
            }
            self.cache = cache;
        }
        Overlays {
            selection,
            cursor: Pos { x: self.cur.col as u16, y: (self.cur.row - self.top) as u16 },
            indicator: (!self.hide_position).then_some(self.indicator.as_str()),
            matches: &self.spans,
        }
    }

    fn selection_overlay(&mut self, g: &mut Grid) -> Option<SelectionOverlay> {
        let s = self.sel?;
        let other = s.end.unwrap_or(self.cur);
        let (top, bottom) = (self.top, self.top + self.rows - 1);
        let (lo, hi) = if s.anchor <= other { (s.anchor, other) } else { (other, s.anchor) };
        if hi.row < top || lo.row > bottom {
            return None;
        }
        let clip_rows = |r: usize| (r.clamp(top, bottom) - top) as u16;
        let shape;
        let (mut start, mut end);
        match s.kind {
            SelKind::Line => {
                shape = SelectionShape::Lines;
                let first = g.line_start(lo.row, self.rows);
                let last = g.line_end(hi.row, self.rows);
                start = Pos { x: 0, y: clip_rows(first) };
                end = Pos { x: (self.cols - 1) as u16, y: clip_rows(last) };
            }
            SelKind::Char if self.rect => {
                shape = SelectionShape::Rect;
                let (a, c) = (s.anchor.col, other.col);
                let (x0, mut x1) = if a <= c { (a, c) } else { (c, a) };
                if a < c && !self.vi() {
                    x1 -= 1;
                }
                x1 = (x1 + g.width_at(other.row, x1) - 1).min(self.cols - 1);
                start = Pos { x: x0 as u16, y: clip_rows(lo.row) };
                end = Pos { x: x1 as u16, y: clip_rows(hi.row) };
            }
            SelKind::Char => {
                shape = SelectionShape::Stream;
                let mut e = hi;
                if !self.vi() {
                    // emacs: the cursor sits between characters, so the
                    // bottom-right end is exclusive.
                    if e == lo {
                        return None;
                    }
                    if e.col > 0 {
                        e.col = g.head(e.row, e.col - 1);
                    } else {
                        e.row -= 1;
                        e.col = self.cols - 1;
                    }
                } else {
                    e.col = (e.col + g.width_at(e.row, e.col) - 1).min(self.cols - 1);
                }
                start = if lo.row < top { Pos { x: 0, y: 0 } } else { Pos { x: lo.col as u16, y: (lo.row - top) as u16 } };
                end = if e.row > bottom {
                    Pos { x: (self.cols - 1) as u16, y: (bottom - top) as u16 }
                } else if e.row < top {
                    return None;
                } else {
                    Pos { x: e.col as u16, y: (e.row - top) as u16 }
                };
            }
        }
        if (start.y, start.x) > (end.y, end.x) && shape == SelectionShape::Stream {
            std::mem::swap(&mut start, &mut end);
        }
        Some(SelectionOverlay { start, end, shape })
    }

    /// The text the current selection would copy.
    pub fn selection_text(&mut self, view: &dyn PaneView) -> Option<String> {
        self.sync(view);
        self.with_grid(view, |m, g| m.sel_text(g))
    }

    // ----- commands -----------------------------------------------------

    fn run(&mut self, view: &mut dyn PaneView, cmd: Command, arg: Option<&str>, n: u32) -> Outcome {
        if !cmd.is_scroll() {
            self.exit_at_bottom = false;
        }
        if cmd != Command::RecentreTopBottom {
            self.recentre = 0;
        }
        if cmd == Command::NextPrompt || cmd == Command::PreviousPrompt {
            return self.prompt_jump(view, cmd == Command::NextPrompt, n);
        }
        self.with_grid(view, |m, g| m.exec(g, cmd, arg, n))
    }

    fn exec(&mut self, g: &mut Grid, cmd: Command, arg: Option<&str>, n: u32) -> Outcome {
        use Command as C;
        let seps = self.cfg.word_separators.clone();
        match cmd {
            C::CursorLeft => {
                for _ in 0..n {
                    self.reader_op(g, |r| r.left(true));
                }
                self.normalize(g);
            }
            C::CursorRight => {
                let all = self.rect_active();
                for _ in 0..n {
                    // At the last column the cursor may rest on (len - 1 in
                    // vi, len in emacs) it moves to the next line.
                    let max = self.max_x(g, self.cur.row);
                    if self.cur.col >= max {
                        if !all && self.cur.row + 1 < self.total {
                            self.cur = Point { row: self.cur.row + 1, col: 0 };
                        }
                        continue;
                    }
                    self.reader_op(g, |r| r.right(false, all));
                    self.normalize(g);
                }
                self.normalize(g);
            }
            C::CursorUp | C::CursorDown | C::CursorDownAndCancel => {
                let down = cmd != C::CursorUp;
                if cmd == C::CursorDownAndCancel && self.cur.row + 1 >= self.total {
                    return Outcome::Exit;
                }
                for _ in 0..n {
                    self.note_col(g);
                    // A line selection keeps its anchored line whole: moving
                    // past it swaps the ends, as tmux does.
                    if let Some(s) = &mut self.sel
                        && s.kind == SelKind::Line
                        && s.end.is_none()
                        && s.ends != down
                        && self.cur.row == s.anchor.row
                    {
                        std::mem::swap(&mut s.anchor, &mut self.cur);
                        s.ends = down;
                    }
                    if down {
                        if self.cur.row + 1 < self.total {
                            self.cur.row += 1;
                        }
                    } else if self.cur.row > 0 {
                        self.cur.row -= 1;
                    }
                    self.place_col(g, true);
                }
            }
            C::StartOfLine => self.reader_op(g, |r| r.start_of_line(true)),
            C::EndOfLine => {
                let all = self.rect_active();
                for k in 0..n {
                    if k > 0 && self.cur.row + 1 < self.total {
                        self.cur.row += 1;
                    }
                    self.reader_op(g, |r| r.end_of_line(true, all));
                }
                self.normalize(g);
            }
            C::BackToIndentation => self.reader_op(g, |r| r.back_to_indentation()),
            C::NextWord | C::NextSpace => {
                let s = if cmd == C::NextWord { seps.as_str() } else { "" };
                for _ in 0..n {
                    self.reader_op(g, |r| r.next_word(s));
                }
                self.normalize(g);
            }
            C::NextWordEnd | C::NextSpaceEnd => {
                let s = if cmd == C::NextWordEnd { seps.as_str() } else { "" };
                let vi = self.vi();
                for _ in 0..n {
                    self.reader_op(g, |r| {
                        if vi {
                            if !r.g.in_set(r.y, r.x, WHITESPACE) {
                                r.right(false, false);
                            }
                            r.next_word_end(s);
                            r.left(true);
                        } else {
                            r.next_word_end(s);
                        }
                    });
                }
                self.normalize(g);
            }
            C::PreviousWord | C::PreviousSpace => {
                let s = if cmd == C::PreviousWord { seps.as_str() } else { "" };
                for _ in 0..n {
                    self.reader_op(g, |r| r.previous_word(s, true, true));
                }
                self.normalize(g);
            }
            C::PageUp | C::HalfpageUp => {
                for _ in 0..n {
                    self.page_up(cmd == C::HalfpageUp);
                }
                self.place_col(g, false);
            }
            C::PageDown | C::PageDownAndCancel | C::HalfpageDown | C::HalfpageDownAndCancel => {
                let half = matches!(cmd, C::HalfpageDown | C::HalfpageDownAndCancel);
                let cancel = matches!(cmd, C::PageDownAndCancel | C::HalfpageDownAndCancel);
                for _ in 0..n {
                    self.page_down(half);
                }
                self.place_col(g, false);
                if self.top == self.history && (cancel || self.exit_at_bottom) && self.sel.is_none() {
                    return Outcome::Exit;
                }
            }
            C::ScrollUp => {
                self.note_col(g);
                self.scroll_lines(-(n as i64));
                self.place_col(g, true);
            }
            C::ScrollDown | C::ScrollDownAndCancel => {
                self.note_col(g);
                self.scroll_lines(n as i64);
                self.place_col(g, true);
                let cancel = cmd == C::ScrollDownAndCancel;
                if self.top == self.history && (cancel || self.exit_at_bottom) && self.sel.is_none() {
                    return Outcome::Exit;
                }
            }
            C::TopLine | C::MiddleLine | C::BottomLine => {
                let off = match cmd {
                    C::TopLine => 0,
                    C::MiddleLine => (self.rows - 1) / 2,
                    _ => self.rows - 1,
                };
                self.cur = Point { row: (self.top + off).min(self.total - 1), col: 0 };
                self.normalize(g);
            }
            C::CursorCentreVertical => {
                self.cur.row = (self.top + (self.rows - 1) / 2).min(self.total - 1);
                self.normalize(g);
            }
            C::CursorCentreHorizontal => {
                self.cur.col = self.cols / 2;
                self.normalize(g);
            }
            C::HistoryTop => {
                self.top = 0;
                self.cur = Point { row: 0, col: 0 };
            }
            C::HistoryBottom => {
                self.top = self.history;
                let row = self.total - 1;
                self.cur = Point { row, col: g.len(row) };
                self.normalize(g);
            }
            C::GotoLine => {
                let Some(line) = arg.and_then(|a| a.trim().parse::<usize>().ok()) else {
                    return Outcome::Message("Invalid line number".into());
                };
                let rel = self.cur.row - self.top;
                self.top = self.history - line.min(self.history);
                self.cur.row = self.top + rel;
                self.normalize(g);
            }
            C::ScrollMiddle | C::ScrollTop | C::ScrollBottom => {
                let off = match cmd {
                    C::ScrollMiddle => (self.rows - 1) / 2,
                    C::ScrollTop => 0,
                    _ => self.rows - 1,
                };
                self.top = self.cur.row.saturating_sub(off).min(self.history);
                self.ensure_visible();
            }
            C::RecentreTopBottom => {
                let off = match self.recentre % 3 {
                    0 => (self.rows - 1) / 2,
                    1 => 0,
                    _ => self.rows - 1,
                };
                self.recentre = self.recentre.wrapping_add(1);
                self.top = self.cur.row.saturating_sub(off).min(self.history);
                self.ensure_visible();
            }
            C::BeginSelection => {
                self.sel = Some(Sel { anchor: self.cur, kind: SelKind::Char, end: None, ends: true });
            }
            C::SelectLine => {
                self.rect = false;
                self.reader_op(g, |r| r.start_of_line(true));
                self.sel = Some(Sel { anchor: self.cur, kind: SelKind::Line, end: None, ends: true });
                for k in 0..n {
                    if k > 0 && self.cur.row + 1 < self.total {
                        self.cur.row += 1;
                    }
                    self.reader_op(g, |r| r.end_of_line(true, false));
                }
                self.normalize(g);
            }
            C::SelectWord => {
                self.rect = false;
                self.select_word(g);
            }
            C::RectangleToggle => self.rect = !self.rect,
            C::RectangleOn => self.rect = true,
            C::RectangleOff => self.rect = false,
            C::OtherEnd => {
                if let Some(s) = &mut self.sel
                    && s.end.is_none()
                {
                    std::mem::swap(&mut s.anchor, &mut self.cur);
                    s.ends = !s.ends;
                    let row = self.cur.row;
                    self.scroll_to(row);
                }
            }
            C::ClearSelection => {
                if self.sel.take().is_none() {
                    return Outcome::None;
                }
            }
            C::StopSelection => {
                if let Some(s) = &mut self.sel {
                    s.end = Some(self.cur);
                }
            }
            C::CopySelection
            | C::CopySelectionNoClear
            | C::CopySelectionAndCancel
            | C::AppendSelection
            | C::AppendSelectionAndCancel => {
                let exit = matches!(cmd, C::CopySelectionAndCancel | C::AppendSelectionAndCancel);
                let append = matches!(cmd, C::AppendSelection | C::AppendSelectionAndCancel);
                let text = self.sel_text(g);
                if cmd != C::CopySelectionNoClear {
                    self.sel = None;
                }
                return match text {
                    Some(t) => Outcome::Copy { data: t.into_bytes(), append, exit, pipe: false },
                    None if exit => Outcome::Exit,
                    None => Outcome::Redraw,
                };
            }
            C::CopyLine | C::CopyLineAndCancel | C::CopyEndOfLine | C::CopyEndOfLineAndCancel => {
                let exit = matches!(cmd, C::CopyLineAndCancel | C::CopyEndOfLineAndCancel);
                let whole = matches!(cmd, C::CopyLine | C::CopyLineAndCancel);
                let (saved_cur, saved_sel, saved_rect) = (self.cur, self.sel, self.rect);
                self.rect = false;
                if whole {
                    self.reader_op(g, |r| r.start_of_line(true));
                }
                self.sel = Some(Sel { anchor: self.cur, kind: SelKind::Char, end: None, ends: true });
                for k in 0..n {
                    if k > 0 && self.cur.row + 1 < self.total {
                        self.cur.row += 1;
                    }
                    self.reader_op(g, |r| r.end_of_line(true, false));
                }
                self.normalize(g);
                let text = self.sel_text(g);
                (self.cur, self.sel, self.rect) = (saved_cur, saved_sel, saved_rect);
                return match text {
                    Some(t) => Outcome::Copy { data: t.into_bytes(), append: false, exit, pipe: false },
                    None if exit => Outcome::Exit,
                    None => Outcome::Redraw,
                };
            }
            C::Cancel => return Outcome::Exit,
            C::JumpForward | C::JumpBackward | C::JumpToForward | C::JumpToBackward => {
                let Some(c) = arg.and_then(|a| a.chars().next()) else { return Outcome::None };
                self.last_jump = Some((cmd, c));
                for _ in 0..n {
                    self.jump(g, cmd, c);
                }
                self.normalize(g);
            }
            C::JumpAgain | C::JumpReverse => {
                let Some((kind, c)) = self.last_jump else { return Outcome::None };
                let kind = if cmd == C::JumpReverse {
                    match kind {
                        C::JumpForward => C::JumpBackward,
                        C::JumpBackward => C::JumpForward,
                        C::JumpToForward => C::JumpToBackward,
                        _ => C::JumpToForward,
                    }
                } else {
                    kind
                };
                for _ in 0..n {
                    self.jump(g, kind, c);
                }
                self.normalize(g);
            }
            C::SetMark => self.mark = Some(self.cur),
            C::JumpToMark => {
                let Some(m) = self.mark else { return Outcome::None };
                self.mark = Some(self.cur);
                self.cur = m;
                self.scroll_to(m.row);
                self.normalize(g);
            }
            C::NextParagraph | C::PreviousParagraph => {
                for _ in 0..n {
                    self.paragraph(g, cmd == C::NextParagraph);
                }
                self.normalize(g);
            }
            C::NextMatchingBracket | C::PreviousMatchingBracket => {
                for _ in 0..n {
                    if !self.bracket(g, cmd == C::NextMatchingBracket) {
                        break;
                    }
                }
                self.normalize(g);
            }
            C::NextPrompt | C::PreviousPrompt => {}
            C::SearchForward
            | C::SearchBackward
            | C::SearchForwardText
            | C::SearchBackwardText
            | C::SearchForwardIncremental
            | C::SearchBackwardIncremental => {
                let Some(text) = arg.filter(|a| !a.is_empty()) else { return Outcome::None };
                let forward =
                    matches!(cmd, C::SearchForward | C::SearchForwardText | C::SearchForwardIncremental);
                let regex = matches!(cmd, C::SearchForward | C::SearchBackward);
                let origin = self.search_origin(forward);
                return self.start_search(g, text, regex, forward, n, origin, false);
            }
            C::SearchAgain | C::SearchReverse => {
                let Some((text, regex, forward)) = self.search.last.clone() else { return Outcome::None };
                let forward = if cmd == C::SearchReverse { !forward } else { forward };
                let origin = self.search_origin(forward);
                let out = self.start_search(g, &text, regex, forward, n, origin, false);
                // search-reverse does not change the remembered direction.
                if let Some(l) = &mut self.search.last {
                    l.2 = if cmd == C::SearchReverse { !forward } else { forward };
                }
                return out;
            }
            C::TogglePosition => self.hide_position = !self.hide_position,
            C::ScrollExitOn => self.exit_at_bottom = true,
            C::ScrollExitOff => self.exit_at_bottom = false,
            C::ScrollExitToggle => self.exit_at_bottom = !self.exit_at_bottom,
            C::RefreshNow => {}
        }
        self.ensure_visible();
        Outcome::Redraw
    }

    fn page_up(&mut self, half: bool) {
        let n = if self.rows > 2 { if half { self.rows / 2 } else { self.rows - 2 } } else { 1 };
        let offset = self.history - self.top;
        let cy = self.cur.row - self.top;
        if offset + n > self.history {
            self.top = 0;
            self.cur.row = cy.saturating_sub(n);
        } else {
            self.top -= n;
            self.cur.row = self.top + cy;
        }
    }

    fn page_down(&mut self, half: bool) {
        let n = if self.rows > 2 { if half { self.rows / 2 } else { self.rows - 2 } } else { 1 };
        let offset = self.history - self.top;
        let cy = self.cur.row - self.top;
        if offset < n {
            self.top = self.history;
            self.cur.row = self.top + (cy + (n - offset)).min(self.rows - 1);
        } else {
            self.top += n;
            self.cur.row = self.top + cy;
        }
    }

    /// Move the viewport by `delta` rows (negative: back in history). In vi
    /// mode the cursor stays on its text unless pushed off the screen; in
    /// emacs mode it stays on its screen row.
    fn scroll_lines(&mut self, delta: i64) {
        let old = self.top;
        let new = (self.top as i64 + delta).clamp(0, self.history as i64) as usize;
        self.top = new;
        if self.vi() {
            if self.cur.row < self.top {
                self.cur.row = self.top;
            } else if self.cur.row >= self.top + self.rows {
                self.cur.row = self.top + self.rows - 1;
            }
        } else {
            self.cur.row = self.top + (self.cur.row - old);
        }
        self.cur.row = self.cur.row.min(self.total - 1);
    }

    fn edge_scroll(&mut self, up: bool) -> bool {
        if up {
            if self.top == 0 {
                return false;
            }
            self.top -= 1;
            self.cur.row = self.top;
        } else {
            if self.top >= self.history {
                return false;
            }
            self.top += 1;
            self.cur.row = self.top + self.rows - 1;
        }
        true
    }

    fn jump(&mut self, g: &mut Grid, kind: Command, c: char) {
        let y = self.cur.row;
        let xx = g.len(y);
        let x = self.cur.col;
        let hit = |g: &mut Grid, px: usize| g.single_char(y, px) == Some(c);
        match kind {
            Command::JumpForward => {
                let mut px = x + 1;
                while px < xx {
                    if hit(g, px) {
                        self.cur.col = px;
                        return;
                    }
                    px += 1;
                }
            }
            Command::JumpToForward => {
                let mut px = x + 2;
                while px < xx {
                    if hit(g, px) {
                        self.cur.col = g.head(y, px - 1);
                        return;
                    }
                    px += 1;
                }
            }
            Command::JumpBackward => {
                let mut px = x;
                while px > 0 {
                    px -= 1;
                    if hit(g, px) {
                        self.cur.col = px;
                        return;
                    }
                }
            }
            _ => {
                // jump-to-backward: tmux starts two cells back.
                if x < 2 {
                    return;
                }
                let mut px = x - 1;
                while px > 0 {
                    px -= 1;
                    if hit(g, px) {
                        let mut to = px + 1;
                        while to < self.cols - 1 && g.is_tail(y, to) {
                            to += 1;
                        }
                        self.cur.col = to;
                        return;
                    }
                }
            }
        }
    }

    fn paragraph(&mut self, g: &mut Grid, next: bool) {
        let mut oy = self.cur.row;
        if next {
            let maxy = self.total - 1;
            while oy < maxy && g.len(oy) == 0 {
                oy += 1;
            }
            while oy < maxy && g.len(oy) != 0 {
                oy += 1;
            }
            self.cur = Point { row: oy, col: g.len(oy) };
        } else {
            while oy > 0 && g.len(oy) == 0 {
                oy -= 1;
            }
            while oy > 0 && g.len(oy) > 0 {
                oy -= 1;
            }
            self.cur = Point { row: oy, col: 0 };
        }
        self.scroll_to(oy);
    }

    /// Move to the bracket matching the one under (or, for next, after) the
    /// cursor. Only the same kind of bracket nests.
    fn bracket(&mut self, g: &mut Grid, next: bool) -> bool {
        const OPEN: &str = "{[(";
        const CLOSE: &str = "}])";
        let y = self.cur.row;
        let len = g.len(y);
        if len == 0 {
            return false;
        }
        let mut found = None;
        if next {
            let mut x = self.cur.col;
            while x < len {
                if let Some(c) = g.single_char(y, x)
                    && (OPEN.contains(c) || CLOSE.contains(c))
                {
                    found = Some((x, c));
                    break;
                }
                x += 1;
            }
        } else {
            let mut x = self.cur.col.min(len - 1) + 1;
            while x > 0 {
                x -= 1;
                if let Some(c) = g.single_char(y, x)
                    && CLOSE.contains(c)
                {
                    found = Some((x, c));
                    break;
                }
            }
        }
        let Some((bx, b)) = found else { return false };
        let (open, close, forward) = match OPEN.find(b) {
            Some(i) => (b, CLOSE.as_bytes()[i] as char, true),
            None => (OPEN.as_bytes()[CLOSE.find(b).unwrap_or(0)] as char, b, false),
        };
        let (mut px, mut py) = (bx, y);
        let mut depth = 0usize;
        let limit_lo = y.saturating_sub(BRACKET_ROWS);
        let limit_hi = (y + BRACKET_ROWS).min(self.total - 1);
        loop {
            if let Some(c) = g.single_char(py, px) {
                if c == open {
                    if forward { depth += 1 } else { depth = depth.saturating_sub(1) }
                } else if c == close {
                    if forward { depth = depth.saturating_sub(1) } else { depth += 1 }
                }
                if depth == 0 && (c == open || c == close) && (px, py) != (bx, y) {
                    self.cur = Point { row: py, col: px };
                    self.scroll_to(py);
                    return true;
                }
            }
            if forward {
                px += 1;
                if px >= g.len(py) {
                    if py >= limit_hi {
                        return false;
                    }
                    py += 1;
                    px = 0;
                }
            } else {
                if px == 0 {
                    if py <= limit_lo {
                        return false;
                    }
                    py -= 1;
                    px = g.len(py);
                    if px == 0 {
                        continue;
                    }
                }
                px -= 1;
            }
        }
    }

    fn prompt_jump(&mut self, view: &mut dyn PaneView, next: bool, n: u32) -> Outcome {
        let mut row = self.cur.row;
        for _ in 0..n {
            let r = if next {
                if row + 1 >= self.total {
                    None
                } else {
                    view.find_prompt(row + 1, true)
                }
            } else if row == 0 {
                None
            } else {
                view.find_prompt(row - 1, false)
            };
            match r {
                Some(r) if r < self.total => row = r,
                _ => break,
            }
        }
        if row == self.cur.row {
            return Outcome::None;
        }
        self.cur = Point { row, col: 0 };
        self.scroll_to(row);
        Outcome::Redraw
    }

    fn select_word(&mut self, g: &mut Grid) {
        let seps = self.cfg.word_separators.clone();
        self.reader_op(g, |r| r.previous_word(&seps, false, true));
        self.cur.col = g.head(self.cur.row, self.cur.col);
        let start = self.cur;
        let mut r = Reader::new(g, start.col, start.row);
        r.right(false, false);
        let single = r.x >= r.g.len(r.y) || r.g.in_set(r.y, r.x, WHITESPACE) || (r.x, r.y) == (start.col, start.row);
        let end = if single {
            start
        } else {
            let mut r = Reader::new(g, start.col, start.row);
            if !r.g.in_set(r.y, r.x, WHITESPACE) {
                r.right(false, false);
            }
            r.next_word_end(&seps);
            r.left(true);
            let x = r.x.min(self.cols - 1);
            Point { row: r.y, col: r.g.head(r.y, x) }
        };
        self.sel = Some(Sel { anchor: start, kind: SelKind::Char, end: None, ends: true });
        self.cur = end;
        if !self.vi() {
            // The emacs cursor sits after the last character.
            let w = g.width_at(end.row, end.col);
            self.cur = if end.col + w < self.cols {
                Point { row: end.row, col: end.col + w }
            } else if end.row + 1 < self.total {
                Point { row: end.row + 1, col: 0 }
            } else {
                end
            };
        }
        self.ensure_visible();
    }

    /// The word under the cursor (tmux's copy_cursor_word).
    fn cursor_word(&mut self, g: &mut Grid) -> String {
        let seps = self.cfg.word_separators.clone();
        let y = self.cur.row;
        let in_word = |g: &mut Grid, x: usize| !(g.in_set(y, x, &seps) || g.in_set(y, x, WHITESPACE));
        let x = g.head(y, self.cur.col);
        if x >= g.len(y) || !in_word(g, x) {
            return String::new();
        }
        let mut a = x;
        while a > 0 {
            let h = g.head(y, a - 1);
            if !in_word(g, h) {
                break;
            }
            a = h;
        }
        let mut b = x;
        let len = g.len(y);
        let mut s = String::new();
        while b < len && in_word(g, b) {
            b += 1;
        }
        for k in a..b {
            g.push_cell_text(y, k, &mut s);
        }
        s
    }

    // ----- selection text -----------------------------------------------

    fn sel_text(&mut self, g: &mut Grid) -> Option<String> {
        let s = self.sel?;
        let other = s.end.unwrap_or(self.cur);
        let vi = self.vi();
        let (lo, hi) = if s.anchor <= other { (s.anchor, other) } else { (other, s.anchor) };
        let cols = self.cols;
        let (sy, ey) = (lo.row, hi.row);
        let ey_last = g.len(ey);
        let mut firstsx = lo.col;
        let mut restsx = 0;
        let mut restex = cols;
        let mut lastex;
        match s.kind {
            SelKind::Line => {
                let sy2 = g.line_start(sy, usize::MAX);
                let ey2 = g.line_end(ey, usize::MAX);
                let l = g.len(ey2);
                return self.copy_rows(g, sy2, ey2, 0, 0, if vi { l + 1 } else { l }, cols);
            }
            SelKind::Char => {
                let ex = hi.col.min(ey_last);
                lastex = if vi { ex + 1 } else { ex };
                if self.rect {
                    let (a, c) = (s.anchor.col, other.col);
                    if a < c {
                        lastex = if vi { c + 1 } else { c };
                        restex = lastex;
                        firstsx = a;
                        restsx = a;
                    } else {
                        lastex = a + 1;
                        restex = a + 1;
                        firstsx = c;
                        restsx = c;
                    }
                }
            }
        }
        self.copy_rows(g, sy, ey, firstsx, restsx, lastex, restex)
    }

    /// tmux's window_copy_get_selection loop and final-newline rule.
    #[allow(clippy::too_many_arguments)]
    fn copy_rows(
        &self,
        g: &mut Grid,
        sy: usize,
        ey: usize,
        firstsx: usize,
        restsx: usize,
        lastex: usize,
        restex: usize,
    ) -> Option<String> {
        let mut buf = String::new();
        for y in sy..=ey {
            let sx = if y == sy { firstsx } else { restsx };
            let ex = if y == ey { lastex } else { restex };
            let wrapped = g.wrapped(y);
            let xx = if wrapped { self.cols } else { g.len(y) };
            let (sx, ex) = (sx.min(xx), ex.min(xx));
            for x in sx..ex {
                g.push_cell_text(y, x, &mut buf);
            }
            if !wrapped || ex != xx {
                buf.push('\n');
            }
        }
        if buf.is_empty() {
            return None;
        }
        let ey_last = g.len(ey);
        if (!self.vi() || lastex <= ey_last) && (!g.wrapped(ey) || lastex != ey_last) && buf.ends_with('\n') {
            buf.pop();
        }
        Some(buf)
    }

    // ----- search -------------------------------------------------------

    fn search_origin(&self, forward: bool) -> Point {
        // The emacs cursor sits after a forward match; searching backward
        // from there must not find the same match again.
        if !forward
            && !self.vi()
            && let Some(m) = self.search.current
            && self.cur > m.start
            && self.cur <= (Point { row: m.end.row, col: m.end.col + 1 })
        {
            return m.start;
        }
        self.cur
    }

    #[allow(clippy::too_many_arguments)]
    fn start_search(
        &mut self,
        g: &mut Grid,
        text: &str,
        regex: bool,
        forward: bool,
        n: u32,
        origin: Point,
        quiet: bool,
    ) -> Outcome {
        self.exit_at_bottom = false;
        self.search.last = Some((text.to_string(), regex, forward));
        if self.search.pat.as_ref().is_none_or(|p| p.text != text || p.regex != regex) {
            self.search.pat = Some(Pattern::new(text, regex));
        }
        self.search.show = true;
        self.search.quiet = quiet;
        self.search.job = Some(Job::new(g, origin, forward, n));
        let budget = self.cfg.search_step_rows;
        self.drive_search(g, budget)
    }

    fn drive_search(&mut self, g: &mut Grid, budget: usize) -> Outcome {
        let wrap = self.cfg.wrap_search;
        loop {
            let (Some(job), Some(pat)) = (self.search.job.as_mut(), self.search.pat.as_ref()) else {
                return Outcome::None;
            };
            match job.step(g, pat, &mut self.search.line, &mut self.search.scratch, budget, wrap) {
                Step::Pending => return Outcome::Searching,
                Step::NotFound => {
                    self.search.job = None;
                    if self.search.quiet {
                        return Outcome::Redraw;
                    }
                    let text = self.search.last.as_ref().map_or("", |l| l.0.as_str());
                    return Outcome::Message(format!("No match: {text}"));
                }
                Step::Found(m) => {
                    let (forward, remaining) = (job.forward, job.remaining);
                    self.search.current = Some(m);
                    self.cur = m.start;
                    self.scroll_to(m.start.row);
                    if remaining > 1 {
                        self.search.job = Some(Job::new(g, m.start, forward, remaining - 1));
                        continue;
                    }
                    self.search.job = None;
                    if forward && !self.vi() {
                        let w = g.width_at(m.end.row, m.end.col);
                        let c = m.end.col + 1 + (w - 1);
                        self.cur = if c < self.cols {
                            Point { row: m.end.row, col: m.end.col + 1 }
                        } else if m.end.row + 1 < self.total {
                            Point { row: m.end.row + 1, col: 0 }
                        } else {
                            m.end
                        };
                    }
                    let row = self.cur.row;
                    self.scroll_to(row);
                    self.cur.col = g.head(self.cur.row, self.cur.col);
                    return Outcome::Redraw;
                }
            }
        }
    }

    // ----- mouse --------------------------------------------------------

    fn mouse_point(&self, g: &mut Grid, p: Pos) -> Point {
        let row = self.top + (p.y as usize).min(self.rows - 1);
        let row = row.min(self.total - 1);
        let col = (p.x as usize).min(self.cols - 1);
        Point { row, col: g.head(row, col) }
    }

    fn mouse(&mut self, ev: &MouseEvent, now: u64, g: &mut Grid) -> MouseOut {
        match (ev.action, ev.button) {
            (MouseAction::Press, Some(MouseButton::WheelUp)) => {
                MouseOut::Run(Command::ScrollUp, self.cfg.mouse.wheel_lines)
            }
            (MouseAction::Press, Some(MouseButton::WheelDown)) => {
                MouseOut::Run(Command::ScrollDown, self.cfg.mouse.wheel_lines)
            }
            (MouseAction::Press, Some(MouseButton::Left)) => {
                let clicks = match self.mouse.last_press {
                    Some((p, t)) if p == ev.cell && now.saturating_sub(t) <= self.cfg.mouse.click_ms => {
                        self.mouse.clicks % 3 + 1
                    }
                    _ => 1,
                };
                self.mouse.clicks = clicks;
                self.mouse.last_press = Some((ev.cell, now));
                self.mouse.dragging = false;
                self.mouse.edge = 0;
                let p = self.mouse_point(g, ev.cell);
                self.mouse.press = Some(p);
                self.exit_at_bottom = false;
                match clicks {
                    1 => MouseOut::Done(Outcome::None),
                    _ => {
                        self.mouse.press = None;
                        self.cur = p;
                        if clicks == 2 {
                            self.rect = false;
                            self.select_word(g);
                        } else {
                            let _ = self.exec(g, Command::SelectLine, None, 1);
                        }
                        MouseOut::Done(self.mouse_copy(g))
                    }
                }
            }
            (MouseAction::Motion, Some(MouseButton::Left) | None) => {
                let Some(press) = self.mouse.press else { return MouseOut::Done(Outcome::None) };
                if !self.mouse.dragging {
                    self.mouse.dragging = true;
                    self.sel = Some(Sel { anchor: press, kind: SelKind::Char, end: None, ends: true });
                }
                let p = self.mouse_point(g, ev.cell);
                let before = (self.cur, self.top);
                self.cur = p;
                self.mouse.edge = if ev.cell.y == 0 && self.top > 0 {
                    -1
                } else if ev.cell.y as usize >= self.rows - 1 && self.top < self.history {
                    1
                } else {
                    0
                };
                if self.mouse.edge != 0 {
                    self.edge_scroll(self.mouse.edge < 0);
                    self.cur.col = g.head(self.cur.row, p.col);
                }
                if (self.cur, self.top) == before {
                    MouseOut::Done(Outcome::None)
                } else {
                    MouseOut::Done(Outcome::Redraw)
                }
            }
            (MouseAction::Release, _) => {
                self.mouse.press = None;
                self.mouse.edge = 0;
                if !std::mem::take(&mut self.mouse.dragging) {
                    return MouseOut::Done(Outcome::None);
                }
                MouseOut::Done(self.mouse_copy(g))
            }
            _ => MouseOut::Done(Outcome::None),
        }
    }

    fn mouse_copy(&mut self, g: &mut Grid) -> Outcome {
        match self.cfg.mouse.drag_end {
            DragEnd::Keep => Outcome::Redraw,
            end => match self.sel_text(g) {
                Some(t) => {
                    Outcome::Copy { data: t.into_bytes(), append: false, exit: end == DragEnd::CopyAndCancel, pipe: true }
                }
                None => Outcome::Redraw,
            },
        }
    }
}

enum MouseOut {
    Done(Outcome),
    Run(Command, u32),
}
