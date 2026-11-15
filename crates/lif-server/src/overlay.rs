//! Popups and menus on a client's screen (`display-popup`,
//! `display-menu`).
//!
//! A popup's program runs in a pane only the server knows: it is in
//! `Rt::panes` (so reading, writing, reaping and the kill on exit are the
//! ordinary pane paths) under an id the model never hands out, and the
//! client's view gets it as one more pane, last, over the others. Its
//! border, and a whole menu, are frames drawn here; the renderer needs
//! nothing new, because a later pane of a scene covers the earlier ones.

use lif_core::{BorderLines, MenuItem, MenuState, PaneId, PopupClose};
use lif_render::char_width;
use lif_types::{Cell, CellWidth, FrameRow, PaneFrame, Rect, Size, Style};

/// Pane ids from here up are popups: far above any the model allocates.
pub(crate) const POPUP_PANE_BASE: u32 = 0xF000_0000;

/// Scene ids of a popup's border and of a menu (pane ids are below 2^32).
pub(crate) const BORDER_SCENE_ID: u64 = 1 << 40;
pub(crate) const MENU_SCENE_ID: u64 = (1 << 40) + 1;

/// Scrollback of a popup's terminal: it is a transient view.
pub(crate) const POPUP_SCROLLBACK_BYTES: usize = 256 * 1024;

pub(crate) struct Popup {
    /// The hidden pane running its program.
    pub(crate) pane: PaneId,
    /// On the client's screen, border included.
    pub(crate) rect: Rect,
    /// Where the program's terminal is drawn.
    pub(crate) inner: Rect,
    /// The border, a frame of `rect`'s size; `None` without one (`-B`).
    pub(crate) border: Option<PaneFrame>,
    /// The model's id for the commands waiting for the popup.
    pub(crate) job: u64,
    pub(crate) close: PopupClose,
    pub(crate) any_key: bool,
    /// The program has exited (the popup stays until a key).
    pub(crate) exited: bool,
}

pub(crate) struct Menu {
    pub(crate) state: MenuState,
    pub(crate) frame: PaneFrame,
}

pub(crate) enum Overlay {
    Popup(Popup),
    Menu(Menu),
}

impl Overlay {
    /// After a frame: like `PaneFrame::clear_dirty` on a pane's.
    pub(crate) fn clear_dirty(&mut self) {
        match self {
            Overlay::Popup(p) => {
                if let Some(f) = p.border.as_mut() {
                    f.clear_dirty();
                }
            }
            Overlay::Menu(m) => m.frame.clear_dirty(),
        }
    }
}

fn put(row: &mut FrameRow, mut x: usize, end: usize, s: &str, style: Style) {
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

/// A box of `size` drawn with `lines` in `style`, filled with blanks in
/// `fill`, `title` on its top edge from the third cell (tmux's popup
/// title).
pub(crate) fn box_frame(size: Size, lines: BorderLines, title: &str, style: Style, fill: Style) -> PaneFrame {
    let mut f = PaneFrame::new(size);
    let (w, h) = (size.cols as usize, size.rows as usize);
    if w < 2 || h < 2 {
        return f;
    }
    let [tl, tr, bl, br, hz, vt, _, _] = lines.glyphs();
    let cell = |ch: char| Cell { ch, style, ..Cell::BLANK };
    for (y, row) in f.rows.iter_mut().enumerate() {
        for c in row.cells.iter_mut() {
            *c = Cell { style: fill, ..Cell::BLANK };
        }
        let (l, r, mid) = match y {
            0 => (tl, tr, Some(hz)),
            y if y == h - 1 => (bl, br, Some(hz)),
            _ => (vt, vt, None),
        };
        if let Some(m) = mid {
            for c in &mut row.cells[1..w - 1] {
                *c = cell(m);
            }
        }
        row.cells[0] = cell(l);
        row.cells[w - 1] = cell(r);
    }
    if !title.is_empty() && w > 4 {
        put(&mut f.rows[0], 2, w - 1, title, style);
    }
    f
}

/// A menu drawn as tmux draws it: the title centred on the top border,
/// ` name   (key) ` lines, separators joined to the border, the selected
/// line in `selected`, lines that cannot be chosen dimmed.
pub(crate) fn menu_frame(state: &MenuState, style: Style, selected: Style, border: Style) -> PaneFrame {
    let size = state.rect.size();
    let mut f = box_frame(size, state.border, "", border, style);
    let (w, h) = (size.cols as usize, size.rows as usize);
    if w < 3 || h < 3 {
        return f;
    }
    let [.., hz, _, lt, rt] = state.border.glyphs();
    let tw: usize = state.title.chars().map(char_width).sum();
    if tw > 0 && tw + 2 <= w {
        put(&mut f.rows[0], 1 + (w - 2 - tw) / 2, w - 1, &state.title, border);
    }
    for (i, line) in state.lines().iter().enumerate().take(h - 2) {
        let row = &mut f.rows[i + 1];
        match line {
            None => {
                for c in &mut row.cells[1..w - 1] {
                    *c = Cell { ch: hz, style: border, ..Cell::BLANK };
                }
                row.cells[0].ch = lt;
                row.cells[w - 1].ch = rt;
            }
            Some(text) => {
                let enabled = matches!(state.items.get(i), Some(MenuItem::Entry { enabled: true, .. }));
                let mut st = if state.selected == Some(i) { selected } else { style };
                if !enabled {
                    st.attrs.insert(lif_types::Attrs::FAINT);
                }
                for c in &mut row.cells[1..w - 1] {
                    *c = Cell { style: st, ..Cell::BLANK };
                }
                put(row, 1, w - 1, text, st);
            }
        }
    }
    f
}
