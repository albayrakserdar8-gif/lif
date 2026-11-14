//! Pane modes: something that takes over a pane's keys and mouse and adds
//! overlays, instead of the program in it. Copy mode and view mode (the
//! read-only pager for long command output) are both lif-copy's
//! [`CopyMode`]; they differ in what they show: the pane's own terminal,
//! scrolled through its history, or text held in a [`MemoryView`].
//!
//! # The seam
//!
//! While a pane has a mode, keys the key tables send to the pane go to
//! [`PaneMode::key`] instead of the program (or run as commands, when the
//! copy-mode table binds them to something copy mode cannot do itself),
//! mouse events go to [`PaneMode::mouse`], `send-keys -X` to
//! [`PaneMode::command`], and every frame reads the overlays the mode keeps
//! up to date ([`PaneMode::overlays`]) and, in view mode, its own frame
//! ([`PaneMode::frame`]). Every method returns a [`ModeAction`] for the
//! server to carry out (copy to a buffer, show a prompt, keep searching...).
//! [`new_copy_mode`] and [`new_view_mode`] are the constructors the server
//! calls; swapping in another implementation means changing only those.

use std::cell::{Cell, RefCell};

use lif_copy::{Command, Config, CopyMode, Enter, MemoryView, Outcome, PaneView, SelectionShape};
use lif_core::CopyModeOptions;
use lif_render::{MatchSpan, Selection, SelectionKind};
use lif_types::{FrameRow, KeyEvent, MouseEvent, PaneFrame, Pos, Size};
use lif_vt::VtEngine;

/// The terminal engine type of every pane.
pub type Engine = lif_vt::GhosttyVt;

/// What a mode can touch.
pub struct ModeCtx<'a> {
    pub engine: &'a mut Engine,
}

/// The pane's terminal as copy mode reads it: scrollback and screen, row
/// by row, and its viewport.
///
/// Rows outside the viewport are read with `VtEngine::row_scan`, which
/// moves the viewport next to them: a search through a deep history then
/// costs about the same per row as one near the screen (see lif-vt's
/// bench). The viewport copy mode chose is put back when the view is
/// dropped, at the end of every call into the mode.
pub struct EngineView<'a> {
    engine: RefCell<&'a mut Engine>,
    /// Where the viewport belongs, once a read has moved it.
    restore: Cell<Option<usize>>,
}

impl<'a> EngineView<'a> {
    pub fn new(engine: &'a mut Engine) -> Self {
        EngineView { engine: RefCell::new(engine), restore: Cell::new(None) }
    }
}

impl Drop for EngineView<'_> {
    fn drop(&mut self) {
        if let Some(top) = self.restore.take() {
            self.engine.get_mut().set_viewport_top(top);
        }
    }
}

impl PaneView for EngineView<'_> {
    fn size(&self) -> Size {
        self.engine.borrow().size()
    }

    fn history_rows(&self) -> usize {
        self.engine.borrow().scrollback_rows()
    }

    fn row(&self, row: usize, out: &mut FrameRow) {
        let mut e = self.engine.borrow_mut();
        let top = e.viewport_top();
        if (top..top + e.size().rows as usize).contains(&row) {
            e.row(row, out);
            return;
        }
        if self.restore.get().is_none() {
            self.restore.set(Some(top));
        }
        e.row_scan(row, out);
    }

    fn find_prompt(&self, from: usize, forward: bool) -> Option<usize> {
        self.engine.borrow().find_prompt(from, forward)
    }

    fn trimmed_rows(&self) -> u64 {
        self.engine.borrow().trimmed_rows()
    }

    fn set_viewport_top(&mut self, top: usize) {
        self.restore.set(None);
        self.engine.get_mut().set_viewport_top(top);
    }
}

/// What the server should do after the mode handled something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeAction {
    /// Nothing changed.
    None,
    /// Redraw the pane.
    Redraw,
    /// Put `data` in a new paste buffer (or append it to the top one), send
    /// it to the outer clipboards per `set-clipboard`, pipe it to `pipe`
    /// (`copy-pipe`; empty: the `copy-command` option), and leave the mode
    /// if `exit`.
    Copy { data: Vec<u8>, append: bool, exit: bool, pipe: Option<String> },
    /// The key is bound to commands the mode does not run itself: run the
    /// binding (see `lif_copy::Outcome::Binding`).
    Binding,
    /// Leave the mode.
    Exit,
    /// Show a prompt and report its edits back (`prompt_*`).
    Prompt { label: &'static str, initial: String, incremental: bool },
    /// Show a message.
    Message(String),
    /// A search is under way: call [`PaneMode::search_step`] once per turn
    /// of the event loop, polling with a zero timeout in between.
    Searching,
}

