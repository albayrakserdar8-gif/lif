//! `clock-mode`: a large clock in a pane (tmux's digits), until a key is
//! pressed there. The server draws it when the minute or the pane's size
//! changes (`Server::refresh_clocks`), on a deadline that exists only
//! while a pane shows a clock.

use lif_copy::Config;
use lif_core::{CopyModeOptions, WallClock};
use lif_types::{Cell, KeyAction, KeyEvent, Key, MouseEvent, PaneFrame, Pos, Size, Style};

use crate::mode::{ModeAction, ModeCtx, Overlays, PaneMode};

/// tmux's 5x5 digits (window-clock.c): 0-9, then `:`, `A`, `P`, `M`.
const GLYPHS: [[u8; 5]; 14] = [
    [0b11111, 0b10001, 0b10001, 0b10001, 0b11111],
    [0b00001, 0b00001, 0b00001, 0b00001, 0b00001],
    [0b11111, 0b00001, 0b11111, 0b10000, 0b11111],
    [0b11111, 0b00001, 0b11111, 0b00001, 0b11111],
    [0b10001, 0b10001, 0b11111, 0b00001, 0b00001],
    [0b11111, 0b10000, 0b11111, 0b00001, 0b11111],
    [0b11111, 0b10000, 0b11111, 0b10001, 0b11111],
    [0b11111, 0b00001, 0b00001, 0b00001, 0b00001],
    [0b11111, 0b10001, 0b11111, 0b10001, 0b11111],
    [0b11111, 0b10001, 0b11111, 0b00001, 0b11111],
    [0b00000, 0b00100, 0b00000, 0b00100, 0b00000],
    [0b11111, 0b10001, 0b11111, 0b10001, 0b10001],
    [0b11111, 0b10001, 0b11111, 0b10000, 0b10000],
    [0b10001, 0b11011, 0b10101, 0b10001, 0b10001],
];

fn glyph(c: char) -> Option<&'static [u8; 5]> {
    let i = match c {
        '0'..='9' => c as usize - '0' as usize,
        ':' => 10,
        'A' => 11,
        'P' => 12,
        'M' => 13,
        _ => return None,
    };
    Some(&GLYPHS[i])
}

/// The time as the clock shows it: `20:42`, or `8:42 PM`.
pub(crate) fn clock_text(now: WallClock, twelve: bool) -> String {
    let local = (now.unix_secs + now.utc_offset_secs as i64).rem_euclid(86_400);
    let (h, m) = (local / 3600, local % 3600 / 60);
    if twelve {
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        format!("{h12}:{m:02} {}", if h < 12 { "AM" } else { "PM" })
    } else {
        format!("{h:02}:{m:02}")
    }
}

pub struct ClockPane {
    frame: PaneFrame,
    ov: Overlays,
    /// What the frame shows.
    shown: String,
}

impl ClockPane {
    pub(crate) fn new(size: Size) -> ClockPane {
        let ov = Overlays { cursor: Pos { x: u16::MAX, y: u16::MAX }, ..Overlays::default() };
        ClockPane { frame: PaneFrame::new(size), ov, shown: String::new() }
    }

    /// Whether the frame is out of date for `text` in a pane of `size`.
    pub(crate) fn needs_draw(&self, text: &str, size: Size) -> bool {
        self.shown != text || self.frame.size != size
    }

    /// Draw `text` in the middle of a pane of `size`: in big digits if
    /// they fit, as plain text if not (tmux's rule and placement).
    pub(crate) fn draw(&mut self, text: &str, size: Size, style: Style) {
        self.shown.clear();
        self.shown.push_str(text);
        self.frame.reset(size);
        let (sx, sy) = (size.cols as usize, size.rows as usize);
        let n = text.chars().count();
        if sx < 6 * n || sy < 6 {
            if sx >= n && sy >= 1 {
                let (x, y) = (sx / 2 - n / 2, sy / 2);
                for (i, ch) in text.chars().enumerate() {
                    self.frame.rows[y].cells[x + i] = Cell { ch, style, ..Cell::BLANK };
                }
            }
            return;
        }
        let (x0, y0) = (sx / 2 - 3 * n, sy / 2 - 3);
        for (i, c) in text.chars().enumerate() {
            let Some(g) = glyph(c) else { continue };
            for (row, bits) in g.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0b10000 >> col) != 0 {
                        self.frame.rows[y0 + row].cells[x0 + 6 * i + col] = Cell { ch: '#', style, ..Cell::BLANK };
                    }
                }
            }
        }
    }
}

impl PaneMode for ClockPane {
    /// Any key ends the clock, as in tmux.
    fn key(&mut self, key: &KeyEvent, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        if key.action == KeyAction::Release || matches!(key.key, Key::Modifier(_)) {
            return ModeAction::None;
        }
        ModeAction::Exit
    }

    fn mouse(&mut self, _ev: &MouseEvent, _now_ms: u64, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        ModeAction::None
    }

    fn command(&mut self, name: &str, _arg: Option<&str>, _count: u32, _ctx: &mut ModeCtx<'_>) -> Result<ModeAction, String> {
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

    fn prompt_accept(&mut self, _text: &str, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        ModeAction::None
    }

    fn prompt_cancel(&mut self, _ctx: &mut ModeCtx<'_>) -> ModeAction {
        ModeAction::None
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

    fn pending(&self) -> bool {
        true
    }

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

    fn clock(&mut self) -> Option<&mut ClockPane> {
        Some(self)
    }
}
