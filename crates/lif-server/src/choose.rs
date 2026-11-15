//! The chooser modes in a pane: `choose-tree`, `choose-buffer`,
//! `choose-client`, `find-window`.
//!
//! `lif_core::ChooseState` is the mode; this is its [`PaneMode`]: it
//! draws the state's screen into a frame of its own (the list, and below
//! it a box with a preview of what is selected) and turns what the state
//! asks for into [`ModeAction`]s. Drawing needs the other panes (the
//! preview shows their terminals), so the server does it
//! (`Server::refresh_choosers`), once before a frame when something
//! changed, never per frame.

use std::collections::HashMap;

use lif_copy::Config;
use lif_core::{ChooseOutcome, ChooseState, ClientId, CopyModeOptions, PaneId, Preview};
use lif_render::char_width;
use lif_types::{Cell, CellWidth, FrameRow, KeyEvent, MouseAction, MouseButton, MouseEvent, PaneFrame, Pos, Size, Style};
use lif_vt::VtEngine;

use crate::mode::{ModeAction, ModeCtx, Overlays, PaneMode};
use crate::pane::PaneRt;

/// Two clicks on a line this close together choose it.
const DOUBLE_CLICK_MS: u64 = 400;

/// Narrowest column a previewed pane gets; panes that do not fit at this
/// width are left out of the preview.
const MIN_PREVIEW_COLS: usize = 16;

pub struct ChoosePane {
    pub(crate) state: ChooseState,
    /// The client the mode was opened for: its choices run as that client.
    pub(crate) client: ClientId,
    frame: PaneFrame,
    ov: Overlays,
    /// The state changed since the frame was drawn.
    dirty: bool,
    last_click: Option<(u64, u16)>,
}

