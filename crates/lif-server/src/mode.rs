//! Pane modes: something that takes over a pane's keys and adds overlays,
//! instead of the program in it. Copy mode is one; it lives in its own crate
//! (`lif-copy`) and plugs in here.
//!
//! # The seam
//!
//! While a pane has a mode, keys the key tables send to the pane go to
//! [`PaneMode::key`] instead of the program, mouse wheel events go to
//! [`PaneMode::scroll`], and every frame asks the mode for its overlays
//! ([`PaneMode::indicator`], [`PaneMode::selection`], [`PaneMode::cursor`]).
//! The mode moves the pane's viewport through [`ModeCtx::engine`]
//! (`VtEngine::scroll`, `text`, ...). [`new_copy_mode`] is the one
//! constructor the server calls; swapping in another implementation means
//! changing only that function.

use std::fmt::Write;

use lif_core::CopyModeOptions;
use lif_render::Selection;
use lif_types::{Key, KeyAction, KeyEvent, Mods, Pos};
use lif_vt::{Scroll, VtEngine};

/// The terminal engine type of every pane.
pub type Engine = lif_vt::GhosttyVt;

/// What a mode can touch.
pub struct ModeCtx<'a> {
    pub engine: &'a mut Engine,
    /// Rows of the pane.
    pub rows: u16,
}

/// What the server should do after the mode handled something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeAction {
    /// Stay in the mode (redraw the pane).
    Stay,
    /// Leave the mode.
    Exit,
    /// Store `data` as the top paste buffer (and send it to the outer
    /// clipboard per `set-clipboard`); leave the mode if `exit`.
    Copy { data: Vec<u8>, exit: bool },
}

pub trait PaneMode {
    /// A key the key tables sent to the pane.
    fn key(&mut self, key: &KeyEvent, ctx: &mut ModeCtx<'_>) -> ModeAction;
    /// Mouse wheel: positive lines scroll toward the newest output.
    fn scroll(&mut self, lines: i32, ctx: &mut ModeCtx<'_>) -> ModeAction;
    /// The pane got output or changed size.
    fn update(&mut self, _ctx: &mut ModeCtx<'_>) {}
    /// Apply `copy-mode` flags given while already in the mode.
    fn reenter(&mut self, _opts: CopyModeOptions, _ctx: &mut ModeCtx<'_>) {}
    /// Right-aligned text on the pane's first row (`[12/340]`).
    fn indicator(&self) -> Option<&str>;
    fn selection(&self) -> Option<Selection> {
        None
    }
    /// Cursor in pane coordinates; `None` hides it.
    fn cursor(&self) -> Option<Pos> {
        None
    }
    /// Leaving: put the viewport back on live output.
    fn leave(&mut self, ctx: &mut ModeCtx<'_>) {
        ctx.engine.scroll(Scroll::Bottom);
    }
}

/// The copy mode the server uses. Today a minimal scroll-back viewer.
pub fn new_copy_mode(opts: CopyModeOptions, ctx: &mut ModeCtx<'_>) -> Box<dyn PaneMode> {
    let mut m = ScrollMode { exit_at_bottom: opts.exit_at_bottom, hide_position: opts.hide_position, indicator: String::new() };
    if opts.page_up {
        ctx.engine.scroll(Scroll::Lines(-(page(ctx.rows))));
    }
    m.refresh(ctx);
    Box::new(m)
}

fn page(rows: u16) -> i32 {
    (rows as i32 - 1).max(1)
}

/// Scroll with arrows / PageUp / PageDown / g / G, leave with q or Escape.
struct ScrollMode {
    exit_at_bottom: bool,
    hide_position: bool,
    indicator: String,
}

impl ScrollMode {
    fn refresh(&mut self, ctx: &mut ModeCtx<'_>) {
        self.indicator.clear();
        let _ = write!(self.indicator, "[{}/{}]", ctx.engine.viewport_offset(), ctx.engine.scrollback_rows());
    }

    fn moved(&mut self, ctx: &mut ModeCtx<'_>) -> ModeAction {
        self.refresh(ctx);
        if self.exit_at_bottom && ctx.engine.viewport_offset() == 0 { ModeAction::Exit } else { ModeAction::Stay }
    }
}

impl PaneMode for ScrollMode {
    fn key(&mut self, key: &KeyEvent, ctx: &mut ModeCtx<'_>) -> ModeAction {
        if key.action == KeyAction::Release {
            return ModeAction::Stay;
        }
        let ctrl = key.mods.contains(Mods::CTRL);
        let to = match key.key {
            Key::Char('q') | Key::Escape => return ModeAction::Exit,
            Key::Char('c') if ctrl => return ModeAction::Exit,
            Key::Up | Key::Char('k') | Key::Char('y') => Scroll::Lines(-1),
            Key::Char('p') if ctrl => Scroll::Lines(-1),
            Key::Down | Key::Char('j') | Key::Char('e') => Scroll::Lines(1),
            Key::Char('n') if ctrl => Scroll::Lines(1),
            Key::PageUp => Scroll::Lines(-page(ctx.rows)),
            Key::Char('b') if ctrl => Scroll::Lines(-page(ctx.rows)),
            Key::PageDown | Key::Char(' ') => Scroll::Lines(page(ctx.rows)),
            Key::Char('f') if ctrl => Scroll::Lines(page(ctx.rows)),
            Key::Char('u') if ctrl => Scroll::Lines(-(ctx.rows as i32 / 2).max(1)),
            Key::Char('d') if ctrl => Scroll::Lines((ctx.rows as i32 / 2).max(1)),
            Key::Char('g') | Key::Home => Scroll::Top,
            Key::Char('G') | Key::End => Scroll::Bottom,
            _ => return ModeAction::Stay,
        };
        ctx.engine.scroll(to);
        self.moved(ctx)
    }

    fn scroll(&mut self, lines: i32, ctx: &mut ModeCtx<'_>) -> ModeAction {
        ctx.engine.scroll(Scroll::Lines(lines));
        self.moved(ctx)
    }

    fn update(&mut self, ctx: &mut ModeCtx<'_>) {
        self.refresh(ctx);
    }

    fn reenter(&mut self, opts: CopyModeOptions, ctx: &mut ModeCtx<'_>) {
        if opts.page_up {
            ctx.engine.scroll(Scroll::Lines(-page(ctx.rows)));
        }
        self.refresh(ctx);
    }

    fn indicator(&self) -> Option<&str> {
        (!self.hide_position).then_some(self.indicator.as_str())
    }
}