impl From<Outcome> for ModeAction {
    fn from(o: Outcome) -> Self {
        match o {
            Outcome::None => ModeAction::None,
            Outcome::Redraw => ModeAction::Redraw,
            Outcome::Copy { data, append, exit, pipe } => {
                ModeAction::Copy { data, append, exit, pipe: pipe.then(String::new) }
            }
            Outcome::Binding => ModeAction::Binding,
            Outcome::Exit => ModeAction::Exit,
            Outcome::Prompt { label, initial, incremental } => ModeAction::Prompt { label, initial, incremental },
            Outcome::Message(m) => ModeAction::Message(m),
            Outcome::Searching => ModeAction::Searching,
        }
    }
}

/// What a frame draws over a pane in a mode, kept up to date by the mode
/// so that drawing only reads it.
#[derive(Clone, Debug, Default)]
pub struct Overlays {
    pub selection: Option<Selection>,
    /// The mode's cursor, in pane coordinates.
    pub cursor: Pos,
    /// `[offset/history]`; empty when hidden.
    pub indicator: String,
    pub matches: Vec<MatchSpan>,
}

pub trait PaneMode {
    /// A key the key tables sent to the pane.
    fn key(&mut self, key: &KeyEvent, ctx: &mut ModeCtx<'_>) -> ModeAction;
    /// A mouse event in pane coordinates; `now_ms` tells double clicks.
    fn mouse(&mut self, ev: &MouseEvent, now_ms: u64, ctx: &mut ModeCtx<'_>) -> ModeAction;
    /// `send-keys -X name [arg]`, `count` times.
    fn command(&mut self, name: &str, arg: Option<&str>, count: u32, ctx: &mut ModeCtx<'_>) -> Result<ModeAction, String>;
    /// The prompt's text changed (incremental search).
    fn prompt_changed(&mut self, text: &str, ctx: &mut ModeCtx<'_>) -> ModeAction;
    /// C-s / C-r inside an incremental prompt.
    fn prompt_again(&mut self, forward: bool, ctx: &mut ModeCtx<'_>) -> ModeAction;
    fn prompt_accept(&mut self, text: &str, ctx: &mut ModeCtx<'_>) -> ModeAction;
    fn prompt_cancel(&mut self, ctx: &mut ModeCtx<'_>) -> ModeAction;
    /// Continue a search, examining about `budget_rows` rows.
    fn search_step(&mut self, budget_rows: usize, ctx: &mut ModeCtx<'_>) -> ModeAction;
    fn searching(&self) -> bool;
    /// A drag rests on the pane's top or bottom edge: call
    /// [`PaneMode::autoscroll_tick`] every 50 ms until it stops.
    fn autoscroll_active(&self) -> bool;
    fn autoscroll_tick(&mut self, ctx: &mut ModeCtx<'_>) -> ModeAction;
    /// The pane got output or changed size.
    fn update(&mut self, ctx: &mut ModeCtx<'_>);
    /// `copy-mode` again while in the mode.
    fn reenter(&mut self, opts: CopyModeOptions, ctx: &mut ModeCtx<'_>) -> ModeAction;
    /// New settings (mode-keys, key table, word separators...).
    fn set_config(&mut self, cfg: &Config, ctx: &mut ModeCtx<'_>);
    /// The mode waits for a key of its own (a jump target, or digits of a
    /// count): keys go straight to [`PaneMode::key`].
    fn pending(&self) -> bool;
    /// Read-only output (view mode): q and Escape leave.
    fn is_view(&self) -> bool;
    fn overlays(&self) -> &Overlays;
    /// What to draw instead of the pane's terminal, if anything.
    fn frame(&self) -> Option<&PaneFrame>;
    /// After a frame tick, like `PaneFrame::clear_dirty` on the pane's own.
    fn clear_frame_dirty(&mut self);
    /// Leaving: put the viewport back on live output.
    fn leave(&mut self, ctx: &mut ModeCtx<'_>);
}

/// Text shown by view mode, laid out at the pane's width.
struct Memory {
    text: String,
    view: MemoryView,
    frame: PaneFrame,
    /// Viewport top the frame was built for.
    shown: Option<usize>,
}

impl Memory {
    fn new(text: String, size: Size) -> Memory {
        let mut m = Memory { text, view: MemoryView::new(size.cols, size.rows), frame: PaneFrame::new(size), shown: None };
        m.layout(size);
        m
    }

    fn layout(&mut self, size: Size) {
        let lines: Vec<&str> = self.text.lines().collect();
        let top = self.view.viewport_top;
        self.view = MemoryView::from_lines(size.cols.max(1), size.rows.max(1), &lines);
        self.view.viewport_top = top.min(self.view.history_rows());
        self.frame = PaneFrame::new(size);
        self.shown = None;
    }

    /// Rebuild the frame when the viewport moved.
    fn sync_frame(&mut self) {
        let top = self.view.viewport_top;
        if self.shown == Some(top) {
            return;
        }
        self.shown = Some(top);
        for (y, row) in self.frame.rows.iter_mut().enumerate() {
            self.view.row(top + y, row);
        }
        self.frame.mark_all_dirty();
    }
}

enum Source {
    Engine,
    Memory(Box<Memory>),
}

/// Copy mode (or view mode) on one pane.
struct CopyPane {
    copy: CopyMode,
    src: Source,
    ov: Overlays,
}

/// Run `f` with the mode and the view it works on. A free function so the
/// borrows of the mode and the source stay apart.
fn with_view<R>(
    copy: &mut CopyMode,
    src: &mut Source,
    ctx: &mut ModeCtx<'_>,
    f: impl FnOnce(&mut CopyMode, &mut dyn PaneView) -> R,
) -> R {
    match src {
        Source::Engine => f(copy, &mut EngineView::new(ctx.engine)),
        Source::Memory(m) => f(copy, &mut m.view),
    }
}

impl CopyPane {
    fn run<R>(&mut self, ctx: &mut ModeCtx<'_>, f: impl FnOnce(&mut CopyMode, &mut dyn PaneView) -> R) -> R {
        let r = with_view(&mut self.copy, &mut self.src, ctx, f);
        self.refresh_overlays(ctx);
        r
    }

    fn refresh_overlays(&mut self, ctx: &mut ModeCtx<'_>) {
        let ov = &mut self.ov;
        with_view(&mut self.copy, &mut self.src, ctx, |copy, view| {
            let o = copy.overlays(view);
            ov.selection = o.selection.map(|s| Selection {
                start: s.start,
                end: s.end,
                kind: match s.shape {
                    SelectionShape::Stream => SelectionKind::Stream,
                    SelectionShape::Lines => SelectionKind::Lines,
                    SelectionShape::Rect => SelectionKind::Rect,
                },
            });
            ov.cursor = o.cursor;
            ov.indicator.clear();
            ov.indicator.push_str(o.indicator.unwrap_or(""));
            ov.matches.clear();
            ov.matches.extend(o.matches.iter().map(|m| MatchSpan { y: m.y, x0: m.x0, x1: m.x1, current: m.current }));
        });
        if let Source::Memory(m) = &mut self.src {
            m.sync_frame();
        }
    }
}

impl PaneMode for CopyPane {
    fn key(&mut self, key: &KeyEvent, ctx: &mut ModeCtx<'_>) -> ModeAction {
        self.run(ctx, |c, v| c.handle_key(key, v)).into()
    }

    fn mouse(&mut self, ev: &MouseEvent, now_ms: u64, ctx: &mut ModeCtx<'_>) -> ModeAction {
        self.run(ctx, |c, v| c.handle_mouse(ev, now_ms, v)).into()
    }

    fn command(&mut self, name: &str, arg: Option<&str>, count: u32, ctx: &mut ModeCtx<'_>) -> Result<ModeAction, String> {
        let cmd = Command::from_name(name).ok_or_else(|| format!("unknown command: {name}"))?;
        Ok(self.run(ctx, |c, v| c.run_command(v, cmd, arg, count)).into())
    }

    fn prompt_changed(&mut self, text: &str, ctx: &mut ModeCtx<'_>) -> ModeAction {
        self.run(ctx, |c, v| c.prompt_changed(v, text)).into()
    }

    fn prompt_again(&mut self, forward: bool, ctx: &mut ModeCtx<'_>) -> ModeAction {
        self.run(ctx, |c, v| c.prompt_again(v, forward)).into()
    }

    fn prompt_accept(&mut self, text: &str, ctx: &mut ModeCtx<'_>) -> ModeAction {
        self.run(ctx, |c, v| c.prompt_accept(v, text)).into()
    }

    fn prompt_cancel(&mut self, ctx: &mut ModeCtx<'_>) -> ModeAction {
        self.run(ctx, |c, v| c.prompt_cancel(v)).into()
    }

    fn search_step(&mut self, budget_rows: usize, ctx: &mut ModeCtx<'_>) -> ModeAction {
        self.run(ctx, |c, v| c.search_step(v, budget_rows)).into()
    }

    fn searching(&self) -> bool {
        self.copy.searching()
    }

    fn autoscroll_active(&self) -> bool {
        self.copy.autoscroll_active()
    }

    fn autoscroll_tick(&mut self, ctx: &mut ModeCtx<'_>) -> ModeAction {
        self.run(ctx, |c, v| c.autoscroll_tick(v)).into()
    }

    fn update(&mut self, ctx: &mut ModeCtx<'_>) {
        if let Source::Memory(m) = &mut self.src {
            let size = ctx.engine.size();
            if m.view.size != size {
                m.layout(size);
            }
        }
        self.run(ctx, |c, v| c.refresh(v));
    }

    fn reenter(&mut self, opts: CopyModeOptions, ctx: &mut ModeCtx<'_>) -> ModeAction {
        let mut out = ModeAction::None;
        if opts.exit_at_bottom {
            out = self.run(ctx, |c, v| c.run_command(v, Command::ScrollExitOn, None, 1)).into();
        }
        if opts.page_up {
            out = self.run(ctx, |c, v| c.run_command(v, Command::PageUp, None, 1)).into();
        }
        if opts.page_down {
            out = self.run(ctx, |c, v| c.run_command(v, Command::PageDown, None, 1)).into();
        }
        out
    }

    fn set_config(&mut self, cfg: &Config, ctx: &mut ModeCtx<'_>) {
        self.copy.set_config(cfg.clone());
        self.refresh_overlays(ctx);
    }

    fn pending(&self) -> bool {
        self.copy.pending_label().is_some() || self.copy.pending_count().is_some()
    }

    fn is_view(&self) -> bool {
        matches!(self.src, Source::Memory(_))
    }

    fn overlays(&self) -> &Overlays {
        &self.ov
    }

    fn frame(&self) -> Option<&PaneFrame> {
        match &self.src {
            Source::Memory(m) => Some(&m.frame),
            Source::Engine => None,
        }
    }

    fn clear_frame_dirty(&mut self) {
        if let Source::Memory(m) = &mut self.src {
            m.frame.clear_dirty();
        }
    }

    fn leave(&mut self, ctx: &mut ModeCtx<'_>) {
        if let Source::Engine = self.src {
            let mut v = EngineView::new(ctx.engine);
            self.copy.leave(&mut v);
        }
    }
}

fn enter(opts: CopyModeOptions) -> Enter {
    Enter { page_up: opts.page_up, exit_at_bottom: opts.exit_at_bottom, hide_position: opts.hide_position }
}

/// Copy mode on the pane's terminal, the cursor starting where the
/// program's is (`cursor`, screen coordinates).
pub fn new_copy_mode(opts: CopyModeOptions, cfg: &Config, cursor: Pos, ctx: &mut ModeCtx<'_>) -> Box<dyn PaneMode> {
    let copy = CopyMode::new(&mut EngineView::new(ctx.engine), cfg.clone(), cursor, enter(opts));
    let mut m = CopyPane { copy, src: Source::Engine, ov: Overlays::default() };
    m.refresh_overlays(ctx);
    Box::new(m)
}

/// View mode: `text` in a read-only copy mode laid out at the pane's
/// size, starting at its first line, like tmux shows command output.
pub fn new_view_mode(text: String, cfg: &Config, ctx: &mut ModeCtx<'_>) -> Box<dyn PaneMode> {
    let mut mem = Box::new(Memory::new(text, ctx.engine.size()));
    let enter = Enter { hide_position: false, ..Enter::default() };
    let mut copy = CopyMode::new(&mut mem.view, cfg.clone(), Pos { x: 0, y: 0 }, enter);
    copy.run_command(&mut mem.view, Command::HistoryTop, None, 1);
    let mut m = CopyPane { copy, src: Source::Memory(mem), ov: Overlays::default() };
    m.refresh_overlays(ctx);
    Box::new(m)
}