impl ChoosePane {
    pub(crate) fn new(state: ChooseState, client: ClientId, size: Size) -> ChoosePane {
        // The mode has no cursor: one outside the pane is not drawn.
        let ov = Overlays { cursor: Pos { x: u16::MAX, y: u16::MAX }, ..Overlays::default() };
        ChoosePane { state, client, frame: PaneFrame::new(size), ov, dirty: true, last_click: None }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Whether the frame is out of date for a pane of `size`.
    pub(crate) fn needs_draw(&self, size: Size) -> bool {
        self.dirty || self.frame.size != size
    }

    fn act(&mut self, o: ChooseOutcome) -> ModeAction {
        self.dirty = true;
        match o {
            ChooseOutcome::None => {
                self.dirty = false;
                ModeAction::None
            }
            ChooseOutcome::Redraw => ModeAction::Redraw,
            ChooseOutcome::Choose(command) => ModeAction::Run { commands: vec![command], exit: true },
            ChooseOutcome::Run(commands) => ModeAction::Run { commands, exit: false },
            ChooseOutcome::Prompt { label } => ModeAction::Ask { label, single_key: self.state.confirming() },
            ChooseOutcome::Rebuild => ModeAction::Rebuild,
            ChooseOutcome::Exit => ModeAction::Exit,
        }
    }

    /// Draw the mode for a pane of `size`; `panes` are the panes a
    /// preview may show (every pane but the mode's own, which the caller
    /// holds), `selected` the style of the selected line.
    pub(crate) fn draw(&mut self, size: Size, panes: &HashMap<PaneId, PaneRt>, selected: Style) {
        self.dirty = false;
        self.frame.reset(size);
        let (cols, rows) = (size.cols as usize, size.rows as usize);
        if cols == 0 || rows == 0 {
            return;
        }
        let screen = self.state.screen(size.cols, size.rows);
        for (y, line) in screen.lines.iter().enumerate().take(rows) {
            let row = &mut self.frame.rows[y];
            let style = if line.selected { selected } else { Style::default() };
            if line.selected {
                for c in &mut row.cells {
                    c.style = style;
                }
            }
            put_str(row, 0, cols, &line.text, style);
        }
        self.draw_preview(size, panes, screen.title, screen.list_rows as usize, screen.preview);
        if let Some(help) = screen.help {
            draw_help(&mut self.frame, &help, cols, rows);
        }
    }

    /// The box under the list and what it shows of the selected item.
    fn draw_preview(&mut self, size: Size, panes: &HashMap<PaneId, PaneRt>, title: Option<String>, top: usize, preview: Preview) {
        let (cols, rows) = (size.cols as usize, size.rows as usize);
        let Some(title) = title else { return };
        if rows < top + 2 || cols < 4 {
            return;
        }
        let bottom = rows - 1;
        let plain = Style::default();
        // The box: its title on the top border, as tmux draws it.
        for (y, row) in self.frame.rows.iter_mut().enumerate().take(bottom + 1).skip(top) {
            let (l, r, fill) = match y {
                y if y == top => ('┌', '┐', '─'),
                y if y == bottom => ('└', '┘', '─'),
                _ => ('│', '│', ' '),
            };
            for c in &mut row.cells[1..cols - 1] {
                c.ch = fill;
            }
            row.cells[0].ch = l;
            row.cells[cols - 1].ch = r;
        }
        let head = format!(" {title} ");
        put_str(&mut self.frame.rows[top], 1, cols - 1, &head, plain);
        // Inside, one cell of padding on each side.
        let (x0, w, y0, h) = (2, cols - 4, top + 1, bottom - top - 1);
        if w == 0 || h == 0 {
            return;
        }
        match preview {
            Preview::None => {}
            Preview::Text(text) => {
                for (i, line) in text.lines().take(h).enumerate() {
                    let clean: String = line.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
                    put_str(&mut self.frame.rows[y0 + i], x0, x0 + w, &clean, plain);
                }
            }
            Preview::Panes(list) => {
                // As many as fit side by side, each a column of the box.
                let n = list.len().clamp(1, (w / MIN_PREVIEW_COLS).max(1));
                let each = w / n;
                for (i, (pane, label)) in list.iter().take(n).enumerate() {
                    let (cx, cw) = if i == 0 { (x0, each) } else { (x0 + i * each + 1, each - 1) };
                    if i > 0 {
                        for row in &mut self.frame.rows[y0..y0 + h] {
                            row.cells[x0 + i * each].ch = '│';
                        }
                    }
                    if let Some(src) = panes.get(pane) {
                        copy_pane(&mut self.frame, src, cx, cw, y0, h);
                    }
                    draw_label(&mut self.frame, label, cx, cw, y0, h);
                }
            }
        }
    }
}

/// The key help: a box in the middle of the pane, keys on the left of a
/// rule and what they do on the right, as tmux draws it. What does not
/// fit is cut.
fn draw_help(frame: &mut PaneFrame, help: &[(&str, &str)], cols: usize, rows: usize) {
    let kw = help.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let dw = help.iter().map(|(_, d)| d.len()).max().unwrap_or(0);
    // │ keys │ text │
    let w = (kw + dw + 7).min(cols);
    let h = (help.len() + 2).min(rows);
    if w < kw + 8 || h < 3 {
        return;
    }
    let (x0, y0) = ((cols - w) / 2, (rows - h) / 2);
    let plain = Style::default();
    for (i, row) in frame.rows.iter_mut().enumerate().skip(y0).take(h) {
        let edge = i == y0 || i + 1 == y0 + h;
        let (l, r, fill) = match i {
            i if i == y0 => ('┌', '┐', '─'),
            _ if edge => ('└', '┘', '─'),
            _ => ('│', '│', ' '),
        };
        for c in &mut row.cells[x0..x0 + w] {
            *c = Cell { ch: fill, ..Cell::BLANK };
        }
        row.cells[x0].ch = l;
        row.cells[x0 + w - 1].ch = r;
        if !edge {
            let (keys, text) = help[i - y0 - 1];
            put_str(row, x0 + 2 + (kw - keys.len()), x0 + 2 + kw, keys, plain);
            row.cells[x0 + kw + 3].ch = '│';
            put_str(row, x0 + kw + 5, x0 + w - 2, text, plain);
        }
    }
}

/// Write `s` into `row` from column `x`, stopping before `end`.
fn put_str(row: &mut FrameRow, mut x: usize, end: usize, s: &str, style: Style) {
    for ch in s.chars() {
        let w = char_width(ch);
        if w == 0 {
            continue;
        }
        if x + w > end.min(row.cells.len()) {
            break;
        }
        row.cells[x] = Cell { ch, style, width: if w == 2 { CellWidth::Wide } else { CellWidth::Narrow }, ..Cell::BLANK };
        if w == 2 {
            row.cells[x + 1] = Cell { style, width: CellWidth::WideTail, ..Cell::BLANK };
        }
        x += w;
    }
}

/// The part of `src`'s screen around its cursor, `w` by `h` cells, at
/// (`x0`, `y0`) of `frame`: tmux's `screen_write_preview`.
fn copy_pane(frame: &mut PaneFrame, src: &PaneRt, x0: usize, w: usize, y0: usize, h: usize) {
    let size = src.engine.size();
    let (sx, sy) = (size.cols as usize, size.rows as usize);
    let cursor = src.frame.cursor.pos.unwrap_or_default();
    let origin = |c: usize, n: usize, s: usize| {
        let p = c.saturating_sub(n / 3);
        if p + n > s { s.saturating_sub(n) } else { p }
    };
    let (px, py) = (origin(cursor.x as usize, w, sx), origin(cursor.y as usize, h, sy));
    let top = src.engine.viewport_top();
    let mut line = FrameRow::default();
    for y in 0..h.min(sy.saturating_sub(py)) {
        src.engine.row(top + py + y, &mut line);
        let out = &mut frame.rows[y0 + y];
        for x in 0..w.min(sx.saturating_sub(px)) {
            let Some(c) = line.cells.get(px + x) else { break };
            // Half of a wide character at either edge is left blank, and
            // clusters keep their first codepoint: the preview is a
            // picture of the pane, not a copy of it.
            let cut = (c.width == CellWidth::WideTail && x == 0) || (c.width == CellWidth::Wide && x + 1 == w);
            out.cells[x0 + x] = if cut { Cell::BLANK } else { Cell { extra: 0, link: 0, ..*c } };
        }
    }
}

/// `label` in a box in the middle of a preview column, if it fits.
fn draw_label(frame: &mut PaneFrame, label: &str, x0: usize, w: usize, y0: usize, h: usize) {
    let len: usize = label.chars().map(char_width).sum();
    if label.is_empty() || len + 4 > w || h < 3 {
        return;
    }
    let (bx, by) = (x0 + (w - (len + 4)) / 2, y0 + (h / 2).saturating_sub(1));
    let plain = Style::default();
    let line = "─".repeat(len + 2);
    put_str(&mut frame.rows[by], bx, bx + len + 4, &format!("┌{line}┐"), plain);
    put_str(&mut frame.rows[by + 1], bx, bx + len + 4, &format!("│ {label} │"), plain);
    put_str(&mut frame.rows[by + 2], bx, bx + len + 4, &format!("└{line}┘"), plain);
}

impl PaneMode for ChoosePane {
    fn key(&mut self, key: &KeyEvent, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        let o = self.state.key(key);
        self.act(o)
    }

    fn mouse(&mut self, ev: &MouseEvent, now_ms: u64, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        if ev.action != MouseAction::Press {
            return ModeAction::None;
        }
        let o = match ev.button {
            Some(MouseButton::WheelUp) => self.state.mouse(None, false, -1),
            Some(MouseButton::WheelDown) => self.state.mouse(None, false, 1),
            Some(MouseButton::Left) => {
                let double = self.last_click.is_some_and(|(t, y)| y == ev.cell.y && now_ms.saturating_sub(t) <= DOUBLE_CLICK_MS);
                self.last_click = Some((now_ms, ev.cell.y));
                self.state.mouse(Some(ev.cell.y), double, 0)
            }
            _ => ChooseOutcome::None,
        };
        self.act(o)
    }

    fn command(&mut self, name: &str, _arg: Option<&str>, _count: u32, _ctx: &mut ModeCtx<'_>) -> Result<ModeAction, String> {
        // tmux's tree mode takes no `send-keys -X` commands either,
        // except to leave.
        match name {
            "cancel" => Ok(ModeAction::Exit),
            _ => Err(format!("unknown command: {name}")),
        }
    }

    fn prompt_changed(&mut self, _text: &str, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        ModeAction::None
    }

    fn prompt_again(&mut self, _forward: bool, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        ModeAction::None
    }

    fn prompt_accept(&mut self, text: &str, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        let o = self.state.prompt_accept(text);
        self.act(o)
    }

    fn prompt_cancel(&mut self, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        let o = self.state.prompt_cancel();
        self.act(o)
    }

    fn search_step(&mut self, _budget_rows: usize, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        ModeAction::None
    }

    fn searching(&self) -> bool {
        false
    }

    fn autoscroll_active(&self) -> bool {
        false
    }

    fn autoscroll_tick(&mut self, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        ModeAction::None
    }

    fn update(&mut self, _ctx: &mut ModeCtx<'_>) {}

    fn reenter(&mut self, _opts: CopyModeOptions, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        ModeAction::None
    }

    fn set_config(&mut self, _cfg: &Config, _ctx: &mut ModeCtx<'_>) {}

    /// Every key is the mode's: none goes through the copy-mode table.
    fn pending(&self) -> bool {
        true
    }

    /// It draws its own frame, like view mode.
    fn is_view(&self) -> bool {
        true
    }

    fn overlays(&self) -> &Overlays {
        &self.ov
    }

    fn frame(&self) -> Option<&PaneFrame> {
        Some(&self.frame)
    }

    fn clear_frame_dirty(&mut self) {
        self.frame.clear_dirty();
    }

    fn leave(&mut self, _ctx: &mut ModeCtx<'_>) {}

    fn chooser(&mut self) -> Option<&mut ChoosePane> {
        Some(self)
    }
}
